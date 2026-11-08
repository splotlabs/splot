// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 eight-bit blocks: both
//! references of a compound cell and their § 7.13.3.15 COMPOUND_AVERAGE, or
//! one single-reference block and its `Clip1`.
//!
//! A block whose windows lie inside their clip bounds and storage runs every
//! pass in registers, with the arithmetic of the byte-lane kernels: a
//! horizontal pass is `Round2(sum, InterRound0)` of halved taps (or the
//! single-pass put `Round2`), a vertical-only pass weighs the samples by the
//! halved taps, and a two-pass vertical rounds the intermediates. Every other
//! block takes the general path.

use super::block::{TAPS, Taps};
use super::{
    CellPlacement, InterpolationFilter, ReferencePlaneView, SCALE_SUBPEL_BITS, SUBPEL_MASK,
    SubpelPredictParams,
};

/// One reference of a cell: window row `r` starts at `origin + r * stride`,
/// or, for a window that crosses its clip bounds, at the bounded row
/// `Clip3(clamped[1], clamped[2], clamped[0] + r)` and window column `c`
/// at `Clip3(0, clamped[4], c + clamped[3])` from `origin`.
#[derive(Clone, Copy)]
pub(super) struct Cell<'a> {
    source: &'a [u8],
    origin: usize,
    stride: usize,
    horizontal: Option<&'static Taps>,
    vertical: Option<&'static Taps>,
    clamped: Option<[i32; 5]>,
}

impl<'a> Cell<'a> {
    /// The cell of a `size`-square block, or `None` when its clip range is
    /// empty or its window leaves the storage of 16 bytes per row.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn new(
        view: &ReferencePlaneView<'a, u8>,
        params: &SubpelPredictParams,
        size: usize,
    ) -> Option<Self> {
        Self::at(
            view,
            params.interp,
            size,
            ([params.start_x, params.start_y], params.bounds()),
        )
    }

    /// [`Self::new`] of a cell at 1/1024-sample `[x, y]` with clip bounds
    /// `[firstX, firstY, lastX, lastY]`.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn at(
        view: &ReferencePlaneView<'a, u8>,
        interp: InterpolationFilter,
        size: usize,
        ([start_x, start_y], bounds): CellPlacement,
    ) -> Option<Self> {
        let filters = &TAPS[interp.pass_index(size as u32) as usize];
        let taps = |start: i32| filters[((start >> 6) & SUBPEL_MASK) as usize].as_ref();
        let (horizontal, vertical) = (taps(start_x), taps(start_y));
        let window = |start: i32, taps: Option<&Taps>| {
            let base = start >> SCALE_SUBPEL_BITS;
            taps.map_or((base, size as i32), |taps| {
                (
                    base - 3 + taps.start as i32,
                    (size + taps.span() - 1) as i32,
                )
            })
        };
        let ((column, columns), (row, rows)) =
            (window(start_x, horizontal), window(start_y, vertical));
        let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
        let (width, height) = (view.width, view.readable_rows);
        let [first_x, last_x] = [bound(bounds[0], width), bound(bounds[2], width)];
        let [first_y, last_y] = [bound(bounds[1], height), bound(bounds[3], height)];
        if first_x > last_x || first_y > last_y {
            return None;
        }
        let inside = column >= first_x
            && column + columns <= last_x + 1
            && row >= first_y
            && row + rows <= last_y + 1;
        let fits = |row: i32, column: i32| {
            row as usize * view.stride + column as usize + 16 <= view.samples.len()
        };
        let cell = |origin: usize, clamped| Self {
            source: view.samples,
            origin,
            stride: view.stride,
            horizontal,
            vertical,
            clamped,
        };
        if inside {
            return fits(row + rows - 1, column)
                .then(|| cell(row as usize * view.stride + column as usize, None));
        }
        let base = column.max(first_x).min(last_x);
        let columns = [(column - base).max(-16), (last_x - base).min(15)];
        fits((row + rows - 1).max(first_y).min(last_y), base).then(|| {
            cell(
                base as usize,
                Some([row, first_y, last_y, columns[0], columns[1]]),
            )
        })
    }
}

/// The cell of a 4x4 or 8x8 single-reference block that [`Cell::put`]
/// stores, or `None` outside that subset.
#[inline]
pub(super) fn put_cell<'a>(
    reference: &ReferencePlaneView<'a, u8>,
    params: &SubpelPredictParams,
) -> Option<Cell<'a>> {
    let size = params.w;
    if params.h != size || (size != 4 && size != 8) {
        return None;
    }
    let cell = Cell::new(reference, params, size)?;
    if size == 4 && cell.horizontal.is_some_and(|taps| taps.count > 2) {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    if fearless_simd::Level::baseline().as_avx2().is_some() {
        return Some(cell);
    }
    None
}

