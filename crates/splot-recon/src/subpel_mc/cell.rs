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
use super::{ReferencePlaneView, SCALE_SUBPEL_BITS, SUBPEL_MASK, SubpelPredictParams};

/// One reference of a cell: window row `r` starts at `origin + r * stride`.
pub(super) struct Cell<'a> {
    source: &'a [u8],
    origin: usize,
    stride: usize,
    horizontal: Option<&'static Taps>,
    vertical: Option<&'static Taps>,
}

impl<'a> Cell<'a> {
    /// The cell of a `size`-square block, or `None` when its window leaves
    /// the plane-bounded clip range or the storage of 16 bytes per row.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn new(
        view: &ReferencePlaneView<'a, u8>,
        params: &SubpelPredictParams,
        size: usize,
    ) -> Option<Self> {
        let filters = &TAPS[params.interp.pass_index(size as u32) as usize];
        let taps = |start: i32| filters[((start >> 6) & SUBPEL_MASK) as usize].as_ref();
        let (horizontal, vertical) = (taps(params.start_x), taps(params.start_y));
        let window = |start: i32, taps: Option<&Taps>| {
            let base = start >> SCALE_SUBPEL_BITS;
            taps.map_or((base, size as i32), |taps| {
                (
                    base - 3 + taps.start as i32,
                    (size + taps.span() - 1) as i32,
                )
            })
        };
        let ((column, columns), (row, rows)) = (
            window(params.start_x, horizontal),
            window(params.start_y, vertical),
        );
        let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
        let (width, height) = (view.width, view.readable_rows);
        let inside = column >= bound(params.first_x, width)
            && column + columns <= bound(params.last_x, width) + 1
            && row >= bound(params.first_y, height)
            && row + rows <= bound(params.last_y, height) + 1;
        if !inside {
            return None;
        }
        let origin = row as usize * view.stride + column as usize;
        (origin + (rows as usize - 1) * view.stride + 16 <= view.samples.len()).then_some(Self {
            source: view.samples,
            origin,
            stride: view.stride,
            horizontal,
            vertical,
        })
    }
}

