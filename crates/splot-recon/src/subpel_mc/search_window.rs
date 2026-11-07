// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit § 7.13.3.6 refine-MV search: the bilinear window of each
//! reference as bytes, and the SADs between two windows.

use super::*;

/// Predicts the `w x h` bilinear refine-MV search window of `params` (`w`
/// and `h` each 16 or 24) into `output` at stride `w` as bytes, or returns
/// `false` without writing for any other block. Its clip bounds are its
/// inner `w - 1` columns and `h - 1` rows, so the outer column and row read
/// their inner neighbour twice.
#[must_use]
pub fn subpel_predict_bilinear_search_window_u8<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
) -> bool {
    let (w, h) = (params.w, params.h);
    let (x0, y0) = (
        params.start_x >> SCALE_SUBPEL_BITS,
        params.start_y >> SCALE_SUBPEL_BITS,
    );
    let Some(view) = view.as_u8() else {
        return false;
    };
    if params.interp != InterpolationFilter::Bilinear
        || (params.step_x, params.step_y) != (1 << SCALE_SUBPEL_BITS, 1 << SCALE_SUBPEL_BITS)
        || !matches!((w, h), (16 | 24, 16 | 24))
        || [params.first_x, params.last_x, params.first_y, params.last_y]
            != [x0 + 1, x0 + w as i32 - 1, y0 + 1, y0 + h as i32 - 1]
        || x0 + 1 < 0
        || y0 + 1 < 0
        || params.last_x >= view.width as i32
        || params.last_y >= view.readable_rows as i32
        || output.len() < w * h
    {
        return false;
    }
    let origin = (y0 + 1) as usize * view.stride + (x0 + 1) as usize;
    let last_load = origin + (h - 2) * view.stride + if w == 24 { 31 } else { 16 };
    if last_load > view.samples.len() {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        let phases = [
            ((params.start_x >> 6) & SUBPEL_MASK) as i16,
            ((params.start_y >> 6) & SUBPEL_MASK) as i16,
        ];
        avx2::window(t, view.samples, [origin, view.stride, w, h], phases, output);
        return true;
    }
    false
}

