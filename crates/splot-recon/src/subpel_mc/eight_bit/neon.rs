// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit unscaled § 7.13.3.18 interpolation of [`super`] in NEON
//! lanes.
//!
//! The arithmetic is that of the AVX2 byte-lane kernels: with halved taps
//! one pass over eight-bit samples sums exactly in `i16`, one `mla` by lane
//! per tap, and the vertical pass of a two-pass block weighs its rounded
//! horizontal rows into `i32`, one `smlal` pair per tap. A group is eight
//! final-pass values: one row of an 8-column strip, or two rows of a
//! 4-column block. The rows of a vertical pass stay in a register ring, and
//! a compound cell keeps its first predictors in a group array for the
//! average, so no pass stages through memory.

#![allow(
    clippy::inline_always,
    reason = "measured strip-kernel row and store helpers"
)]

use super::super::block::{Block, Sink, Taps};
use core::arch::aarch64::*;
use core::slice::{ChunksExact, ChunksExactMut};
use fearless_simd::{Bytes, Neon, Simd, SimdBase, SimdFrom, i16x8, i32x4, u8x16, u16x8, u64x2};

/// Rows `0..n` of a 2-D array in `data` from `at`, `stride` apart and `len`
/// wide: the first `n - 1` as whole strides, then the last, `len` long.
/// `None` when they leave `data`, `n` is zero or `stride < len`.
#[inline(always)]
fn rows<T>(
    data: &[T],
    at: usize,
    stride: usize,
    len: usize,
    n: usize,
) -> Option<(ChunksExact<'_, T>, &[T])> {
    let span = n.checked_sub(1)?.checked_mul(stride)?;
    let rows = data.get(at..)?.get(..span.checked_add(len)?)?;
    let (body, last) = rows.split_at_checked(span)?;
    (stride >= len && len > 0).then(|| (body.chunks_exact(stride), last))
}

/// [`rows`] of mutable `data`.
#[inline(always)]
fn rows_mut<T>(
    data: &mut [T],
    at: usize,
    stride: usize,
    len: usize,
    n: usize,
) -> Option<(ChunksExactMut<'_, T>, &mut [T])> {
    let span = n.checked_sub(1)?.checked_mul(stride)?;
    let rows = data.get_mut(at..)?.get_mut(..span.checked_add(len)?)?;
    let (body, last) = rows.split_at_mut_checked(span)?;
    (stride >= len && len > 0).then(|| (body.chunks_exact_mut(stride), last))
}

/// What a strip kernel stores: `N` outputs of a row at a time, `N` 8 or
/// 16, as `Clip1` bytes ([`Self::bytes`]) or compound predictors
/// ([`Self::preds`]).
pub(in crate::subpel_mc) trait Out {
    /// Whether the outputs are single-reference predictions.
    const PUT: bool;
    /// Whether the outputs average a first predictor ([`Self::first`]).
    const AVERAGE: bool = false;
    /// One output row of a strip.
    type Row<'r>
    where
        Self: 'r;
    /// Output rows of a strip.
    type Rows<'r>: Iterator<Item = Self::Row<'r>>
    where
        Self: 'r;
    /// The COMPOUND_AVERAGE weight of [`Self::first`].
    fn forward(&self) -> i32 {
        8
    }
    /// Rows `0..n` of the strip `len` wide from column `col`, as [`rows`].
    fn rows(&mut self, col: usize, len: usize, n: usize)
    -> Option<(Self::Rows<'_>, Self::Row<'_>)>;
    /// The `N` first predictors of `row`.
    fn first<const N: usize>(t: Neon, _: &Self::Row<'_>) -> [int16x8_t; 2] {
        [i16x8::splat(t, 0).into(); 2]
    }
    /// Stores `N` bytes.
    fn bytes<const N: usize>(_: Neon, _: Self::Row<'_>, _: uint8x16_t) {}
    /// Stores `N` compound predictors, the second eight in `values[1]`.
    fn preds<const N: usize>(_: Neon, _: Self::Row<'_>, _: [int16x8_t; 2]) {}
}

/// `N` bytes into `row`.
#[inline(always)]
fn store_row<const N: usize>(t: Neon, row: &mut [u8], values: uint8x16_t) {
    let values = u8x16::<Neon>::simd_from(t, values);
    if N == 16 {
        if let Some(slot) = row.first_chunk_mut::<16>() {
            values.store_array(slot);
        }
    } else if let Some(slot) = row.first_chunk_mut::<8>() {
        *slot = values.bitcast::<u64x2<Neon>>()[0].to_le_bytes();
    }
}

/// The `Clip1` single-reference prediction into `u8` storage.
pub(in crate::subpel_mc) struct Put<'a> {
    pub(in crate::subpel_mc) output: &'a mut [u8],
    pub(in crate::subpel_mc) stride: usize,
}

impl Out for Put<'_> {
    const PUT: bool = true;
    type Row<'r>
        = &'r mut [u8]
    where
        Self: 'r;
    type Rows<'r>
        = ChunksExactMut<'r, u8>
    where
        Self: 'r;
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        rows_mut(self.output, col, self.stride, len, n)
    }
    #[inline(always)]
    fn bytes<const N: usize>(t: Neon, row: &mut [u8], values: uint8x16_t) {
        store_row::<N>(t, row, values);
    }
}

/// A compound predictor sample: eight-bit predictors fit `i16`, 16-bit
/// ones need `i32`.
pub(in crate::subpel_mc) trait Pred: Copy {
    /// Stores `N` predictors.
    fn store<const N: usize>(t: Neon, row: &mut [Self], values: [int16x8_t; 2]);
    /// Loads `N` predictors, zero past the slice.
    fn load<const N: usize>(t: Neon, row: &[Self]) -> [int16x8_t; 2];
}

