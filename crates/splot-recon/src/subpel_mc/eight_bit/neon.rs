// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit unscaled § 7.13.3.18 interpolation of [`super`] in NEON
//! lanes.
//!
//! With halved taps one pass over eight-bit samples sums exactly in `i16`,
//! and the vertical pass of a two-pass block weighs its rounded horizontal
//! rows into `i32`. A strip kernel runs 8 or 16 columns of a row per vector
//! and a 4-column kernel two rows; the rows of a vertical pass stay in a
//! register ring, so no pass stages through memory. A compound cell keeps
//! both predictors of a 4x4 or 8x8 cell in registers for the average. Each
//! entry takes the blocks [`super`] sends it.

#![allow(
    clippy::inline_always,
    reason = "measured strip-kernel row and store helpers"
)]

use super::super::block::{Block, Geometry, Sink, Taps, Window, is_cell};
use super::super::cell::{Cell, Plane, Written};
use super::super::{CellPlacement, ReferencePlaneView};
use super::pairs;
use core::arch::aarch64::*;
use core::slice::{ChunksExact, ChunksExactMut};
use fearless_simd::{Bytes, Neon, Simd, SimdBase, SimdFrom, i8x16, i16x8, i32x4, u8x16, u64x2};

/// Where a register cell reads window row `r`: `base + r * stride`, or
/// when clamped row `top + r` clamped to `first..=last` from column `base`,
/// window column `c` from column `clamp(start + c, low, high)` of the load.
#[derive(Clone, Copy, Default)]
pub(in crate::subpel_mc) struct Reads {
    pub(in crate::subpel_mc) base: usize,
    pub(in crate::subpel_mc) stride: usize,
    pub(in crate::subpel_mc) top: i32,
    pub(in crate::subpel_mc) first: i32,
    pub(in crate::subpel_mc) last: i32,
    pub(in crate::subpel_mc) columns: [i16; 3],
}

/// The [`Reads`] of `window` in a plane of `geometry` within `bounds`, and
/// whether they are clamped: in place when every read is inside the clip
/// bounds, else the § 7.13.3.18 `Clip3` reads; `None` when the clip range
/// is empty or a clamped row cannot load 16 samples.
#[inline(always)]
pub(in crate::subpel_mc) fn reads(
    window: &Window,
    geometry: Geometry,
    bounds: [i32; 4],
) -> Option<(Reads, bool)> {
    let [stride, width, rows, len] = geometry;
    if let Some(base) = window.origin(geometry, bounds) {
        let reads = Reads {
            base,
            stride,
            ..Reads::default()
        };
        return Some((reads, false));
    }
    let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
    let (first_x, last_x) = (bound(bounds[0], width), bound(bounds[2], width));
    let (first, last) = (bound(bounds[1], rows), bound(bounds[3], rows));
    if first_x > last_x || first > last || last_x > i32::from(i16::MAX) - 32 {
        return None;
    }
    let column = window.column.max(first_x).min(last_x);
    let end = (last as usize)
        .checked_mul(stride)?
        .checked_add(column as usize + 16)?;
    if end > len {
        return None;
    }
    let start = window.column.max(first_x - 16).min(last_x + 16) - column;
    let reads = Reads {
        base: column as usize,
        stride,
        top: window.row,
        first,
        last,
        columns: [start, first_x - column, last_x - column].map(|value| value as i16),
    };
    Some((reads, true))
}

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