/// Stores the `Clip1` single-reference prediction of a 4x4 or 8x8 block, or
/// returns `false` without writing outside the subset.
pub(super) fn put(
    reference: &ReferencePlaneView<'_, u8>,
    params: &SubpelPredictParams,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let size = params.w;
    if params.h != size || (size != 4 && size != 8) {
        return false;
    }
    let Some(cell) = Cell::new(reference, params, size) else {
        return false;
    };
    if size == 4 && cell.horizontal.is_some_and(|taps| taps.count > 2) {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        avx2::put(token, &cell, size, output, stride);
        return true;
    }
    let _ = (cell, output, stride);
    false
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
    if params[0].h != size || (size != 4 && size != 8) {
        return false;
    }
    let (Some(first), Some(second)) = (
        Cell::new(references[0], params[0], size),
        Cell::new(references[1], params[1], size),
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

    /// Calls `$f::<$w, H, V>` for the tap-pair counts of a cell's passes.
    macro_rules! by_taps {
        ($w:literal, $put:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<$w, 0, 0, $put>($($arg),*),
                (0, 1) => $f::<$w, 0, 1, $put>($($arg),*),
                (0, 2) => $f::<$w, 0, 2, $put>($($arg),*),
                (0, 3) => $f::<$w, 0, 3, $put>($($arg),*),
                (0, _) => $f::<$w, 0, 4, $put>($($arg),*),
                (1, 0) => $f::<$w, 1, 0, $put>($($arg),*),
                (1, 1) => $f::<$w, 1, 1, $put>($($arg),*),
                (1, 2) => $f::<$w, 1, 2, $put>($($arg),*),
                (1, 3) => $f::<$w, 1, 3, $put>($($arg),*),
                (1, _) => $f::<$w, 1, 4, $put>($($arg),*),
                (2, 0) => $f::<$w, 2, 0, $put>($($arg),*),
                (2, 1) => $f::<$w, 2, 1, $put>($($arg),*),
                (2, 2) => $f::<$w, 2, 2, $put>($($arg),*),
                (2, 3) => $f::<$w, 2, 3, $put>($($arg),*),
                (2, _) => $f::<$w, 2, 4, $put>($($arg),*),
                (3, 0) => $f::<$w, 3, 0, $put>($($arg),*),
                (3, 1) => $f::<$w, 3, 1, $put>($($arg),*),
                (3, 2) => $f::<$w, 3, 2, $put>($($arg),*),
                (3, 3) => $f::<$w, 3, 3, $put>($($arg),*),
                (3, _) => $f::<$w, 3, 4, $put>($($arg),*),
                (_, 0) => $f::<$w, 4, 0, $put>($($arg),*),
                (_, 1) => $f::<$w, 4, 1, $put>($($arg),*),
                (_, 2) => $f::<$w, 4, 2, $put>($($arg),*),
                (_, 3) => $f::<$w, 4, 3, $put>($($arg),*),
                (_, _) => $f::<$w, 4, 4, $put>($($arg),*),
            }
        };
    }

    /// [`by_taps`] of a 4x4 cell, whose horizontal pass has at most two
    /// tap pairs.
    macro_rules! by_narrow_taps {
        ($put:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<4, 0, 0, $put>($($arg),*),
                (0, 1) => $f::<4, 0, 1, $put>($($arg),*),
                (0, 2) => $f::<4, 0, 2, $put>($($arg),*),
                (0, 3) => $f::<4, 0, 3, $put>($($arg),*),
                (0, _) => $f::<4, 0, 4, $put>($($arg),*),
                (1, 0) => $f::<4, 1, 0, $put>($($arg),*),
                (1, 1) => $f::<4, 1, 1, $put>($($arg),*),
                (1, 2) => $f::<4, 1, 2, $put>($($arg),*),
                (1, 3) => $f::<4, 1, 3, $put>($($arg),*),
                (1, _) => $f::<4, 1, 4, $put>($($arg),*),
                (_, 0) => $f::<4, 2, 0, $put>($($arg),*),
                (_, 1) => $f::<4, 2, 1, $put>($($arg),*),
                (_, 2) => $f::<4, 2, 2, $put>($($arg),*),
                (_, 3) => $f::<4, 2, 3, $put>($($arg),*),
                (_, _) => $f::<4, 2, 4, $put>($($arg),*),
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
                let first = by_narrow_taps!(false, h0, v0, predict(t, cells[0]));
                let second = by_narrow_taps!(false, h1, v1, predict(t, cells[1]));
                store_bytes::<4>(t, output, 0, stride, average(first[0], second[0], forward));
                return;
            }
            let first = by_taps!(8, false, h0, v0, predict(t, cells[0]));
            let second = by_taps!(8, false, h1, v1, predict(t, cells[1]));
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
                let groups = by_narrow_taps!(true, h, v, predict(t, cell));
                store_bytes::<4>(t, output, 0, stride, groups[0]);
                return;
            }
            let groups = by_taps!(8, true, h, v, predict(t, cell));
            for (group, &values) in groups.iter().enumerate() {
                store_bytes::<8>(t, output, 2 * group * stride, stride, values);
            }
        }
    );

    /// The compound predictors of one `W`-square cell whose passes have `H`
    /// and `V` tap pairs, none for a skipped pass.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn predict<const W: usize, const H: usize, const V: usize, const PUT: bool>(
        t: Avx2,
        cell: &Cell<'_>,
    ) -> Groups {
        let per = 16 / W;
        let rows = if V > 0 { W + 2 * V - 1 } else { W };
        let zero = _mm256_setzero_si256();
        let start = |row: usize| cell.origin + row.min(rows - 1) * cell.stride;
        let filter = match cell.horizontal {
            Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
            _ => None,
        };
        let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
        let mut lines = [zero; 8];
        for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
            let row = group * per;
            let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
            let Some(bytes) = tap_bytes::<W>(t, cell.source, starts, None) else {
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