impl<'a> Cell<'a> {
    /// This cell read from `source`, storage of the same geometry.
    pub(super) fn with_source(&self, source: &'a [u8]) -> Self {
        Self { source, ..*self }
    }

    /// Stores the `Clip1` single-reference prediction of this `size`-square
    /// cell of [`put_cell`].
    pub(super) fn put(&self, size: usize, output: &mut [u8], stride: usize) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
            avx2::put(token, self, size, output, stride);
        }
        let _ = (size, output, stride);
    }
}

/// Predicts both references of a 4x4 or 8x8 cell and stores their
/// COMPOUND_AVERAGE, or returns `false` without writing outside the subset.
pub(super) fn compound(
    references: [&ReferencePlaneView<'_, u8>; 2],
    params: [&SubpelPredictParams; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let size = params[0].w;
    params[0].h == size
        && compound_at(
            references,
            params[0].interp,
            size,
            params.map(|params| ([params.start_x, params.start_y], params.bounds())),
            forward,
            output,
            stride,
        )
}

/// [`compound`] of a `size`-square cell from each reference's
/// [`CellPlacement`].
#[inline]
pub(super) fn compound_at(
    references: [&ReferencePlaneView<'_, u8>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    if size != 4 && size != 8 {
        return false;
    }
    let (Some(first), Some(second)) = (
        Cell::at(references[0], interp, size, placements[0]),
        Cell::at(references[1], interp, size, placements[1]),
    ) else {
        return false;
    };
    if size == 4
        && [&first, &second]
            .iter()
            .any(|cell| cell.horizontal.is_some_and(|taps| taps.count > 2))
    {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        avx2::compound(token, [&first, &second], size, forward, output, stride);
        return true;
    }
    let _ = (first, second, forward, output, stride);
    false
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::Cell;
    use crate::subpel_mc::eight_bit::avx2::{Horizontal, average, store_bytes, tap_bytes};
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, u8x32};

    /// The 16 predictors of each group of a cell; a 4x4 cell has one.
    type Groups = [__m256i; 4];

    /// `pshufb` pattern widening columns `0..4` of the two 8-byte rows of
    /// each lane to words.
    const NARROW_ROWS: [u8; 32] = {
        let mut out = [0x80; 32];
        let mut lane = 0;
        while lane < 32 {
            if lane % 2 == 0 {
                let word = (lane % 16) / 2;
                out[lane] = ((word / 4) * 8 + word % 4) as u8;
            }
            lane += 1;
        }
        out
    };

    /// Calls `$f::<$w, H, V, $put, CLAMPED>` for the tap-pair counts of a
    /// cell's passes and whether its window is clamped.
    macro_rules! by_taps {
        ($w:literal, $put:literal, $h:expr, $v:expr, $clamped:expr, $f:ident($($arg:expr),*)) => {
            if $clamped {
                by_taps!(@ $w, $put, true, $h, $v, $f($($arg),*))
            } else {
                by_taps!(@ $w, $put, false, $h, $v, $f($($arg),*))
            }
        };
        (@ $w:literal, $put:literal, $c:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<$w, 0, 0, $put, $c>($($arg),*),
                (0, 1) => $f::<$w, 0, 1, $put, $c>($($arg),*),
                (0, 2) => $f::<$w, 0, 2, $put, $c>($($arg),*),
                (0, 3) => $f::<$w, 0, 3, $put, $c>($($arg),*),
                (0, _) => $f::<$w, 0, 4, $put, $c>($($arg),*),
                (1, 0) => $f::<$w, 1, 0, $put, $c>($($arg),*),
                (1, 1) => $f::<$w, 1, 1, $put, $c>($($arg),*),
                (1, 2) => $f::<$w, 1, 2, $put, $c>($($arg),*),
                (1, 3) => $f::<$w, 1, 3, $put, $c>($($arg),*),
                (1, _) => $f::<$w, 1, 4, $put, $c>($($arg),*),
                (2, 0) => $f::<$w, 2, 0, $put, $c>($($arg),*),
                (2, 1) => $f::<$w, 2, 1, $put, $c>($($arg),*),
                (2, 2) => $f::<$w, 2, 2, $put, $c>($($arg),*),
                (2, 3) => $f::<$w, 2, 3, $put, $c>($($arg),*),
                (2, _) => $f::<$w, 2, 4, $put, $c>($($arg),*),
                (3, 0) => $f::<$w, 3, 0, $put, $c>($($arg),*),
                (3, 1) => $f::<$w, 3, 1, $put, $c>($($arg),*),
                (3, 2) => $f::<$w, 3, 2, $put, $c>($($arg),*),
                (3, 3) => $f::<$w, 3, 3, $put, $c>($($arg),*),
                (3, _) => $f::<$w, 3, 4, $put, $c>($($arg),*),
                (_, 0) => $f::<$w, 4, 0, $put, $c>($($arg),*),
                (_, 1) => $f::<$w, 4, 1, $put, $c>($($arg),*),
                (_, 2) => $f::<$w, 4, 2, $put, $c>($($arg),*),
                (_, 3) => $f::<$w, 4, 3, $put, $c>($($arg),*),
                (_, _) => $f::<$w, 4, 4, $put, $c>($($arg),*),
            }
        };
    }

    /// [`by_taps`] of a 4x4 cell, whose horizontal pass has at most two
    /// tap pairs.
    macro_rules! by_narrow_taps {
        ($put:literal, $h:expr, $v:expr, $clamped:expr, $f:ident($($arg:expr),*)) => {
            if $clamped {
                by_narrow_taps!(@ $put, true, $h, $v, $f($($arg),*))
            } else {
                by_narrow_taps!(@ $put, false, $h, $v, $f($($arg),*))
            }
        };
        (@ $put:literal, $c:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<4, 0, 0, $put, $c>($($arg),*),
                (0, 1) => $f::<4, 0, 1, $put, $c>($($arg),*),
                (0, 2) => $f::<4, 0, 2, $put, $c>($($arg),*),
                (0, 3) => $f::<4, 0, 3, $put, $c>($($arg),*),
                (0, _) => $f::<4, 0, 4, $put, $c>($($arg),*),
                (1, 0) => $f::<4, 1, 0, $put, $c>($($arg),*),
                (1, 1) => $f::<4, 1, 1, $put, $c>($($arg),*),
                (1, 2) => $f::<4, 1, 2, $put, $c>($($arg),*),
                (1, 3) => $f::<4, 1, 3, $put, $c>($($arg),*),
                (1, _) => $f::<4, 1, 4, $put, $c>($($arg),*),
                (_, 0) => $f::<4, 2, 0, $put, $c>($($arg),*),
                (_, 1) => $f::<4, 2, 1, $put, $c>($($arg),*),
                (_, 2) => $f::<4, 2, 2, $put, $c>($($arg),*),
                (_, 3) => $f::<4, 2, 3, $put, $c>($($arg),*),
                (_, _) => $f::<4, 2, 4, $put, $c>($($arg),*),
            }
        };
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn compound(
            t: Avx2,
            cells: [&Cell<'_>; 2],
            size: usize,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            let counts = |cell: &Cell<'_>| {
                (
                    cell.horizontal.map_or(0, |taps| taps.count),
                    cell.vertical.map_or(0, |taps| taps.count),
                )
            };
            let ([h0, v0], [h1, v1]) = (counts(cells[0]).into(), counts(cells[1]).into());
            if size == 4 {
                let first = by_narrow_taps!(
                    false,
                    h0,
                    v0,
                    cells[0].clamped.is_some(),
                    predicted(t, cells[0])
                );
                let second = by_narrow_taps!(
                    false,
                    h1,
                    v1,
                    cells[1].clamped.is_some(),
                    predicted(t, cells[1])
                );
                store_bytes::<4>(t, output, 0, stride, average(first[0], second[0], forward));
                return;
            }
            let first = by_taps!(
                8,
                false,
                h0,
                v0,
                cells[0].clamped.is_some(),
                predicted(t, cells[0])
            );
            let second = by_taps!(
                8,
                false,
                h1,
                v1,
                cells[1].clamped.is_some(),
                predicted(t, cells[1])
            );
            for (group, (&first, &second)) in first.iter().zip(&second).enumerate() {
                let blended = average(first, second, forward);
                store_bytes::<8>(t, output, 2 * group * stride, stride, blended);
            }
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(t: Avx2, cell: &Cell<'_>, size: usize, output: &mut [u8], stride: usize) {
            let h = cell.horizontal.map_or(0, |taps| taps.count);
            let v = cell.vertical.map_or(0, |taps| taps.count);
            if size == 4 {
                by_narrow_taps!(
                    true,
                    h,
                    v,
                    cell.clamped.is_some(),
                    put_cell(t, cell, output, stride)
                );
            } else {
                by_taps!(
                    8,
                    true,
                    h,
                    v,
                    cell.clamped.is_some(),
                    put_cell(t, cell, output, stride)
                );
            }
        }
    );

    /// Stores the [`predict`] put of one cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn put_cell<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
        output: &mut [u8],
        stride: usize,
    ) {
        let groups = predict::<W, H, V, PUT, CLAMPED>(t, cell);
        if W == 4 {
            store_bytes::<4>(t, output, 0, stride, groups[0]);
            return;
        }
        for (group, &values) in groups.iter().enumerate() {
            store_bytes::<8>(t, output, 2 * group * stride, stride, values);
        }
    }

    /// [`predict`] out of line, for the two references of a compound cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn predicted<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
    ) -> Groups {
        predict::<W, H, V, PUT, CLAMPED>(t, cell)
    }

    /// The compound predictors of one `W`-square cell whose passes have `H`
    /// and `V` tap pairs, none for a skipped pass.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn predict<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
    ) -> Groups {
        let per = 16 / W;
        let rows = if V > 0 { W + 2 * V - 1 } else { W };
        let zero = _mm256_setzero_si256();
        let (bounds, remap) = match cell.clamped.filter(|_| CLAMPED) {
            Some(bounds) => {
                let lanes = _mm_add_epi8(
                    _mm_setr_epi8(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
                    _mm_set1_epi8(bounds[3] as i8),
                );
                let limit = _mm_set1_epi8(bounds[4] as i8);
                (
                    bounds,
                    Some(_mm_min_epi8(
                        _mm_max_epi8(lanes, _mm_setzero_si128()),
                        limit,
                    )),
                )
            }
            None => ([0; 5], None),
        };
        let start = |row: usize| {
            if CLAMPED {
                let row = (bounds[0] + row.min(rows - 1) as i32)
                    .max(bounds[1])
                    .min(bounds[2]);
                cell.origin + row as usize * cell.stride
            } else {
                cell.origin + row.min(rows - 1) * cell.stride
            }
        };
        let filter = match cell.horizontal {
            Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
            _ => None,
        };
        let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
        let mut lines = [zero; 8];
        for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
            let row = group * per;
            let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
            let Some(bytes) = tap_bytes::<W>(t, cell.source, starts, remap) else {
                return [zero; 4];
            };
            *line = match &filter {
                Some(filter) if PUT && V == 0 => _mm256_srai_epi16::<6>(_mm256_add_epi16(
                    filter.sums::<H>(bytes),
                    _mm256_set1_epi16(34),
                )),
                Some(filter) => {
                    _mm256_mulhrs_epi16(filter.sums::<H>(bytes), _mm256_set1_epi16(1 << 13))
                }
                None if W == 4 => _mm256_shuffle_epi8(bytes, narrow),
                None => _mm256_unpacklo_epi8(bytes, zero),
            };
        }
        let mut out = [zero; 4];
        let Some(taps) = cell.vertical.filter(|_| V > 0) else {
            for (out, &line) in out.iter_mut().zip(&lines) {
                *out = if H > 0 || PUT {
                    line
                } else {
                    _mm256_slli_epi16::<4>(line)
                };
            }
            return out;
        };
        let line = |row: usize| -> __m256i {
            let (index, offset) = (row / per, row % per);
            let first = lines[index.min(7)];
            if offset == 0 {
                return first;
            }
            let second = lines[(index + 1).min(7)];
            let middle = _mm256_permute2x128_si256::<0x21>(first, second);
            match (W, offset) {
                (8, _) | (_, 2) => middle,
                (_, 1) => _mm256_alignr_epi8::<8>(middle, first),
                _ => _mm256_alignr_epi8::<8>(second, middle),
            }
        };
        let weights: [__m256i; 4] =
            core::array::from_fn(|pair| _mm256_set1_epi32(taps.words[pair.min(V.max(1) - 1)]));
        let bias = _mm256_set1_epi32(match (H > 0, PUT) {
            (true, true) => 1 << 9,
            (true, false) | (false, true) => 1 << 5,
            (false, false) => 1 << 1,
        });
        for (group, out) in out.iter_mut().enumerate().take(W * W / 16) {
            let (mut low, mut high) = (bias, bias);
            for (pair, &weights) in weights.iter().enumerate().take(V) {
                let (a, b) = (
                    line(group * per + 2 * pair),
                    line(group * per + 2 * pair + 1),
                );
                low =
                    _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights));
                high = _mm256_add_epi32(
                    high,
                    _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
                );
            }
            *out = match (H > 0, PUT) {
                (true, true) => {
                    _mm256_packs_epi32(_mm256_srai_epi32::<10>(low), _mm256_srai_epi32::<10>(high))
                }
                (true, false) | (false, true) => {
                    _mm256_packs_epi32(_mm256_srai_epi32::<6>(low), _mm256_srai_epi32::<6>(high))
                }
                (false, false) => {
                    _mm256_packs_epi32(_mm256_srai_epi32::<2>(low), _mm256_srai_epi32::<2>(high))
                }
            };
        }
        out
    }
}