/// What a kernel stores: `N` outputs of a row at a time, `N` 8 or 16, or
/// four of each of two rows, as `Clip1` samples ([`Self::bytes`]) or
/// compound predictors ([`Self::preds`]).
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
    /// The four first predictors of each of rows `a` and `b`.
    fn first_pair(t: Neon, _: &Self::Row<'_>, _: &Self::Row<'_>) -> int16x8_t {
        i16x8::splat(t, 0).into()
    }
    /// Stores `N` samples.
    fn bytes<const N: usize>(_: Neon, _: Self::Row<'_>, _: uint8x16_t) {}
    /// Stores four samples into each of rows `a` (lanes `0..4`) and `b`
    /// (lanes `4..8`).
    fn bytes_pair(_: Neon, _: Self::Row<'_>, _: Self::Row<'_>, _: uint8x16_t) {}
    /// Stores `N` compound predictors, the second eight in `values[1]`.
    fn preds<const N: usize>(_: Neon, _: Self::Row<'_>, _: [int16x8_t; 2]) {}
    /// Stores four compound predictors into each of rows `a` and `b`.
    fn preds_pair(_: Neon, _: Self::Row<'_>, _: Self::Row<'_>, _: int16x8_t) {}
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

/// Lanes `0..4` of `values` into row `a`, lanes `4..8` into row `b`.
#[inline(always)]
fn store_pair<T: Copy, const N: usize>(a: &mut [T], b: &mut [T], lanes: [T; N]) {
    for (row, lanes) in [a, b].into_iter().zip(lanes.as_chunks::<4>().0) {
        if let Some(slot) = row.first_chunk_mut::<4>() {
            *slot = *lanes;
        }
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
    #[inline(always)]
    fn bytes_pair(t: Neon, a: &mut [u8], b: &mut [u8], values: uint8x16_t) {
        store_pair(a, b, u8x16::<Neon>::simd_from(t, values).to_array());
    }
}

/// The `Clip1` single-reference prediction into `u16` storage.
pub(in crate::subpel_mc) struct PutWide<'a> {
    pub(in crate::subpel_mc) output: &'a mut [u16],
    pub(in crate::subpel_mc) stride: usize,
}

impl Out for PutWide<'_> {
    const PUT: bool = true;
    type Row<'r>
        = &'r mut [u16]
    where
        Self: 'r;
    type Rows<'r>
        = ChunksExactMut<'r, u16>
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
    fn bytes<const N: usize>(t: Neon, row: &mut [u16], values: uint8x16_t) {
        let (low, high) = t.widen_u8x16(u8x16::simd_from(t, values));
        for (at, lanes) in [low, high].into_iter().enumerate().take(N / 8) {
            if let Some(slot) = row
                .get_mut(8 * at..)
                .and_then(<[u16]>::first_chunk_mut::<8>)
            {
                lanes.store_array(slot);
            }
        }
    }
    #[inline(always)]
    fn bytes_pair(t: Neon, a: &mut [u16], b: &mut [u16], values: uint8x16_t) {
        store_pair(
            a,
            b,
            t.widen_u8x16(u8x16::simd_from(t, values)).0.to_array(),
        );
    }
}

/// A compound predictor sample: eight-bit predictors fit `i16`, 16-bit
/// ones need `i32`.
pub(in crate::subpel_mc) trait Pred: Copy {
    /// Stores `N` predictors, 8 or 16.
    fn store<const N: usize>(t: Neon, row: &mut [Self], values: [int16x8_t; 2]);
    /// Loads `N` predictors, 8 or 16, zero past the slice.
    fn load<const N: usize>(t: Neon, row: &[Self]) -> [int16x8_t; 2];
    /// Stores lanes `0..4` into row `a` and `4..8` into row `b`.
    fn store_pair(t: Neon, a: &mut [Self], b: &mut [Self], values: int16x8_t);
    /// Loads four predictors of each of rows `a` and `b`, zero past them.
    fn load_pair(t: Neon, a: &[Self], b: &[Self]) -> int16x8_t;
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
    #[inline(always)]
    fn store_pair(t: Neon, a: &mut [i16], b: &mut [i16], values: int16x8_t) {
        store_pair(a, b, i16x8::<Neon>::simd_from(t, values).to_array());
    }
    #[inline(always)]
    fn load_pair(t: Neon, a: &[i16], b: &[i16]) -> int16x8_t {
        let (a, b) = (a.first_chunk::<4>(), b.first_chunk::<4>());
        let lanes: [i16; 8] = core::array::from_fn(|lane| {
            let row = if lane < 4 { a } else { b };
            row.map_or(0, |row| row[lane % 4])
        });
        i16x8::load_array(t, lanes).into()
    }
}

impl Pred for i32 {
    #[inline(always)]
    fn store<const N: usize>(t: Neon, row: &mut [i32], values: [int16x8_t; 2]) {
        for (at, values) in values.into_iter().enumerate().take(N / 8) {
            let (low, high) = t.widen_i16x8(i16x8::simd_from(t, values));
            for (half, lanes) in [low, high].into_iter().enumerate() {
                if let Some(slot) = row
                    .get_mut(8 * at + 4 * half..)
                    .and_then(<[i32]>::first_chunk_mut::<4>)
                {
                    lanes.store_array(slot);
                }
            }
        }
    }
    #[inline(always)]
    fn load<const N: usize>(t: Neon, row: &[i32]) -> [int16x8_t; 2] {
        let lanes = |at: usize| {
            t.narrow_i32x4(load4(t, row, at), load4(t, row, at + 4))
                .into()
        };
        [lanes(0), if N == 16 { lanes(8) } else { lanes(0) }]
    }
    #[inline(always)]
    fn store_pair(t: Neon, a: &mut [i32], b: &mut [i32], values: int16x8_t) {
        let (low, high) = t.widen_i16x8(i16x8::simd_from(t, values));
        for (row, lanes) in [a, b].into_iter().zip([low, high]) {
            if let Some(slot) = row.first_chunk_mut::<4>() {
                lanes.store_array(slot);
            }
        }
    }
    #[inline(always)]
    fn load_pair(t: Neon, a: &[i32], b: &[i32]) -> int16x8_t {
        t.narrow_i32x4(load4(t, a, 0), load4(t, b, 0)).into()
    }
}

/// Four `i32` values from `at`, zero past the slice.
#[inline(always)]
fn load4(t: Neon, values: &[i32], at: usize) -> i32x4<Neon> {
    values
        .get(at..)
        .and_then(<[i32]>::first_chunk::<4>)
        .map_or(i32x4::splat(t, 0), |lanes| i32x4::load_array_ref(t, lanes))
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
    #[inline(always)]
    fn preds_pair(t: Neon, a: &mut [P], b: &mut [P], values: int16x8_t) {
        P::store_pair(t, a, b, values);
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
    fn first_pair(t: Neon, (a, _): &(&[P], &mut [u8]), (b, _): &(&[P], &mut [u8])) -> int16x8_t {
        P::load_pair(t, a, b)
    }
    #[inline(always)]
    fn bytes<const N: usize>(t: Neon, (_, row): (&[P], &mut [u8]), values: uint8x16_t) {
        store_row::<N>(t, row, values);
    }
    #[inline(always)]
    fn bytes_pair(
        t: Neon,
        (_, a): (&[P], &mut [u8]),
        (_, b): (&[P], &mut [u8]),
        values: uint8x16_t,
    ) {
        store_pair(a, b, u8x16::<Neon>::simd_from(t, values).to_array());
    }
}

/// Calls `$f::<H, V, N>` for the shape key `15 * (N / 16) + 5 * (H / 2) +
/// V` of a strip kernel, `N` 8 or 16, `H` 0, 2 or 4 and `V` 0 to 4; a
/// 16-column strip has one pass.
macro_rules! strip_shapes {
    ($key:expr, $f:ident::<$o:ident> $args:tt) => {
        strip_shapes!(@ $key, $f $o $args, [
            0 0 0 8, 1 0 1 8, 2 0 2 8, 3 0 3 8, 4 0 4 8, 5 2 0 8, 6 2 1 8, 7 2 2 8, 8 2 3 8,
            9 2 4 8, 10 4 0 8, 11 4 1 8, 12 4 2 8, 13 4 3 8, 14 4 4 8, 15 0 0 16, 16 0 1 16,
            17 0 2 16, 18 0 3 16, 19 0 4 16, 20 2 0 16, 25 4 0 16
        ])
    };
    (@ $key:expr, $f:ident $o:ident $args:tt, [$($k:literal $hh:literal $vv:literal $n:literal),*]) => {
        match $key {
            $($k => $f::<$hh, $vv, $n, $o> $args,)*
            _ => {}
        }
    };
}

/// Calls `$f::<H, V>` of a 4-column block for the shape key `5 * (H / 2) +
/// V`, `H` 0 or 2 and `V` 0 to 4.
macro_rules! pair_shapes {
    ($key:expr, $f:ident::<$o:ident> $args:tt) => {
        pair_shapes!(@ $key, $f $o $args,
            [0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 2 0, 6 2 1, 7 2 2, 8 2 3, 9 2 4])
    };
    (@ $key:expr, $f:ident $o:ident $args:tt, [$($k:literal $hh:literal $vv:literal),*]) => {
        match $key {
            $($k => $f::<$hh, $vv, $o> $args,)*
            _ => {}
        }
    };
}

/// [`cell_groups`] of the `W`-square cell whose passes have `$h` and `$v`
/// tap pairs, which [`is_cell`] admits, read in place or with `$c` clamped;
/// zero for any other shape.
macro_rules! cells {
    ($h:expr, $v:expr, $c:expr, $args:tt) => {
        if W == 4 {
            cells!(@ 3 * $h + $v, $c, $args, [0 0 0, 1 0 1, 2 0 2, 3 1 0, 4 1 1, 5 1 2, 6 2 0, 7 2 1, 8 2 2])
        } else {
            cells!(@ 5 * $h + $v, $c, $args, [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4, 10 2 0,
                11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3, 19 3 4, 20 4 0,
                21 4 1, 22 4 2, 23 4 3, 24 4 4
            ])
        }
    };
    (@ $key:expr, $c:expr, $args:tt, [$($k:literal $hh:literal $vv:literal),*]) => {
        match ($key, $c) {
            $(($k, false) => cell_groups::<W, $hh, $vv, false> $args,)*
            $(($k, true) => cell_groups::<W, $hh, $vv, true> $args,)*
            _ => [vdupq_n_s16(0); W],
        }
    };
}

/// A reference of a register cell: its [`Reads`], whether they are
/// clamped, and the taps of both passes.
type Placed = (Reads, bool, Option<&'static Taps>, Option<&'static Taps>);

/// The [`Placed`] reads of the `W`-square register cell of `filter` at
/// `placement` in a plane of `geometry`; `None` for a cell [`is_cell`] does
/// not admit or an empty clip range.
#[inline(always)]
fn place<const W: usize>(
    filter: usize,
    (start, bounds): CellPlacement,
    geometry: Geometry,
) -> Option<Placed> {
    place_reads::<W>(
        &Window::of_filters([filter; 2], W, W, start),
        geometry,
        bounds,
    )
}

/// [`place`] of the window of a `W`-square cell.
#[inline(always)]
fn place_reads<const W: usize>(
    window: &Window,
    geometry: Geometry,
    bounds: [i32; 4],
) -> Option<Placed> {
    let (horizontal, vertical) = (window.horizontal, window.vertical);
    if window.w != W || !is_cell(window.w, window.h, horizontal, vertical) {
        return None;
    }
    let (reads, clamped) = reads(window, geometry, bounds)?;
    Some((reads, clamped, horizontal, vertical))
}

/// Both references' samples, the output and its stride of one plane of a
/// [`Program`].
pub(in crate::subpel_mc) type ProgramPlane<'a, 'b> = ([&'b [u8]; 2], &'a mut [u8], usize);

/// A program of compound cells of `subpel_mc::cell` for the kernels: each
/// cell whose windows are read in place is stored into both `planes` (U and
/// V share the geometry) and marked in the result.
pub(in crate::subpel_mc) struct Program<'a, 'b> {
    pub(in crate::subpel_mc) planes: [Option<ProgramPlane<'a, 'b>>; 2],
    pub(in crate::subpel_mc) geometry: [Geometry; 2],
    pub(in crate::subpel_mc) filter: usize,
    pub(in crate::subpel_mc) cells: &'a [super::super::cell::Cell],
    pub(in crate::subpel_mc) forward: i32,
}

/// The kernels of `P` blocks of one window, each into its `O`.
pub(in crate::subpel_mc) struct Strips<'a, 'b, O, const P: usize = 1>(
    [&'a Block<'b, u8>; P],
    [&'a mut O; P],
);

/// The kernels of `P` compound pairs of one window shape, each into its
/// `u8` storage: with `CELL` 4x4 or 8x8 register cells, else the first
/// predictor staged for the second.
pub(in crate::subpel_mc) struct Compound<'a, 'b, const CELL: bool, const P: usize>(
    [[&'a Block<'b, u8>; 2]; P],
    i32,
    [(&'a mut [u8], usize); P],
);

/// The kernels of a compound cell of two windows within their bounds, read
/// from each reference's samples of its geometry, into `u8` storage.
pub(in crate::subpel_mc) struct Windows<'a, 'b>(
    [(&'b [u8], Geometry); 2],
    [&'a Window; 2],
    [[i32; 4]; 2],
    i32,
    &'a mut [u8],
    usize,
);

/// The NEON token, which the AArch64 baseline always has.
#[inline(always)]
pub(in crate::subpel_mc) fn token() -> Option<Neon> {
    fearless_simd::Level::baseline().as_neon()
}

/// Stores the COMPOUND_AVERAGE of the 4x4 or 8x8 cell of `windows` within
/// `bounds` read from `views`, in place or by clamped rows and columns, or
/// returns `false` without writing when a clamped row cannot load 16
/// samples or the output cannot hold the cell.
#[inline]
pub(in crate::subpel_mc) fn compound_windows(
    views: [&ReferencePlaneView<'_, u8>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let references = views.map(|view| (view.samples, view.geometry()));
    let windows = [&windows[0], &windows[1]];
    let job = Windows(references, windows, bounds, forward, output, stride);
    token().is_some_and(|t| dot::run(t, job))
}

/// The kernels of a [`Program`] of `size`-square cells.
pub(in crate::subpel_mc) struct Programs<'a, 'b>(Program<'a, 'b>, usize, &'a mut Written);

/// Runs the cells of a program of `size`-square cells, 4 or 8, whose
/// windows are read in place or by clamped rows and columns into both
/// `planes`, and returns their bits.
#[inline]
pub(in crate::subpel_mc) fn program(
    planes: &mut [Option<Plane<'_, '_, u8>>; 2],
    (filter, size): (usize, usize),
    cells: &[Cell],
    (forward, _): (i32, u16),
) -> Written {
    let mut written = [[0; 4]; 2];
    let (Some(t), Some((views, ..))) = (token(), &planes[0]) else {
        return written;
    };
    let geometry = views.each_ref().map(ReferencePlaneView::geometry);
    let planes = planes.each_mut().map(|plane| {
        let (views, output, stride) = plane.as_mut()?;
        Some(([views[0].samples, views[1].samples], &mut **output, *stride))
    });
    let program = Program {
        planes,
        geometry,
        filter,
        cells,
        forward,
    };
    dot::run(t, Programs(program, size, &mut written));
    written
}

/// Predicts `block` into an eight-bit `sink`, as [`super::predict`] admits.
#[inline]
pub(in crate::subpel_mc) fn predict(block: &Block<'_, u8>, sink: &mut Sink<'_>) {
    let Some(t) = token() else { return };
    let w = block.w;
    match sink {
        Sink::Put { output, stride } => {
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut Put {
                        output,
                        stride: *stride,
                    }],
                ),
            );
        }
        Sink::PutWide { output, stride, .. } => {
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut PutWide {
                        output,
                        stride: *stride,
                    }],
                ),
            );
        }
        Sink::Prep { output, stride } => {
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut Prep {
                        output,
                        stride: *stride,
                    }],
                ),
            );
        }
        Sink::PrepNarrow { output, stride } => {
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut Prep {
                        output,
                        stride: *stride,
                    }],
                ),
            );
        }
        Sink::Average {
            pred0,
            forward,
            output,
            stride,
        } => {
            let (forward, stride) = (*forward, *stride);
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    }],
                ),
            );
        }
        Sink::AverageNarrow {
            pred0,
            forward,
            output,
            stride,
        } => {
            let (forward, stride) = (*forward, *stride);
            dot::run(
                t,
                Strips(
                    [block],
                    [&mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    }],
                ),
            );
        }
        Sink::AverageWide { .. } => {}
    }
}