impl Pred for i16 {
    #[inline(always)]
    fn store<const N: usize>(t: Neon, row: &mut [i16], values: [int16x8_t; 2]) {
        if let Some(slot) = row.first_chunk_mut::<8>() {
            i16x8::<Neon>::simd_from(t, values[0]).store_array(slot);
        }
        if N == 16
            && let Some(slot) = row.get_mut(8..).and_then(<[i16]>::first_chunk_mut::<8>)
        {
            i16x8::<Neon>::simd_from(t, values[1]).store_array(slot);
        }
    }
    #[inline(always)]
    fn load<const N: usize>(t: Neon, row: &[i16]) -> [int16x8_t; 2] {
        let lanes = |at: usize| {
            row.get(at..)
                .and_then(<[i16]>::first_chunk::<8>)
                .map_or(i16x8::splat(t, 0), |lanes| i16x8::load_array_ref(t, lanes))
                .into()
        };
        [lanes(0), if N == 16 { lanes(8) } else { lanes(0) }]
    }
}

impl Pred for i32 {
    #[inline(always)]
    fn store<const N: usize>(t: Neon, row: &mut [i32], values: [int16x8_t; 2]) {
        let mut store = |at: usize, values: int16x8_t| {
            let (low, high) = t.widen_i16x8(i16x8::simd_from(t, values));
            if let Some(slot) = row.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>)
                && let (first, second) = slot.split_at_mut(4)
                && let (Some(first), Some(second)) =
                    (first.first_chunk_mut::<4>(), second.first_chunk_mut::<4>())
            {
                low.store_array(first);
                high.store_array(second);
            }
        };
        store(0, values[0]);
        if N == 16 {
            store(8, values[1]);
        }
    }
    #[inline(always)]
    fn load<const N: usize>(t: Neon, row: &[i32]) -> [int16x8_t; 2] {
        let lanes = |at: usize| {
            row.get(at..)
                .and_then(<[i32]>::first_chunk::<8>)
                .map_or(i16x8::splat(t, 0), |lanes| {
                    let (first, second) = lanes.split_at(4);
                    match (first.first_chunk::<4>(), second.first_chunk::<4>()) {
                        (Some(first), Some(second)) => t.narrow_i32x4(
                            i32x4::load_array_ref(t, first),
                            i32x4::load_array_ref(t, second),
                        ),
                        _ => i16x8::splat(t, 0),
                    }
                })
                .into()
        };
        [lanes(0), if N == 16 { lanes(8) } else { lanes(0) }]
    }
}

/// The compound predictor into `P` storage.
pub(in crate::subpel_mc) struct Prep<'a, P> {
    pub(in crate::subpel_mc) output: &'a mut [P],
    pub(in crate::subpel_mc) stride: usize,
}

impl<P: Pred> Out for Prep<'_, P> {
    const PUT: bool = false;
    type Row<'r>
        = &'r mut [P]
    where
        Self: 'r;
    type Rows<'r>
        = ChunksExactMut<'r, P>
    where
        Self: 'r;
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        rows_mut(self.output, col, self.stride, len, n)
    }
    #[inline(always)]
    fn preds<const N: usize>(t: Neon, row: &mut [P], values: [int16x8_t; 2]) {
        P::store::<N>(t, row, values);
    }
}

/// The § 7.13.3.15 COMPOUND_AVERAGE of the contiguous `w`-wide first
/// predictor `pred0` and the block's compound predictor, into `u8`.
pub(in crate::subpel_mc) struct Average<'a, P> {
    pub(in crate::subpel_mc) pred0: &'a [P],
    pub(in crate::subpel_mc) w: usize,
    pub(in crate::subpel_mc) forward: i32,
    pub(in crate::subpel_mc) output: &'a mut [u8],
    pub(in crate::subpel_mc) stride: usize,
}

impl<P: Pred> Out for Average<'_, P> {
    const PUT: bool = false;
    const AVERAGE: bool = true;
    type Row<'r>
        = (&'r [P], &'r mut [u8])
    where
        Self: 'r;
    type Rows<'r>
        = core::iter::Zip<ChunksExact<'r, P>, ChunksExactMut<'r, u8>>
    where
        Self: 'r;
    #[inline(always)]
    fn forward(&self) -> i32 {
        self.forward
    }
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        let (first, first_last) = rows(self.pred0, col, self.w, len, n)?;
        let (output, last) = rows_mut(self.output, col, self.stride, len, n)?;
        Some((first.zip(output), (first_last, last)))
    }
    #[inline(always)]
    fn first<const N: usize>(t: Neon, (pred0, _): &(&[P], &mut [u8])) -> [int16x8_t; 2] {
        P::load::<N>(t, pred0)
    }
    #[inline(always)]
    fn bytes<const N: usize>(t: Neon, (_, row): (&[P], &mut [u8]), values: uint8x16_t) {
        store_row::<N>(t, row, values);
    }
}

