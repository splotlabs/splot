// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 7.13.5 4:2:0 8-bit CfL luma `L` of a whole block in one AVX2
//! pass: `pmaddubsw` pairs the centre and right taps of each luma row, and
//! the left tap where the filter has one.

use super::cfl::CFL_FILTERS_420;

/// A block whose every luma tap lies inside the stored plane, so no tap
/// clamps at a frame edge.
#[derive(Clone, Copy)]
pub(super) struct Luma420Block {
    /// Index of luma sample `(2 * x, 2 * y)` in the plane samples.
    pub(super) origin: usize,
    pub(super) stride: usize,
    /// Luma coordinates of the block origin, for the § 7.13.5 64-sample clamps.
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    /// Chroma block width.
    pub(super) width: usize,
    /// The CfL `L` clamps: the first row and column, and every 64th luma
    /// row and column; otherwise only the first row or column clamps, when
    /// [`Self::clamp_row0`] or [`Self::clamp_col0`].
    pub(super) periodic: bool,
    pub(super) clamp_row0: bool,
    pub(super) clamp_col0: bool,
}

impl Luma420Block {
    /// Whether every load of a `height`-row block stays inside `len` samples.
    pub(super) fn fits(&self, height: usize, len: usize) -> bool {
        let row_reach = if self.width <= 8 {
            16
        } else {
            2 * self.width.next_multiple_of(16)
        };
        let first = if self.clamp_row0 {
            Some(self.origin)
        } else {
            self.origin.checked_sub(self.stride)
        };
        self.luma_x >= 2
            && first.is_some_and(|first| first >= 3)
            && self.origin + (2 * height - 1) * self.stride + row_reach <= len
    }
}

/// The luma rows of tap rows top, centre and bottom of chroma row `r`.
#[inline]
fn tap_rows(block: &Luma420Block, r: usize) -> [usize; 3] {
    let mid = block.origin + 2 * r * block.stride;
    let clamp = (r == 0 && block.clamp_row0)
        || (block.periodic && (block.luma_y + 2 * r).is_multiple_of(64));
    [
        if clamp { mid } else { mid - block.stride },
        mid,
        mid + block.stride,
    ]
}

/// The § 7.13.5 sum of `weights[dy + 1][dx + 1]` around the centre samples
/// `centres` (top, middle, bottom row); the left tap reads the centre when
/// `clamp_left`.
#[allow(clippy::inline_always, reason = "folds the constant filter taps")]
#[inline(always)]
fn tap_sum(luma: &[u8], weights: &[[i32; 3]; 3], centres: [usize; 3], clamp_left: bool) -> i32 {
    let mut total = 0;
    for (&centre, weights) in centres.iter().zip(weights) {
        let cols = [centre - usize::from(!clamp_left), centre, centre + 1];
        for (&col, &weight) in cols.iter().zip(weights) {
            if weight != 0 {
                total += weight * i32::from(luma[col]);
            }
        }
    }
    total
}

/// `pmaddubsw` byte-pair weights of each tap row: the (centre, right) pair
/// read from the centre column and the (left, centre) pair from one before.
const fn pair_taps(weights: &[[i32; 3]; 3]) -> [(i16, i16); 3] {
    let mut taps = [(0i16, 0i16); 3];
    let mut row = 0;
    while row < 3 {
        let [left, centre, right] = weights[row];
        taps[row] = (
            ((centre as u8) as i16) | (((right as u8) as i16) << 8),
            (left as u8) as i16,
        );
        row += 1;
    }
    taps
}

/// Writes `L` of the block rows with the 4:2:0 filter `F` into `out` (row
/// `r` at `r * width`) and, when given, the downsampled luma left of each
/// row into `left`; `false`, writing nothing, for a filter outside 4:2:0.
#[inline]
pub(super) fn luma_420<const F: usize>(
    token: fearless_simd::Avx2,
    luma: &[u8],
    block: Luma420Block,
    mut left: Option<&mut [i32]>,
    out: &mut [i32],
) -> bool {
    let Some(&weights) = CFL_FILTERS_420.get(F) else {
        return false;
    };
    luma_420_rows_avx2(token, pair_taps(&weights), luma, block, out);
    let clamp_cols = weights.iter().any(|row| row[0] != 0) && block.clamp_col0;
    if !clamp_cols && left.is_none() {
        return true;
    }
    let (luma_x, width) = (block.luma_x, block.width);
    for (r, out) in out.chunks_exact_mut(width).enumerate() {
        let rows = tap_rows(&block, r);
        if clamp_cols {
            let mut j = 0;
            while j < width {
                out[j] = tap_sum(luma, &weights, rows.map(|row| row + 2 * j), true);
                j = if block.periodic {
                    ((luma_x + 2 * j) / 64 + 1) * 32 - luma_x / 2
                } else {
                    width
                };
            }
        }
        if let Some(left) = left.as_deref_mut() {
            let [_, mid, bottom] = rows;
            let top = if r == 0 { mid } else { mid - block.stride };
            left[r] = tap_sum(
                luma,
                &weights,
                [top, mid, bottom].map(|row| row - 2),
                luma_x == 2,
            );
        }
    }
    true
}