/// Stores the `Clip1` single-reference prediction of `block`.
#[inline]
pub(in crate::subpel_mc) fn put(block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
    run(Strips([block], [&mut Put { output, stride }]));
}

/// Runs `job` in the kernel family [`dot::run`] picks.
#[inline(always)]
fn run<J: dot::Job<Output = ()>>(job: J) {
    if let Some(t) = token() {
        dot::run(t, job);
    }
}

/// [`put`] of two blocks of one window into `outputs`.
#[inline]
pub(in crate::subpel_mc) fn put_pair(
    blocks: [&Block<'_, u8>; 2],
    [(first, first_stride), (second, second_stride)]: [(&mut [u8], usize); 2],
) {
    let first = &mut Put {
        output: first,
        stride: first_stride,
    };
    let second = &mut Put {
        output: second,
        stride: second_stride,
    };
    run(Strips(blocks, [first, second]));
}

/// The COMPOUND_AVERAGE of each of `P` 4x4 or 8x8 register cell pairs into
/// its output.
#[inline]
pub(in crate::subpel_mc) fn compound_cell<const P: usize>(
    pairs: [[&Block<'_, u8>; 2]; P],
    forward: i32,
    outputs: [(&mut [u8], usize); P],
) {
    run(Compound::<true, P>(pairs, forward, outputs));
}

/// The COMPOUND_AVERAGE of two full-sample blocks into `output`.
#[inline]
pub(in crate::subpel_mc) fn compound_fullpel(
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    if let Some(t) = token() {
        fullpel_pair(t, blocks, forward, output, stride);
    }
}

fearless_simd::kernel!(
    /// [`fullpel`] entered through the NEON token.
    #[inline]
    fn fullpel_pair(
        t: Neon,
        blocks: [&Block<'_, u8>; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) {
        fullpel(t, blocks, forward, output, stride);
    }
);

/// The COMPOUND_AVERAGE of each of `P` compound pairs into its output, the
/// first predictor staged for the second.
#[inline]
pub(in crate::subpel_mc) fn compound_strips<const P: usize>(
    pairs: [[&Block<'_, u8>; 2]; P],
    forward: i32,
    outputs: [(&mut [u8], usize); P],
) {
    run(Compound::<false, P>(pairs, forward, outputs));
}

fearless_simd::kernel!(
    /// [`mla::strips`] entered through the NEON token.
    #[inline]
    fn mla_strips(t: Neon, block: &Block<'_, u8>, out: &mut impl Out) {
        mla::strips(t, block, out);
    }
);

fearless_simd::kernel!(
    /// [`mla::compound_cell`] or [`mla::compound_strips`] entered through
    /// the NEON token.
    #[inline]
    fn mla_compound(
        t: Neon,
        cell: bool,
        blocks: [&Block<'_, u8>; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) {
        if cell {
            mla::compound_cell(t, blocks, forward, output, stride);
        } else {
            mla::compound_strips(t, blocks, forward, output, stride);
        }
    }
);

fearless_simd::kernel!(
    /// [`mla::windows_cell`] entered through the NEON token.
    #[inline]
    fn mla_windows(t: Neon, job: Windows<'_, '_>) -> bool {
        if job.1[0].w == 4 {
            mla::windows_cell::<4>(t, job)
        } else {
            mla::windows_cell::<8>(t, job)
        }
    }
);

fearless_simd::kernel!(
    /// [`mla::run_program`] entered through the NEON token.
    #[inline]
    fn mla_program(t: Neon, job: Programs<'_, '_>) {
        *job.2 = if job.1 == 4 {
            mla::run_program::<4>(t, job.0)
        } else {
            mla::run_program::<8>(t, job.0)
        };
    }
);

pub(in crate::subpel_mc) mod dot;
mod kernels;

/// The kernels whose horizontal sums multiply-accumulate halved taps.
mod mla {
    use super::*;
    use super::{mla_line as hline, pair_sums as hpair, row_sums_of as hrow, weights as taps};

    super::kernels::kernels!("neon");
}

/// The kernels whose horizontal sums are `usdot` dot products.
mod usdot {
    use super::*;
    use super::{
        dot_pair_sums as hpair, dot_taps as taps, usdot_line as hline, usdot_row_sums as hrow,
    };

    super::kernels::kernels!("neon,dotprod,i8mm");
}

/// The kernels whose horizontal sums are `sdot` dot products.
mod sdot {
    use super::*;
    use super::{
        dot_line as hline, dot_pair_sums as hpair, dot_row_sums as hrow, dot_taps as taps,
    };

    super::kernels::kernels!("neon,dotprod");
}

/// The halved taps of a horizontal pass as `i8`, taps `0..4` in word 0 and
/// `4..8` in word 1, zero past its pairs.
#[inline]
#[target_feature(enable = "neon")]
fn dot_taps(_: Neon, taps: Option<&Taps>) -> int8x16_t {
    vcombine_s8(vcreate_s8(taps.map_or(0, |taps| taps.dot)), vdup_n_s8(0))
}

/// The clamping [`Reads`] of a cell read in place, which only a clamped
/// cell kernel reads.
static IN_PLACE: Reads = Reads {
    base: 0,
    stride: 0,
    top: 0,
    first: 0,
    last: 0,
    columns: [0; 3],
};

/// Byte lane indices `0..16`.
const BYTE_LANES: [i8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

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
    dot_line::<P, 8>(bytes, bytes, taps, 0)[0]
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

/// The halved-tap sums plus `bias` of four outputs of each of the window
/// rows `a` and `b`, the first row in lanes `0..4`, as [`pair_sums`] of at
/// most two tap pairs.
#[inline]
#[target_feature(enable = "neon,dotprod")]
fn dot_pair_sums(a: uint8x8_t, b: uint8x8_t, taps: int8x16_t, bias: i32) -> int16x8_t {
    let samples = vreinterpretq_s8_u8(veorq_u8(vcombine_u8(a, b), vdupq_n_u8(0x80)));
    let bias = vdupq_n_s32(128 * 64 + bias);
    let windows = [dot_windows::<0>(), dot_windows::<2>()];
    let low = vdotq_laneq_s32::<0>(bias, vqtbl1q_s8(samples, windows[0]), taps);
    let high = vdotq_laneq_s32::<0>(bias, vqtbl1q_s8(samples, windows[1]), taps);
    vuzp1q_s16(vreinterpretq_s16_s32(low), vreinterpretq_s16_s32(high))
}

/// [`dot_row_sums`] in `usdot`, which weighs the unbiased samples.
#[inline]
#[target_feature(enable = "neon,i8mm")]
fn usdot_row_sums<const P: usize>(bytes: uint8x16_t, taps: int8x16_t) -> int16x8_t {
    usdot_line::<P, 8>(bytes, bytes, taps, 0)[0]
}

/// [`dot_line`] in `usdot`.
#[inline]
#[target_feature(enable = "neon,i8mm")]
fn usdot_line<const P: usize, const N: usize>(
    a: uint8x16_t,
    b: uint8x16_t,
    taps: int8x16_t,
    bias: i32,
) -> [int16x8_t; 2] {
    let bias = vdupq_n_s32(bias);
    let windows = [dot_windows::<0>(), dot_windows::<1>(), dot_windows::<2>()];
    let a = windows.map(|window| vqtbl1q_u8(a, window));
    let sum = |first: uint8x16_t, second: uint8x16_t| {
        let sum = dot::usdot_lane::<0>(bias, first, taps);
        if P > 2 {
            dot::usdot_lane::<1>(sum, second, taps)
        } else {
            sum
        }
    };
    let pack = |low, high| vuzp1q_s16(vreinterpretq_s16_s32(low), vreinterpretq_s16_s32(high));
    let low = pack(sum(a[0], a[1]), sum(a[1], a[2]));
    if N == 8 {
        return [low, low];
    }
    let (b1, b2) = (vqtbl1q_u8(b, windows[1]), vqtbl1q_u8(b, windows[2]));
    [low, pack(sum(a[2], b1), sum(b1, b2))]
}

/// Byte indices of window columns `k..k + 4` (lanes `0..4`) and `k + 1..k +
/// 5` (lanes `8..12`) of a row in bytes `0..8`, and the same of a row in
/// bytes `8..16` in lanes `4..8` and `12..16`, for `k` 0 and 2, as two
/// little-endian words.
const PAIR_TAPS: [[u64; 2]; 2] = {
    let mut out = [[0; 2]; 2];
    let mut pair = 0;
    while pair < 2 {
        let mut lane = 0;
        while lane < 16 {
            let byte = 2 * pair + lane / 8 + lane % 4 + lane / 4 % 2 * 8;
            out[pair][lane / 8] |= (byte as u64) << (8 * (lane % 8));
            lane += 1;
        }
        pair += 1;
    }
    out
};

/// The halved-tap horizontal sums plus `bias` of four outputs of each of
/// the window rows `a` and `b`, the first row in lanes `0..4`.
#[inline]
#[target_feature(enable = "neon")]
fn pair_sums(a: uint8x8_t, b: uint8x8_t, weights: int16x8_t, bias: i32) -> int16x8_t {
    let bytes = vcombine_u8(a, b);
    let zero = vdupq_n_u8(0);
    let pair = |[low, high]: [u64; 2]| {
        let lanes = vqtbl1q_u8(bytes, vcombine_u8(vcreate_u8(low), vcreate_u8(high)));
        (
            vreinterpretq_s16_u8(vzip1q_u8(lanes, zero)),
            vreinterpretq_s16_u8(vzip2q_u8(lanes, zero)),
        )
    };
    let (first, second) = (pair(PAIR_TAPS[0]), pair(PAIR_TAPS[1]));
    let sums = sums16::<2>(weights, |k| match k {
        0 => first.0,
        1 => first.1,
        2 => second.0,
        _ => second.1,
    });
    vaddq_s16(sums, vdupq_n_s16(bias as i16))
}

/// The COMPOUND_AVERAGE of the groups of two `W`-square cells.
#[inline]
#[target_feature(enable = "neon")]
fn store_cell<const W: usize>(
    t: Neon,
    first: [int16x8_t; W],
    second: [int16x8_t; W],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let groups = first.into_iter().zip(second).enumerate().take(W * W / 8);
    let at = |group: usize| {
        if W == 4 {
            2 * group * stride
        } else {
            group * stride
        }
    };
    for (group, (first, second)) in groups {
        let values = if forward == 8 {
            vqrshrun_n_s16::<5>(vaddq_s16(first, second))
        } else {
            vqmovun_s16(blend(first, second, forward))
        };
        let Some(row) = output.get_mut(at(group)..) else {
            return;
        };
        if W == 8 {
            if let Some(slot) = row.first_chunk_mut::<8>() {
                *slot = vget_lane_u64::<0>(vreinterpret_u64_u8(values)).to_le_bytes();
            }
        } else if let Some((a, b)) = row.split_at_mut_checked(stride) {
            store_pair(
                a,
                b,
                u8x16::<Neon>::simd_from(t, vcombine_u8(values, values)).to_array(),
            );
        }
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

/// The compound pair of two full-sample blocks: the COMPOUND_AVERAGE of the
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

/// The rows of [`fullpel`] averaged by `average`: 16-sample chunks of each
/// row, the last one overlapping the one before it, or one 4- or 8-sample
/// group of a narrower row.
#[inline]
#[target_feature(enable = "neon")]
fn fullpel_rows<'a>(
    t: Neon,
    blocks: [&Block<'a, u8>; 2],
    output: &mut [u8],
    stride: usize,
    average: impl Fn(uint8x16_t, uint8x16_t) -> uint8x16_t,
) {
    let [first, second] = blocks;
    let (w, h) = (first.w, first.h);
    let Some((targets, target)) = rows_mut(output, 0, stride, w, h) else {
        return;
    };
    if w < 16 {
        for (row, out) in targets.chain([target]).enumerate() {
            let bytes = average(
                load16(t, first.source, first.start(row)),
                load16(t, second.source, second.start(row)),
            );
            let bytes = u8x16::<Neon>::simd_from(t, bytes).to_array();
            if w == 4 {
                if let (Some(slot), Some(bytes)) = (out.first_chunk_mut::<4>(), bytes.first_chunk())
                {
                    *slot = *bytes;
                }
                continue;
            }
            let tail = w - 8;
            if let (Some(slot), Some(bytes)) = (out.first_chunk_mut::<8>(), bytes.first_chunk()) {
                *slot = *bytes;
            }
            if let (Some(slot), Some(bytes)) = (
                out.get_mut(tail..).and_then(<[u8]>::first_chunk_mut::<8>),
                bytes.get(tail..).and_then(<[u8]>::first_chunk::<8>),
            ) {
                *slot = *bytes;
            }
        }
        return;
    }
    let line = |block: &Block<'a, u8>| rows(block.source, block.origin, block.stride, w, h);
    let (Some((rows0, last0)), Some((rows1, last1))) = (line(first), line(second)) else {
        return;
    };
    let load = |row: &[u8; 16]| -> uint8x16_t { u8x16::load_array_ref(t, row).into() };
    let lines = rows0.chain([last0]).zip(rows1.chain([last1]));
    for ((row0, row1), out) in lines.zip(targets.chain([target])) {
        let (Some(row0), Some(row1), Some(out)) = (row0.get(..w), row1.get(..w), out.get_mut(..w))
        else {
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