/// Writes into `sads` the § 7.13.3.6 SAD of two `stride`-wide search windows
/// at each `(dy, dx)` of `offsets`: `Σ |w0 - w1|` over every other row of the
/// `sad_w x sad_h` areas starting at `(2 + dy, 2 + dx)` in the first window
/// and at `(2 - dy, 2 - dx)` in the second. Returns `false`, having written
/// nothing, unless `stride` is `sad_w + 4`, `sad_w` and `sad_h` 12 or 20, every offset
/// within `-2..=2` and the windows hold `stride x (sad_h + 4)` samples.
#[must_use]
pub fn refinemv_window_sads_u8(
    windows: [&[u8]; 2],
    stride: usize,
    [sad_w, sad_h]: [usize; 2],
    offsets: &[(i32, i32)],
    sads: &mut [u32],
) -> bool {
    let area = stride * (sad_h + 4);
    if !matches!(sad_w, 12 | 20)
        || !matches!(sad_h, 12 | 20)
        || stride != sad_w + 4
        || windows.iter().any(|window| window.len() < area)
        || sads.len() < offsets.len()
        || offsets
            .iter()
            .any(|&(dy, dx)| !(-2..=2).contains(&dy) || !(-2..=2).contains(&dx))
    {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        let windows = [&windows[0][..area], &windows[1][..area]];
        avx2::sads(t, windows, stride, [sad_w, sad_h], offsets, sads);
        return true;
    }
    false
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, u8x16, u8x32};

    /// The `(left, right)` source index pairs of output columns `first..first
    /// + 16` of an `m`-sample row read from index `base`, both clamped to the
    /// row, as `pshufb` bytes for two lanes of eight columns.
    const fn pairs(first: usize, base: usize, m: usize) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut lane = 0;
        while lane < 16 {
            let column = first + lane;
            let left = if column == 0 { 0 } else { column - 1 };
            let left = if left > m - 1 { m - 1 } else { left };
            let right = if column > m - 1 { m - 1 } else { column };
            out[2 * lane] = (left - base) as u8;
            out[2 * lane + 1] = (right - base) as u8;
            lane += 1;
        }
        out
    }

    const PAIRS_16: [u8; 32] = pairs(0, 0, 15);
    const PAIRS_24_LOW: [u8; 32] = pairs(0, 0, 23);
    const PAIRS_24_HIGH: [u8; 32] = pairs(16, 15, 23);

    /// Lanes below `n` set: the mask of an `n`-sample SAD row.
    const fn row_mask(n: usize) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut lane = 0;
        while lane < n {
            out[lane] = u8::MAX;
            lane += 1;
        }
        out
    }

    const MASK_12: [u8; 32] = {
        let mut out = row_mask(12);
        let mut lane = 16;
        while lane < 28 {
            out[lane] = u8::MAX;
            lane += 1;
        }
        out
    };
    const MASK_20: [u8; 32] = row_mask(20);

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load16(t: Avx2, bytes: &[u8], at: usize) -> __m128i {
        match bytes.get(at..).and_then(<[u8]>::first_chunk::<16>) {
            Some(bytes) => u8x16::load_array_ref(t, bytes).into(),
            None => _mm_setzero_si128(),
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load32(t: Avx2, bytes: &[u8], at: usize) -> __m256i {
        match bytes.get(at..).and_then(<[u8]>::first_chunk::<32>) {
            Some(bytes) => u8x32::load_array_ref(t, bytes).into(),
            None => _mm256_setzero_si256(),
        }
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn window(
            t: Avx2,
            source: &[u8],
            geometry: [usize; 4],
            phases: [i16; 2],
            output: &mut [u8],
        ) {
            let [origin, stride, w, h] = geometry;
            let [h_phase, v_phase] = phases;
            let weights = _mm256_set1_epi16(((h_phase << 8) | (16 - h_phase)) as i16);
            let (low_pairs, high_pairs): (__m256i, __m256i) = if w == 24 {
                (
                    u8x32::load_array(t, PAIRS_24_LOW).into(),
                    u8x32::load_array(t, PAIRS_24_HIGH).into(),
                )
            } else {
                (
                    u8x32::load_array(t, PAIRS_16).into(),
                    _mm256_setzero_si256(),
                )
            };
            let horizontal = |row: usize| -> [__m256i; 2] {
                let at = origin + row * stride;
                let low = _mm256_broadcastsi128_si256(load16(t, source, at));
                let low = _mm256_maddubs_epi16(_mm256_shuffle_epi8(low, low_pairs), weights);
                if w != 24 {
                    return [low, low];
                }
                let high = _mm256_broadcastsi128_si256(load16(t, source, at + 15));
                [
                    low,
                    _mm256_maddubs_epi16(_mm256_shuffle_epi8(high, high_pairs), weights),
                ]
            };
            let (top_weight, bottom_weight) =
                (_mm256_set1_epi16(16 - v_phase), _mm256_set1_epi16(v_phase));
            let bias = _mm256_set1_epi16(128);
            let blend = |top: __m256i, bottom: __m256i| {
                let sum = _mm256_add_epi16(
                    _mm256_mullo_epi16(top, top_weight),
                    _mm256_mullo_epi16(bottom, bottom_weight),
                );
                _mm256_srli_epi16::<8>(_mm256_add_epi16(sum, bias))
            };
            let mut write = |row: usize, top: [__m256i; 2], bottom: [__m256i; 2]| {
                let low = blend(top[0], bottom[0]);
                let at = row * w;
                let high = if w == 24 {
                    blend(top[1], bottom[1])
                } else {
                    low
                };
                let packed =
                    _mm256_permute4x64_epi64::<0b1101_1000>(_mm256_packus_epi16(low, high));
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<16>) {
                    u8x16::<Avx2>::simd_from(t, _mm256_castsi256_si128(packed)).store_array(slot);
                }
                if w == 24
                    && let Some(slot) = output
                        .get_mut(at + 16..)
                        .and_then(<[u8]>::first_chunk_mut::<8>)
                {
                    *slot = _mm_cvtsi128_si64(_mm256_extracti128_si256::<1>(packed)).to_le_bytes();
                }
            };
            let mut top = horizontal(0);
            write(0, top, top);
            for row in 1..h - 1 {
                let bottom = horizontal(row);
                write(row, top, bottom);
                top = bottom;
            }
            write(h - 1, top, top);
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn sads(
            t: Avx2,
            windows: [&[u8]; 2],
            stride: usize,
            area: [usize; 2],
            offsets: &[(i32, i32)],
            out: &mut [u32],
        ) {
            let [sad_w, sad_h] = area;
            let mask: __m256i = if sad_w == 12 {
                u8x32::load_array(t, MASK_12).into()
            } else {
                u8x32::load_array(t, MASK_20).into()
            };
            let start = |dy: i32, dx: i32| ((2 + dy) as usize) * stride + (2 + dx) as usize;
            for (&(dy, dx), out) in offsets.iter().zip(out.iter_mut()) {
                let (left, right) = (start(dy, dx), start(-dy, -dx));
                let mut sum = _mm256_setzero_si256();
                for row in (0..sad_h).step_by(if sad_w == 12 { 4 } else { 2 }) {
                    let load = |window: &[u8], at: usize| {
                        if sad_w == 12 {
                            _mm256_set_m128i(
                                load16(t, window, at + 2 * stride),
                                load16(t, window, at),
                            )
                        } else {
                            load32(t, window, at)
                        }
                    };
                    let (a, b) = (
                        load(windows[0], left + row * stride),
                        load(windows[1], right + row * stride),
                    );
                    let diff =
                        _mm256_sad_epu8(_mm256_and_si256(a, mask), _mm256_and_si256(b, mask));
                    sum = _mm256_add_epi64(sum, diff);
                }
                let folded = _mm_add_epi64(
                    _mm256_castsi256_si128(sum),
                    _mm256_extracti128_si256::<1>(sum),
                );
                *out = (_mm_cvtsi128_si64(folded) + _mm_extract_epi64::<1>(folded)) as u32;
            }
        }
    );
}