/// Calls `$f::<H, V, N>` for the strip width `N`, 16 or 8, and the tap-pair
/// classes of both passes, `H` 0, 2 or 4 and `V` 0 to 4.
macro_rules! strip_shapes {
    ($sixteen:expr, $h:expr, $v:expr, $f:ident::<$o:ident> $args:tt) => {
        strip_shapes!(@ $sixteen, $h, $v, $f $o $args,
            [0 0, 0 1, 0 2, 0 3, 0 4, 2 0, 2 1, 2 2, 2 3, 2 4, 4 0, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $sixteen:expr, $h:expr, $v:expr, $f:ident $o:ident $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($sixteen, $h, $v) {
            $((true, $hh, $vv) => $f::<$hh, $vv, 16, $o> $args,)*
            $((false, $hh, $vv) => $f::<$hh, $vv, 8, $o> $args,)*
            _ => {}
        }
    };
}

/// Whether the strip kernels take `block`: a multiple of 8 columns read in
/// place without a column remap, every window row inside its clip rows and
/// the storage.
#[inline]
fn strip_fits(block: &Block<'_, u8>) -> bool {
    let rows = block.h + block.vertical.map_or(0, |taps| taps.span() - 1);
    let last = block.top + rows as isize - 1;
    block.w.is_multiple_of(8)
        && block.remap.is_none()
        && block.stride >= 24
        && block.top >= block.rows[0]
        && last <= block.rows[1]
        && block.start(rows - 1) + super::super::block::reach(block.w) <= block.source.len()
}

/// Whether the kernels cover `block`: strips of 8 columns, or 4 columns in
/// pairs of rows with at most two horizontal tap pairs.
pub(in crate::subpel_mc) fn fits<T>(block: &Block<'_, T>) -> bool {
    (block.w > 0 && block.w.is_multiple_of(8))
        || (block.w == 4
            && block.h.is_multiple_of(2)
            && block.horizontal.is_none_or(|taps| taps.count <= 2))
}

/// Calls `$f::<PUT, W, H, V>` for the run-time group width and the tap-pair
/// counts of both passes, zero for a skipped pass.
macro_rules! specialize {
    ($four:expr, $h:expr, $v:expr, $f:ident::<$put:ident> $args:tt) => {
        specialize!(@ $four, $h, $v, $f $put $args,
            [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4],
            [3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $four:expr, $h:expr, $v:expr, $f:ident $put:ident $args:tt,
        [$($h4:literal $v4:literal),*], [$($h8:literal $v8:literal),*]) => {
        match ($four, $h, $v) {
            $((true, $h4, $v4) => $f::<$put, 4, $h4, $v4> $args,)*
            $((false, $h4, $v4) => $f::<$put, 8, $h4, $v4> $args,)*
            $((false, $h8, $v8) => $f::<$put, 8, $h8, $v8> $args,)*
            _ => {}
        }
    };
}

/// `Some` of [`cell_groups`] of the `W`-square cell of the run-time tap-pair
/// counts of both passes, `None` outside the register cells.
macro_rules! cells {
    ($h:expr, $v:expr, $put:literal $args:tt) => {
        if W == 4 {
            cells!(@ $h, $v, $put $args, [0 0, 0 1, 0 2, 1 0, 1 1, 1 2, 2 0, 2 1, 2 2])
        } else {
            cells!(@ $h, $v, $put $args,
                [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4,
                 3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
        }
    };
    (@ $h:expr, $v:expr, $put:literal $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($h, $v) {
            $(($hh, $vv) => cell_groups::<$put, W, $hh, $vv> $args,)*
            _ => None,
        }
    };
}

/// Expands `$body` with `$row` bound to each listed index below `$rows`, a
/// constant, so a cell's rows unroll.
macro_rules! unroll {
    ($row:ident < $rows:expr, [$($n:literal)*] => $body:block) => {
        $(
            if $n < $rows {
                let $row: usize = $n;
                $body
            }
        )*
    };
}

/// Both references' samples, the output and its stride of one plane of a
/// [`Program`].
pub(in crate::subpel_mc) type ProgramPlane<'a, 'b> = ([&'b [u8]; 2], &'a mut [u8], usize);

/// A program of compound cells of `subpel_mc::cell` for the kernels: each
/// cell whose windows [`super::super::cell::placed`] reads in place is stored
/// into both `planes` (U and V share the geometry) and marked in the result.
pub(in crate::subpel_mc) struct Program<'a, 'b> {
    pub(in crate::subpel_mc) planes: [Option<ProgramPlane<'a, 'b>>; 2],
    pub(in crate::subpel_mc) geometry: super::super::cell::Geometry,
    pub(in crate::subpel_mc) filter: usize,
    pub(in crate::subpel_mc) cells: &'a [super::super::cell::Cell],
    pub(in crate::subpel_mc) forward: i32,
}

/// Runs the cells of `program` that it covers, `size` 4 or 8, and returns
/// their bits.
#[inline]
pub(in crate::subpel_mc) fn program(
    t: Neon,
    program: Program<'_, '_>,
    size: usize,
) -> super::super::cell::Written {
    let mut written = [[0; 4]; 2];
    dot::run(t, Job::Program(program, size, &mut written));
    written
}

/// One call of the kernels.
pub(in crate::subpel_mc) enum Job<'a, 'b, 's> {
    /// [`program`].
    Program(Program<'a, 'b>, usize, &'a mut super::super::cell::Written),
    /// [`predict`].
    Predict(&'a Block<'b, u8>, &'a mut Sink<'s>),
    /// [`compound`].
    Compound([&'a Block<'b, u8>; 2], i32, &'a mut [u8], usize),
    /// [`compound_cell`].
    Cell([&'a Block<'b, u8>; 2], usize, i32, &'a mut [u8], usize),
    /// [`put_cell`].
    PutCell(&'a Block<'b, u8>, &'a mut [u8], usize),
}

/// Predicts `block`, which [`fits`], into an eight-bit `sink`.
#[inline]
pub(in crate::subpel_mc) fn predict(t: Neon, block: &Block<'_, u8>, sink: &mut Sink<'_>) {
    dot::run(t, Job::Predict(block, sink));
}

/// Predicts both blocks of a compound cell of at most 64 samples (or of two
/// full-sample blocks of any size), which [`fits`], and stores their
/// COMPOUND_AVERAGE into `output`.
#[inline]
pub(in crate::subpel_mc) fn compound(
    t: Neon,
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    dot::run(t, Job::Compound(blocks, forward, output, stride));
}

/// [`compound`] of the blocks of a `size`-square cell of `subpel_mc::cell`,
/// whose unremapped windows lie inside their clip rows.
#[inline]
pub(in crate::subpel_mc) fn compound_cell(
    t: Neon,
    blocks: [&Block<'_, u8>; 2],
    size: usize,
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    dot::run(t, Job::Cell(blocks, size, forward, output, stride));
}

/// Stores the `Clip1` single-reference prediction of a 4x4 or 8x8 `block`,
/// which [`fits`].
#[inline]
pub(in crate::subpel_mc) fn put_cell(
    t: Neon,
    block: &Block<'_, u8>,
    output: &mut [u8],
    stride: usize,
) {
    dot::run(t, Job::PutCell(block, output, stride));
}

fearless_simd::kernel!(
    /// Runs `job` in the multiply-accumulate kernels.
    #[inline]
    fn mla_entry(t: Neon, job: Job<'_, '_, '_>) {
        mla::entry(t, job);
    }
);

pub(in crate::subpel_mc) mod dot;
mod kernels;

/// The kernels whose horizontal sums multiply-accumulate halved taps.
mod mla {
    use super::*;
    use super::{
        mla_line as hline, pair_sums as hpair, pair_taps as pair_columns, row_sums_of as hrow,
        weights as taps,
    };

    super::kernels::kernels!("neon");
}

/// The kernels whose horizontal sums are `sdot` dot products.
mod sdot {
    use super::*;
    use super::{
        dot_line as hline, dot_pair_columns as pair_columns, dot_pair_sums as hpair,
        dot_row_sums as hrow, dot_taps as taps,
    };

    super::kernels::kernels!("neon,dotprod");
}

/// The halved taps of a horizontal pass as `i8`, taps `0..4` in word 0 and
/// `4..8` in word 1, zero past its pairs.
#[inline]
#[target_feature(enable = "neon")]
fn dot_taps(_: Neon, taps: Option<&Taps>) -> int8x16_t {
    let word = taps.map_or(0, |taps| {
        taps.bytes
            .iter()
            .rev()
            .fold(0, |word, &pair| word << 16 | u64::from(pair as u16))
    });
    vcombine_s8(vcreate_s8(word), vdup_n_s8(0))
}

/// Byte indices of the four 4-byte windows of outputs `0..4` from byte
/// `4 * k`, as two little-endian words, for `k` 0, 1 and 2.
const DOT_WINDOWS: [[u64; 2]; 3] = {
    let mut out = [[0; 2]; 3];
    let mut k = 0;
    while k < 3 {
        let mut lane = 0;
        while lane < 16 {
            out[k][lane / 8] |= ((4 * k + lane / 4 + lane % 4) as u64) << (8 * (lane % 8));
            lane += 1;
        }
        k += 1;
    }
    out
};

/// The `tbl` indices of [`DOT_WINDOWS`] `K`.
#[inline]
#[target_feature(enable = "neon")]
fn dot_windows<const K: usize>() -> uint8x16_t {
    vcombine_u8(vcreate_u8(DOT_WINDOWS[K][0]), vcreate_u8(DOT_WINDOWS[K][1]))
}

/// The halved-tap sums of eight outputs of `bytes`, as [`row_sums_of`]: the
/// samples biased to `i8` weigh exactly in `sdot`, and the bias times the
/// halved taps' sum, `128 * 64`, starts each sum.
#[inline]
#[target_feature(enable = "neon,dotprod")]
fn dot_row_sums<const P: usize>(bytes: uint8x16_t, taps: int8x16_t) -> int16x8_t {
    let samples = vreinterpretq_s8_u8(veorq_u8(bytes, vdupq_n_u8(0x80)));
    let windows = [
        vqtbl1q_s8(samples, dot_windows::<0>()),
        vqtbl1q_s8(samples, dot_windows::<1>()),
        vqtbl1q_s8(samples, dot_windows::<2>()),
    ];
    let bias = vdupq_n_s32(128 * 64);
    let mut low = vdotq_laneq_s32::<0>(bias, windows[0], taps);
    let mut high = vdotq_laneq_s32::<0>(bias, windows[1], taps);
    if P > 2 {
        low = vdotq_laneq_s32::<1>(low, windows[1], taps);
        high = vdotq_laneq_s32::<1>(high, windows[2], taps);
    }
    vuzp1q_s16(vreinterpretq_s16_s32(low), vreinterpretq_s16_s32(high))
}

/// The halved-tap sums plus `bias` of `N` outputs, 8 or 16, of a window row
/// whose columns `0..16` are `a` and `8..24` are `b`, as [`dot_row_sums`].
#[inline]
#[target_feature(enable = "neon,dotprod")]
fn dot_line<const P: usize, const N: usize>(
    a: uint8x16_t,
    b: uint8x16_t,
    taps: int8x16_t,
    bias: i32,
) -> [int16x8_t; 2] {
    let flip = vdupq_n_u8(0x80);
    let (a, b) = (
        vreinterpretq_s8_u8(veorq_u8(a, flip)),
        vreinterpretq_s8_u8(veorq_u8(b, flip)),
    );
    let bias = vdupq_n_s32(128 * 64 + bias);
    let windows = [dot_windows::<0>(), dot_windows::<1>(), dot_windows::<2>()];
    let a = windows.map(|window| vqtbl1q_s8(a, window));
    let sum = |first: int8x16_t, second: int8x16_t| {
        let sum = vdotq_laneq_s32::<0>(bias, first, taps);
        if P > 2 {
            vdotq_laneq_s32::<1>(sum, second, taps)
        } else {
            sum
        }
    };
    let pack = |low, high| vuzp1q_s16(vreinterpretq_s16_s32(low), vreinterpretq_s16_s32(high));
    let low = pack(sum(a[0], a[1]), sum(a[1], a[2]));
    if N == 8 {
        return [low, low];
    }
    let (b1, b2) = (vqtbl1q_s8(b, windows[1]), vqtbl1q_s8(b, windows[2]));
    [low, pack(sum(a[2], b1), sum(b1, b2))]
}

/// [`dot_line`] in multiply-accumulate lanes.
#[inline]
#[target_feature(enable = "neon")]
fn mla_line<const P: usize, const N: usize>(
    a: uint8x16_t,
    b: uint8x16_t,
    weights: int16x8_t,
    bias: i32,
) -> [int16x8_t; 2] {
    let bias = vdupq_n_s16(bias as i16);
    let low = vaddq_s16(row_sums_of::<P>(a, weights), bias);
    if N == 8 {
        return [low, low];
    }
    [low, vaddq_s16(row_sums_of::<P>(b, weights), bias)]
}

/// The `tbl` indices of the 4-byte windows of a row pair's four outputs, the
/// first row's in bytes `0..8` and the second's in bytes `8..16`, read
/// through the column remap of a 4-column block.
#[inline]
#[target_feature(enable = "neon")]
fn dot_pair_columns(t: Neon, remap: Option<[i8; 2]>) -> [uint8x16_t; 2] {
    let windows = [dot_windows::<0>(), dot_windows::<2>()];
    let Some(remap) = remap_columns(t, remap) else {
        return windows;
    };
    let (rows, cols) = (vdupq_n_u8(8), vdupq_n_u8(7));
    windows.map(|windows| {
        let column = vqtbl1q_u8(remap, vandq_u8(windows, cols));
        vorrq_u8(vandq_u8(windows, rows), column)
    })
}

/// The halved-tap sums of four outputs of each row from `starts`, the first
/// row in lanes `0..4`, as [`pair_sums`] of at most two tap pairs.
#[inline]
#[target_feature(enable = "neon,dotprod")]
fn dot_pair_sums<const P: usize>(
    source: &[u8],
    starts: [usize; 2],
    taps: int8x16_t,
    columns: &[uint8x16_t; 2],
) -> int16x8_t {
    let bytes = vcombine_u8(load8(source, starts[0]), load8(source, starts[1]));
    let samples = vreinterpretq_s8_u8(veorq_u8(bytes, vdupq_n_u8(0x80)));
    let bias = vdupq_n_s32(128 * 64);
    let low = vdotq_laneq_s32::<0>(bias, vqtbl1q_s8(samples, columns[0]), taps);
    let high = vdotq_laneq_s32::<0>(bias, vqtbl1q_s8(samples, columns[1]), taps);
    vuzp1q_s16(vreinterpretq_s16_s32(low), vreinterpretq_s16_s32(high))
}

/// Whether `block` is a register cell: 4x4 with at most two tap pairs per
/// pass or 8x8, read in place without a column remap, its window rows inside
/// their clip rows.
#[inline]
fn is_cell(block: &Block<'_, u8>) -> bool {
    let (w, rows) = (
        block.w,
        block.w + block.vertical.map_or(0, |taps| taps.span() - 1),
    );
    let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
    w == block.h
        && (w == 8 || (w == 4 && pairs(block.horizontal) <= 2 && pairs(block.vertical) <= 2))
        && block.remap.is_none()
        && block.top >= block.rows[0]
        && block.top + rows as isize - 1 <= block.rows[1]
}

/// The output row of group `group` of a `W`-square cell.
const fn group_row<const W: usize>(group: usize) -> usize {
    if W == 4 { 2 * group } else { group }
}

/// Stores the `Clip1` groups of a `W`-square cell.
#[inline]
#[target_feature(enable = "neon")]
fn store_put<const W: usize>(groups: [int16x8_t; W], output: &mut [u8], stride: usize) {
    for (group, values) in groups.into_iter().enumerate().take(W * W / 8) {
        store_bytes(
            output,
            W,
            group_row::<W>(group) * stride,
            stride,
            vqmovun_s16(values),
        );
    }
}

/// Stores the COMPOUND_AVERAGE of the groups of two `W`-square cells.
#[inline]
#[target_feature(enable = "neon")]
fn store_cell<const W: usize>(
    first: [int16x8_t; W],
    second: [int16x8_t; W],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let groups = first.into_iter().zip(second).enumerate().take(W * W / 8);
    if forward == 8 {
        for (group, (first, second)) in groups {
            let values = vqrshrun_n_s16::<5>(vaddq_s16(first, second));
            store_bytes(output, W, group_row::<W>(group) * stride, stride, values);
        }
        return;
    }
    for (group, (first, second)) in groups {
        let values = vqmovun_s16(blend(first, second, forward));
        store_bytes(output, W, group_row::<W>(group) * stride, stride, values);
    }
}

/// Four bytes from `at` as a little-endian word, zero past the slice.
#[inline]
fn load_u32(samples: &[u8], at: usize) -> u32 {
    samples
        .get(at..)
        .and_then(<[u8]>::first_chunk::<4>)
        .map_or(0, |bytes| u32::from_le_bytes(*bytes))
}

/// [`compound`] of two full-sample blocks: the COMPOUND_AVERAGE of the
/// `<< 4` samples is `(a * f + b * (16 - f) + 8) >> 4`, and for `f == 8` the
/// rounded halving add, 16 samples per vector.
#[inline(never)]
#[target_feature(enable = "neon")]
fn fullpel(t: Neon, blocks: [&Block<'_, u8>; 2], forward: i32, output: &mut [u8], stride: usize) {
    let (front, back) = (vdupq_n_u8(forward as u8), vdupq_n_u8((16 - forward) as u8));
    if forward == 8 {
        fullpel_rows(t, blocks, output, stride, |p0, p1| vrhaddq_u8(p0, p1));
    } else if (0..=16).contains(&forward) {
        fullpel_rows(t, blocks, output, stride, |p0, p1| {
            let low = vmull_u8(vget_low_u8(p0), vget_low_u8(front));
            let low = vmlal_u8(low, vget_low_u8(p1), vget_low_u8(back));
            let high = vmlal_high_u8(vmull_high_u8(p0, front), p1, back);
            vrshrn_high_n_u16::<4>(vrshrn_n_u16::<4>(low), high)
        });
    } else {
        fullpel_rows(t, blocks, output, stride, |p0, p1| {
            let weigh = |p0: uint8x8_t, p1: uint8x8_t| {
                let sum = vmulq_n_s16(widen(p0), forward as i16);
                let sum = vmlaq_n_s16(sum, widen(p1), (16 - forward) as i16);
                vqmovun_s16(vshrq_n_s16::<4>(vaddq_s16(sum, vdupq_n_s16(8))))
            };
            vcombine_u8(
                weigh(vget_low_u8(p0), vget_low_u8(p1)),
                weigh(vget_high_u8(p0), vget_high_u8(p1)),
            )
        });
    }
}

/// The rows of [`fullpel`] averaged by `average`: a row at least 16 wide
/// runs as whole 16-sample chunks of its three rows, which a remap never
/// covers, its last chunk overlapping the one before it.
#[inline]
#[target_feature(enable = "neon")]
fn fullpel_rows(
    t: Neon,
    blocks: [&Block<'_, u8>; 2],
    output: &mut [u8],
    stride: usize,
    average: impl Fn(uint8x16_t, uint8x16_t) -> uint8x16_t,
) {
    let [first, second] = blocks;
    let (w, h) = (first.w, first.h);
    let starts = [starts(first, h), starts(second, h)];
    if w >= 16 {
        let load = |row: &[u8; 16]| -> uint8x16_t { u8x16::load_array_ref(t, row).into() };
        for row in 0..h {
            let (Some(row0), Some(row1), Some(out)) = (
                first
                    .source
                    .get(starts[0](row)..)
                    .and_then(|row| row.get(..w)),
                second
                    .source
                    .get(starts[1](row)..)
                    .and_then(|row| row.get(..w)),
                output
                    .get_mut(row * stride..)
                    .and_then(|row| row.get_mut(..w)),
            ) else {
                return;
            };
            let chunks = row0
                .as_chunks::<16>()
                .0
                .iter()
                .zip(row1.as_chunks::<16>().0);
            for (out, (p0, p1)) in out.as_chunks_mut::<16>().0.iter_mut().zip(chunks) {
                u8x16::<Neon>::simd_from(t, average(load(p0), load(p1))).store_array(out);
            }
            if w % 16 != 0
                && let (Some(p0), Some(p1), Some(out)) =
                    (row0.last_chunk(), row1.last_chunk(), out.last_chunk_mut())
            {
                u8x16::<Neon>::simd_from(t, average(load(p0), load(p1))).store_array(out);
            }
        }
        return;
    }
    let remaps = blocks.map(|block| columns(t, block));
    for row in 0..h {
        let p0 = window(t, first.source, starts[0](row), remaps[0]);
        let p1 = window(t, second.source, starts[1](row), remaps[1]);
        let bytes = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(average(p0, p1))).to_le_bytes();
        let Some(slot) = output.get_mut(row * stride..) else {
            return;
        };
        if w == 8 {
            if let Some(slot) = slot.first_chunk_mut::<8>() {
                *slot = bytes;
            }
        } else if let (Some(slot), Some(bytes)) =
            (slot.first_chunk_mut::<4>(), bytes.first_chunk::<4>())
        {
            *slot = *bytes;
        }
    }
}

/// Sixteen bytes from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load16(t: Neon, samples: &[u8], at: usize) -> uint8x16_t {
    match samples.get(at..).and_then(<[u8]>::first_chunk::<16>) {
        Some(bytes) => u8x16::load_array_ref(t, bytes).into(),
        None => vdupq_n_u8(0),
    }
}

/// Eight bytes from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load8(samples: &[u8], at: usize) -> uint8x8_t {
    let word = samples
        .get(at..)
        .and_then(<[u8]>::first_chunk::<8>)
        .map_or(0, |bytes| u64::from_le_bytes(*bytes));
    vcreate_u8(word)
}

/// Four `i16` values from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load_i16(values: &[i16], at: usize) -> int16x4_t {
    let word = values
        .get(at..)
        .and_then(<[i16]>::first_chunk::<4>)
        .map_or(0, |lanes| {
            lanes
                .iter()
                .rev()
                .fold(0, |word, &lane| word << 16 | u64::from(lane as u16))
        });
    vcreate_s16(word)
}

/// Four `i16` values into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_i16(_: Neon, output: &mut [i16], at: usize, values: int16x4_t) {
    if let Some(slot) = output.get_mut(at..).and_then(<[i16]>::first_chunk_mut::<4>) {
        let word = vget_lane_u64::<0>(vreinterpret_u64_s16(values));
        *slot = core::array::from_fn(|lane| (word >> (16 * lane)) as i16);
    }
}

/// Four `i32` values from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load_i32(t: Neon, values: &[i32], at: usize) -> int32x4_t {
    match values.get(at..).and_then(<[i32]>::first_chunk::<4>) {
        Some(lanes) => i32x4::load_array_ref(t, lanes).into(),
        None => vdupq_n_s32(0),
    }
}

/// Widens eight bytes to `i16` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn widen(bytes: uint8x8_t) -> int16x8_t {
    vreinterpretq_s16_u16(vmovl_u8(bytes))
}

/// The halved taps of a pass in `i16` lanes, zero past its pairs.
#[inline]
#[target_feature(enable = "neon")]
fn weights(t: Neon, taps: Option<&Taps>) -> int16x8_t {
    taps.map_or(vdupq_n_s16(0), |taps| {
        vreinterpretq_s16_s32(i32x4::load_array_ref(t, &taps.words).into())
    })
}

/// The `tbl` indices of a block's column remap.
#[inline]
#[target_feature(enable = "neon")]
fn columns(t: Neon, block: &Block<'_, u8>) -> Option<uint8x16_t> {
    remap_columns(t, block.remap)
}

/// The `tbl` indices of a column remap.
#[inline]
#[target_feature(enable = "neon")]
fn remap_columns(t: Neon, remap: Option<[i8; 2]>) -> Option<uint8x16_t> {
    remap.map(|[offset, limit]| {
        let iota = core::array::from_fn::<u8, 16, _>(|lane| lane as u8);
        let lanes = vaddq_s8(vreinterpretq_s8_u8(load16(t, &iota, 0)), vdupq_n_s8(offset));
        vreinterpretq_u8_s8(vminq_s8(vmaxq_s8(lanes, vdupq_n_s8(0)), vdupq_n_s8(limit)))
    })
}

/// The source index of each of the first `rows` window rows of `block`:
/// stepped by the stride while those rows are inside the clip bounds.
#[inline]
fn starts<'a>(block: &'a Block<'_, u8>, rows: usize) -> impl Fn(usize) -> usize + 'a {
    let inside = block.top >= block.rows[0] && block.top + rows as isize - 1 <= block.rows[1];
    let base = block.start(0);
    move |row| {
        if inside {
            base + row * block.stride
        } else {
            block.start(row)
        }
    }
}

/// Window columns `col..col + 16` of the window row from `start`.
#[inline]
#[target_feature(enable = "neon")]
fn window(t: Neon, source: &[u8], start: usize, remap: Option<uint8x16_t>) -> uint8x16_t {
    let bytes = load16(t, source, start);
    remap.map_or(bytes, |remap| vqtbl1q_u8(bytes, remap))
}

/// Byte indices of window columns `k..k + 4` (lanes `0..4`) and `k + 1..k +
/// 5` (lanes `8..12`) of a row in bytes `0..8`, and the same of a row in
/// bytes `8..16` in lanes `4..8` and `12..16`, for `k` 0 and 2.
const PAIR_TAPS: [[u8; 16]; 2] = {
    let mut out = [[0u8; 16]; 2];
    let mut pair = 0;
    while pair < 2 {
        let mut lane = 0;
        while lane < 16 {
            out[pair][lane] = (2 * pair + lane / 8 + lane % 4 + lane / 4 % 2 * 8) as u8;
            lane += 1;
        }
        pair += 1;
    }
    out
};

/// [`PAIR_TAPS`] read through the column remap of a 4-column block.
#[inline]
#[target_feature(enable = "neon")]
fn pair_taps(t: Neon, remap: Option<[i8; 2]>) -> [uint8x16_t; 2] {
    let taps = PAIR_TAPS.map(|taps| load16(t, &taps, 0));
    let Some(remap) = remap_columns(t, remap) else {
        return taps;
    };
    let (rows, cols) = (vdupq_n_u8(8), vdupq_n_u8(7));
    taps.map(|taps| {
        let column = vqtbl1q_u8(remap, vandq_u8(taps, cols));
        vorrq_u8(vandq_u8(taps, rows), column)
    })
}

#[inline]
#[target_feature(enable = "neon")]
fn mla<const L: i32>(sum: int16x8_t, x: int16x8_t, weights: int16x8_t) -> int16x8_t {
    vmlaq_laneq_s16::<L>(sum, x, weights)
}

/// `Σ` over `2 * P` taps of `tap(k) * weights[k]` in `i16` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums16<const P: usize>(
    weights: int16x8_t,
    mut tap: impl FnMut(usize) -> int16x8_t,
) -> int16x8_t {
    let mut sum = vmulq_laneq_s16::<0>(tap(0), weights);
    sum = mla::<1>(sum, tap(1), weights);
    if P >= 2 {
        sum = mla::<2>(sum, tap(2), weights);
        sum = mla::<3>(sum, tap(3), weights);
    }
    if P >= 3 {
        sum = mla::<4>(sum, tap(4), weights);
        sum = mla::<5>(sum, tap(5), weights);
    }
    if P >= 4 {
        sum = mla::<6>(sum, tap(6), weights);
        sum = mla::<7>(sum, tap(7), weights);
    }
    sum
}

/// The `i32` sums of a group: outputs `0..4` and `4..8`.
type Sums = (int32x4_t, int32x4_t);

#[inline]
#[target_feature(enable = "neon")]
fn mac<const L: i32>((low, high): Sums, x: int16x8_t, weights: int16x8_t) -> Sums {
    (
        vmlal_laneq_s16::<L>(low, vget_low_s16(x), weights),
        vmlal_high_laneq_s16::<L>(high, x, weights),
    )
}

/// `Σ` over `2 * P` taps of `tap(k) * weights[k]` in `i32` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums32<const P: usize>(weights: int16x8_t, mut tap: impl FnMut(usize) -> int16x8_t) -> Sums {
    let x = tap(0);
    let mut sums = (
        vmull_laneq_s16::<0>(vget_low_s16(x), weights),
        vmull_high_laneq_s16::<0>(x, weights),
    );
    sums = mac::<1>(sums, tap(1), weights);
    if P >= 2 {
        sums = mac::<2>(sums, tap(2), weights);
        sums = mac::<3>(sums, tap(3), weights);
    }
    if P >= 3 {
        sums = mac::<4>(sums, tap(4), weights);
        sums = mac::<5>(sums, tap(5), weights);
    }
    if P >= 4 {
        sums = mac::<6>(sums, tap(6), weights);
        sums = mac::<7>(sums, tap(7), weights);
    }
    sums
}

/// The halved-tap horizontal sums of eight outputs of the window bytes
/// `bytes`: zipped with zero bytes, its halves stay `i16` vectors that the
/// taps slide, one `ext` each.
#[inline]
#[target_feature(enable = "neon")]
fn row_sums_of<const P: usize>(bytes: uint8x16_t, weights: int16x8_t) -> int16x8_t {
    let zero = vdupq_n_u8(0);
    let (a, b) = (
        vreinterpretq_s16_u8(vzip1q_u8(bytes, zero)),
        vreinterpretq_s16_u8(vzip2q_u8(bytes, zero)),
    );
    sums16::<P>(weights, |k| match k {
        0 => a,
        1 => vextq_s16::<1>(a, b),
        2 => vextq_s16::<2>(a, b),
        3 => vextq_s16::<3>(a, b),
        4 => vextq_s16::<4>(a, b),
        5 => vextq_s16::<5>(a, b),
        6 => vextq_s16::<6>(a, b),
        _ => vextq_s16::<7>(a, b),
    })
}

/// The halved-tap horizontal sums of four outputs of the window rows from
/// `starts`, the first row in lanes `0..4`.
#[inline]
#[target_feature(enable = "neon")]
fn pair_sums<const P: usize>(
    source: &[u8],
    starts: [usize; 2],
    weights: int16x8_t,
    columns: &[uint8x16_t; 2],
) -> int16x8_t {
    let bytes = vcombine_u8(load8(source, starts[0]), load8(source, starts[1]));
    let zero = vdupq_n_u8(0);
    let pair = |pair: usize| {
        let lanes = vqtbl1q_u8(bytes, columns[pair]);
        (
            vreinterpretq_s16_u8(vzip1q_u8(lanes, zero)),
            vreinterpretq_s16_u8(vzip2q_u8(lanes, zero)),
        )
    };
    let first = pair(0);
    let second = if P > 1 { pair(1) } else { first };
    sums16::<P>(weights, |k| match k {
        0 => first.0,
        1 => first.1,
        2 => second.0,
        _ => second.1,
    })
}

/// A block without passes: its samples, `<< 4` for a compound predictor.
#[inline(never)]
#[target_feature(enable = "neon")]
fn copy<const PUT: bool, const W: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
) {
    let rows = block.h;
    let remap = columns(t, block);
    let scale = |x: int16x8_t| if PUT { x } else { vshlq_n_s16::<4>(x) };
    let start = starts(block, rows);
    if W == 4 {
        for row in (0..rows).step_by(2) {
            let rows = [row, row + 1].map(|row| window(t, block.source, start(row), remap));
            let lanes = vreinterpretq_u8_u32(vzip1q_u32(
                vreinterpretq_u32_u8(rows[0]),
                vreinterpretq_u32_u8(rows[1]),
            ));
            out(W == 4, row / 2, row, 0, scale(widen(vget_low_u8(lanes))));
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        for row in 0..rows {
            let bytes = window(t, block.source, start(row) + col, remap);
            out(
                W == 4,
                col / 8 * rows + row,
                row,
                col,
                scale(widen(vget_low_u8(bytes))),
            );
        }
    }
}

/// A block with only a vertical pass of `V` tap pairs over its samples; the
/// last row pair of a 4-column block reads a row past the window that no
/// tap weighs.
#[inline(never)]
#[target_feature(enable = "neon")]
fn vertical<const PUT: bool, const W: usize, const V: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
) {
    let rows = block.h;
    let weights = weights(t, block.vertical);
    let remap = columns(t, block);
    let finish = |sums: int16x8_t| {
        if PUT {
            vrshrq_n_s16::<6>(sums)
        } else {
            vrshrq_n_s16::<2>(sums)
        }
    };
    let start = starts(block, rows + 2 * V - 1);
    if W == 4 {
        let line = |row: usize| {
            let rows = [row, row + 1].map(|row| window(t, block.source, start(row), remap));
            let lanes = vzip1q_u32(vreinterpretq_u32_u8(rows[0]), vreinterpretq_u32_u8(rows[1]));
            widen(vget_low_u8(vreinterpretq_u8_u32(lanes)))
        };
        let mut pairs: [int16x8_t; 4] =
            core::array::from_fn(|k| if k < V { line(2 * k) } else { vdupq_n_s16(0) });
        for row in (0..rows).step_by(2) {
            let next = line(row + 2 * V);
            let pair = |k: usize| if k < V { pairs[k] } else { next };
            let sums = sums16::<V>(weights, |k| {
                if k % 2 == 0 {
                    pair(k / 2)
                } else {
                    vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                }
            });
            out(W == 4, row / 2, row, 0, finish(sums));
            pairs = core::array::from_fn(|k| pair(k + 1));
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        let line = |row: usize| {
            widen(vget_low_u8(window(
                t,
                block.source,
                start(row) + col,
                remap,
            )))
        };
        let mut ring: [int16x8_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * V {
                line(k)
            } else {
                vdupq_n_s16(0)
            }
        });
        for row in 0..rows {
            let next = line(row + 2 * V - 1);
            let tap = |k: usize| if k + 1 < 2 * V { ring[k] } else { next };
            out(
                W == 4,
                col / 8 * rows + row,
                row,
                col,
                finish(sums16::<V>(weights, tap)),
            );
            ring = core::array::from_fn(|k| tap(k + 1));
        }
    }
}

/// The COMPOUND_AVERAGE of two groups of compound predictors, before
/// `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn blend(first: int16x8_t, second: int16x8_t, forward: i32) -> int16x8_t {
    if forward == 8 {
        return vrshrq_n_s16::<5>(vaddq_s16(first, second));
    }
    let (f, b) = (forward as i16, (16 - forward) as i16);
    let low = vmlal_n_s16(vmull_n_s16(vget_low_s16(first), f), vget_low_s16(second), b);
    let high = vmlal_high_n_s16(vmull_high_n_s16(first, f), second, b);
    vrshrn_high_n_s32::<8>(vrshrn_n_s32::<8>(low), high)
}

/// Four `i32` values into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_i32(t: Neon, output: &mut [i32], at: usize, values: int32x4_t) {
    if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>) {
        i32x4::<Neon>::simd_from(t, values).store_array(slot);
    }
}

/// The eight bytes of a group into `output` from `at`: one row of a
/// `w`-wide strip, or two rows `stride` apart of a 4-wide block.
#[inline]
#[target_feature(enable = "neon")]
fn store_bytes(output: &mut [u8], w: usize, at: usize, stride: usize, values: uint8x8_t) {
    let bytes = vget_lane_u64::<0>(vreinterpret_u64_u8(values)).to_le_bytes();
    if w != 4 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
            *slot = bytes;
        }
        return;
    }
    for (at, lanes) in [at, at + stride].into_iter().zip(bytes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}

/// The eight words of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_words(
    t: Neon,
    output: &mut [u16],
    w: usize,
    at: usize,
    stride: usize,
    values: uint16x8_t,
) {
    let values = u16x8::<Neon>::simd_from(t, values);
    if w != 4 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
            values.store_array(slot);
        }
        return;
    }
    let lanes = values.to_array();
    for (at, lanes) in [at, at + stride].into_iter().zip(lanes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}