fearless_simd::kernel!(
    /// The `pmaddubsw` part of [`luma_420`]: every sample with its left tap
    /// unclamped.
    #[inline]
    fn luma_420_rows_avx2(
        token: Avx2,
        taps: [(i16, i16); 3],
        luma: &[u8],
        block: Luma420Block,
        out: &mut [i32],
    ) {
        use core::arch::x86_64::{
            __m128i, __m256i, _mm_add_epi16, _mm_maddubs_epi16, _mm_set1_epi16, _mm256_add_epi16,
            _mm256_castsi256_si128, _mm256_cvtepi16_epi32, _mm256_extracti128_si256,
            _mm256_maddubs_epi16, _mm256_set1_epi16, _mm256_setzero_si256,
        };
        use fearless_simd::{Simd as _, SimdBase as _, SimdFrom as _, i32x8, u8x16, u8x32};
        let width = block.width;
        for (r, out) in out.chunks_exact_mut(width).enumerate() {
            let rows = tap_rows(&block, r);
            if width > 8 {
                for g in (0..width).step_by(16) {
                    let mut acc = _mm256_setzero_si256();
                    for (&row, &(centre_right, left_centre)) in rows.iter().zip(&taps) {
                        let at = row + 2 * g;
                        if centre_right != 0 {
                            let bytes: __m256i =
                                u8x32::from_slice(token, &luma[at..at + 32]).into();
                            let pairs = _mm256_set1_epi16(centre_right);
                            acc = _mm256_add_epi16(acc, _mm256_maddubs_epi16(bytes, pairs));
                        }
                        if left_centre != 0 {
                            let bytes: __m256i =
                                u8x32::from_slice(token, &luma[at - 1..at + 31]).into();
                            let pairs = _mm256_set1_epi16(left_centre);
                            acc = _mm256_add_epi16(acc, _mm256_maddubs_epi16(bytes, pairs));
                        }
                    }
                    let low =
                        i32x8::simd_from(token, _mm256_cvtepi16_epi32(_mm256_castsi256_si128(acc)));
                    let high = i32x8::simd_from(
                        token,
                        _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(acc)),
                    );
                    if let Some(out) = out.get_mut(g..g + 16) {
                        low.store_slice(&mut out[..8]);
                        high.store_slice(&mut out[8..]);
                    } else {
                        let lanes: [[i32; 8]; 2] = [low.into(), high.into()];
                        for (slot, &lane) in out[g..].iter_mut().zip(lanes.as_flattened()) {
                            *slot = lane;
                        }
                    }
                }
            } else {
                let mut acc: __m128i = u8x16::splat(token, 0).into();
                for (&row, &(centre_right, left_centre)) in rows.iter().zip(&taps) {
                    if centre_right != 0 {
                        let bytes: __m128i = u8x16::from_slice(token, &luma[row..row + 16]).into();
                        let pairs = _mm_set1_epi16(centre_right);
                        acc = _mm_add_epi16(acc, _mm_maddubs_epi16(bytes, pairs));
                    }
                    if left_centre != 0 {
                        let bytes: __m128i =
                            u8x16::from_slice(token, &luma[row - 1..row + 15]).into();
                        let pairs = _mm_set1_epi16(left_centre);
                        acc = _mm_add_epi16(acc, _mm_maddubs_epi16(bytes, pairs));
                    }
                }
                let samples = i32x8::simd_from(token, _mm256_cvtepi16_epi32(acc));
                if let Some(out) = out.first_chunk_mut::<8>() {
                    samples.store_slice(out);
                } else if let Ok(out) = <&mut [i32; 4]>::try_from(&mut *out) {
                    token.split_i32x8(samples).0.store_slice(out);
                } else {
                    let lanes: [i32; 8] = samples.into();
                    for (slot, lane) in out.iter_mut().zip(lanes) {
                        *slot = lane;
                    }
                }
            }
        }
    }
);
