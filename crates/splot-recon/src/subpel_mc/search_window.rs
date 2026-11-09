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
    if last_load > view.samples.len() || u32::try_from(last_load).is_err() {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        let phases = [
            ((params.start_x >> 6) & SUBPEL_MASK) as i16,
            ((params.start_y >> 6) & SUBPEL_MASK) as i16,
        ];
        avx2::window(
            t,
            view.samples,
            [origin as u32, view.stride as u32],
            [w, h],
            phases,
            output,
        );
        return true;
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(t) = fearless_simd::Level::baseline().as_neon() {
        let phases = [
            ((params.start_x >> 6) & SUBPEL_MASK) as u8,
            ((params.start_y >> 6) & SUBPEL_MASK) as u8,
        ];
        neon::window(
            t,
            view.samples,
            [origin, view.stride],
            [w, h],
            phases,
            output,
        );
        return true;
    }
    false
}

/// Predicts a bilinear block of eight-bit storage, `w` a multiple of 8 and
/// `h` even, whose taps all lie inside its clip bounds and the plane into
/// `output` at `stride`, or returns `false` without writing for any other
/// block.
#[must_use]
pub fn subpel_predict_bilinear_u8<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    stride: usize,
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
        || w == 0
        || !w.is_multiple_of(8)
        || h == 0
        || !h.is_multiple_of(2)
        || stride < w
        || output.len() < (h - 1) * stride + w
        || x0 < params.first_x.max(0)
        || y0 < params.first_y.max(0)
        || x0 + w as i32 > params.last_x.min(view.width as i32 - 1)
        || y0 + h as i32 > params.last_y.min(view.readable_rows as i32 - 1)
    {
        return false;
    }
    let origin = y0 as usize * view.stride + x0 as usize;
    let last_load = origin + h * view.stride + w + 8;
    if last_load > view.samples.len() || u32::try_from(last_load).is_err() {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        let phases = [
            ((params.start_x >> 6) & SUBPEL_MASK) as i16,
            ((params.start_y >> 6) & SUBPEL_MASK) as i16,
        ];
        avx2::block(
            t,
            view.samples,
            ([origin as u32, view.stride as u32], [w, h]),
            phases,
            output,
            stride,
        );
        return true;
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(t) = fearless_simd::Level::baseline().as_neon() {
        let phases = [
            ((params.start_x >> 6) & SUBPEL_MASK) as u8,
            ((params.start_y >> 6) & SUBPEL_MASK) as u8,
        ];
        let geometry = ([origin, view.stride], [w, h]);
        neon::block(t, view.samples, geometry, phases, output, stride);
        return true;
    }
    false
}

/// The § 7.13.3.6 search positions `(dy, dx)` in `Refinemv_Neighbors` order.
pub const REFINEMV_NEIGHBORS: [(i32, i32); 24] = {
    let mut out = [(0, 0); 24];
    let mut index = 0;
    let mut position = 0;
    while position < 25 {
        if position != 12 {
            out[index] = (position / 5 - 2, position % 5 - 2);
            index += 1;
        }
        position += 1;
    }
    out
};

/// The § 7.13.3.6 search offset `(dx, dy)` of two `stride`-wide eight-bit
/// search windows: the biased centre SAD when `allow_center` keeps it, else
/// the best of the 24 neighbours, each `Σ |w0 - w1|` over every other row of
/// the `(stride - 4) x sad_h` areas at `(2 + dy, 2 + dx)` in the first window
/// and at `(2 - dy, 2 - dx)` in the second. Returns `None` unless `stride` is
/// 16 or 24, `sad_h` is 12 or 20 and each window holds its
/// `stride x (sad_h + 4)` bytes.
#[must_use]
pub fn refinemv_search_u8(
    windows: [&[u8]; 2],
    stride: usize,
    sad_h: usize,
    allow_center: bool,
) -> Option<(i32, i32)> {
    if !matches!(stride, 16 | 24)
        || !matches!(sad_h, 12 | 20)
        || windows
            .iter()
            .any(|window| window.len() < stride * (sad_h + 4))
    {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        return Some(avx2::search(t, windows, [stride, sad_h], allow_center));
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(t) = fearless_simd::Level::baseline().as_neon() {
        return Some(neon::search(t, windows, [stride, sad_h], allow_center));
    }
    let sad = |dy: i32, dx: i32| {
        let at = |sign: i32, row: usize, col: usize| {
            ((2 + sign * dy) as usize + row) * stride + (2 + sign * dx) as usize + col
        };
        (0..sad_h)
            .step_by(2)
            .flat_map(|row| (0..stride - 4).map(move |col| (row, col)))
            .map(|(row, col)| {
                u32::from(windows[0][at(1, row, col)].abs_diff(windows[1][at(-1, row, col)]))
            })
            .sum::<u32>()
    };
    Some(best_offset(
        || sad(0, 0),
        || REFINEMV_NEIGHBORS.map(|(dy, dx)| sad(dy, dx)),
        (2 * (stride - 4) * sad_h) as u32,
        allow_center,
    ))
}

/// The § 7.13.3.6 search over the biased `center` SAD, kept when below
/// `threshold`, and the SADs of the 24 neighbours in [`REFINEMV_NEIGHBORS`] order.
#[allow(clippy::inline_always, reason = "per refine-MV unit search")]
#[inline(always)]
fn best_offset(
    center: impl FnOnce() -> u32,
    neighbours: impl FnOnce() -> [u32; 24],
    threshold: u32,
    allow_center: bool,
) -> (i32, i32) {
    let mut best = (0, 0);
    let mut best_sad = u32::MAX;
    if allow_center {
        let center = center();
        best_sad = center - (center >> 3);
        if best_sad < threshold {
            return best;
        }
    }
    for ((dy, dx), sad) in REFINEMV_NEIGHBORS.into_iter().zip(neighbours()) {
        if sad < best_sad {
            best_sad = sad;
            best = (dx, dy);
        }
    }
    best
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use crate::subpel_mc::eight_bit::avx2::row_bytes;
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, u8x16, u8x32, u16x16};

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
            at: [u32; 2],
            size: [usize; 2],
            phases: [i16; 2],
            output: &mut [u8],
        ) {
            match size {
                [16, 16] => window_for::<16, 16, 256>(t, source, at, phases, output),
                [16, _] => window_for::<16, 24, 384>(t, source, at, phases, output),
                [_, 16] => window_for::<24, 16, 384>(t, source, at, phases, output),
                _ => window_for::<24, 24, 576>(t, source, at, phases, output),
            };
        }
    );

    /// [`window`] of a `W x H` window of `N` bytes, whose output rows sit at
    /// fixed offsets.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn window_for<const W: usize, const H: usize, const N: usize>(
        t: Avx2,
        source: &[u8],
        [origin, stride]: [u32; 2],
        [h_phase, v_phase]: [i16; 2],
        output: &mut [u8],
    ) -> Option<()> {
        let output = output.first_chunk_mut::<N>()?;
        let weights = _mm256_set1_epi16((h_phase << 8) | (16 - h_phase));
        let (low_pairs, high_pairs): (__m256i, __m256i) = if W == 24 {
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
        let horizontal = |row: usize| -> Option<[__m256i; 2]> {
            let at = origin + row as u32 * stride;
            let half = |bytes: &[u8; 16]| -> __m256i {
                _mm256_broadcastsi128_si256(u8x16::load_array_ref(t, bytes).into())
            };
            if W != 24 {
                let low = half(row_bytes::<16>(source, at)?);
                let low = _mm256_maddubs_epi16(_mm256_shuffle_epi8(low, low_pairs), weights);
                return Some([low, low]);
            }
            let bytes = row_bytes::<31>(source, at)?;
            let (low, high) = (half(bytes.first_chunk()?), half(bytes.last_chunk()?));
            Some([
                _mm256_maddubs_epi16(_mm256_shuffle_epi8(low, low_pairs), weights),
                _mm256_maddubs_epi16(_mm256_shuffle_epi8(high, high_pairs), weights),
            ])
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
        let rows = output.as_chunks_mut::<W>().0;
        let mut write = |row: usize, top: [__m256i; 2], bottom: [__m256i; 2]| {
            let low = blend(top[0], bottom[0]);
            let high = if W == 24 {
                blend(top[1], bottom[1])
            } else {
                low
            };
            let packed = _mm256_permute4x64_epi64::<0b1101_1000>(_mm256_packus_epi16(low, high));
            let Some(slot) = rows.get_mut(row) else {
                return;
            };
            if let Some(bytes) = slot.first_chunk_mut::<16>() {
                u8x16::<Avx2>::simd_from(t, _mm256_castsi256_si128(packed)).store_array(bytes);
            }
            if W == 24
                && let Some(bytes) = slot.last_chunk_mut::<8>()
            {
                *bytes = _mm_cvtsi128_si64(_mm256_extracti128_si256::<1>(packed)).to_le_bytes();
            }
        };
        let mut top = horizontal(0)?;
        write(0, top, top);
        for row in 1..H - 1 {
            let bottom = horizontal(row)?;
            write(row, top, bottom);
            top = bottom;
        }
        write(H - 1, top, top);
        Some(())
    }

    /// The `(c, c + 1)` byte pairs of eight columns, in both lanes.
    const PAIRS_8: [u8; 32] = {
        let mut out = [0u8; 32];
        let mut lane = 0;
        while lane < 32 {
            let column = (lane % 16) / 2;
            out[lane] = (column + lane % 2) as u8;
            lane += 1;
        }
        out
    };

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn block(
            t: Avx2,
            source: &[u8],
            geometry: ([u32; 2], [usize; 2]),
            phases: [i16; 2],
            output: &mut [u16],
            stride: usize,
        ) {
            if geometry.1 == [8; 2] && stride == 8 {
                block_rows::<8>(t, source, geometry, phases, output, 8);
            } else {
                block_rows::<0>(t, source, geometry, phases, output, stride);
            }
        }
    );

    /// [`block`] of a `w x h` block, or of an `N x N` one at stride `N`
    /// when `N` is not zero.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn block_rows<const N: usize>(
        t: Avx2,
        source: &[u8],
        ([origin, source_stride], [w, h]): ([u32; 2], [usize; 2]),
        [h_phase, v_phase]: [i16; 2],
        output: &mut [u16],
        stride: usize,
    ) {
        let (w, h, stride) = if N > 0 { (N, N, N) } else { (w, h, stride) };
        let weights = _mm256_set1_epi16((h_phase << 8) | (16 - h_phase));
        let pairs: __m256i = u8x32::load_array(t, PAIRS_8).into();
        let (top_weight, bottom_weight) =
            (_mm256_set1_epi16(16 - v_phase), _mm256_set1_epi16(v_phase));
        let bias = _mm256_set1_epi16(128);
        for column in (0..w).step_by(8) {
            let rows = |first: usize| {
                let load = |row: usize| -> __m128i {
                    let at = origin + column as u32 + row.min(h) as u32 * source_stride;
                    row_bytes::<16>(source, at).map_or(_mm_setzero_si128(), |bytes| {
                        u8x16::load_array_ref(t, bytes).into()
                    })
                };
                let bytes = _mm256_set_m128i(load(first + 1), load(first));
                _mm256_maddubs_epi16(_mm256_shuffle_epi8(bytes, pairs), weights)
            };
            let mut top = rows(0);
            for pair in 0..h / 2 {
                let next = rows(2 * pair + 2);
                let bottom = _mm256_permute2x128_si256::<0x21>(top, next);
                let sum = _mm256_add_epi16(
                    _mm256_mullo_epi16(top, top_weight),
                    _mm256_mullo_epi16(bottom, bottom_weight),
                );
                let values: [u16; 16] = *u16x16::<Avx2>::simd_from(
                    t,
                    _mm256_srli_epi16::<8>(_mm256_add_epi16(sum, bias)),
                );
                for (half, values) in values.as_chunks::<8>().0.iter().enumerate() {
                    let at = (2 * pair + half) * stride + column;
                    if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>)
                    {
                        *slot = *values;
                    }
                }
                top = next;
            }
        }
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn search(
            t: Avx2,
            windows: [&[u8]; 2],
            geometry: [usize; 2],
            allow_center: bool,
        ) -> (i32, i32) {
            match geometry {
                [16, 12] => search_for::<16, 12, 176, 256>(t, windows, allow_center),
                [16, _] => search_for::<16, 20, 304, 384>(t, windows, allow_center),
                [_, 12] => search_for::<24, 12, 272, 384>(t, windows, allow_center),
                _ => search_for::<24, 20, 464, 576>(t, windows, allow_center),
            }
        }
    );

    /// [`search`] of `STRIDE`-wide windows of `AREA` bytes and `SAD_H`-row
    /// areas, each read as one `SPAN`-byte run: one masked 12-byte row per
    /// `xmm` at stride 16, one masked 20-byte row per `ymm` at stride 24.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn search_for<const STRIDE: usize, const SAD_H: usize, const SPAN: usize, const AREA: usize>(
        t: Avx2,
        [first, second]: [&[u8]; 2],
        allow_center: bool,
    ) -> (i32, i32) {
        let (Some(first), Some(second)) =
            (first.first_chunk::<AREA>(), second.first_chunk::<AREA>())
        else {
            return (0, 0);
        };
        let mask: __m256i = if STRIDE == 16 {
            u8x32::load_array(t, MASK_12).into()
        } else {
            u8x32::load_array(t, MASK_20).into()
        };
        macro_rules! sad {
            ($dy:expr, $dx:expr) => {{
                let start = |dy: i32, dx: i32| ((2 + dy) * STRIDE as i32 + 2 + dx) as usize;
                match (
                    first
                        .get(start($dy, $dx)..)
                        .and_then(<[u8]>::first_chunk::<SPAN>),
                    second
                        .get(start(-$dy, -$dx)..)
                        .and_then(<[u8]>::first_chunk::<SPAN>),
                ) {
                    (Some(left), Some(right)) if STRIDE == 16 => {
                        let mask = _mm256_castsi256_si128(mask);
                        let mut sum = _mm_setzero_si128();
                        for row in (0..SAD_H).step_by(2) {
                            let a = _mm_and_si128(load16(t, left, row * STRIDE), mask);
                            let b = _mm_and_si128(load16(t, right, row * STRIDE), mask);
                            sum = _mm_add_epi64(sum, _mm_sad_epu8(a, b));
                        }
                        (_mm_cvtsi128_si64(sum) + _mm_extract_epi64::<1>(sum)) as u32
                    }
                    (Some(left), Some(right)) => {
                        let mut sum = _mm256_setzero_si256();
                        for row in (0..SAD_H).step_by(2) {
                            let a = _mm256_and_si256(load32(t, left, row * STRIDE), mask);
                            let b = _mm256_and_si256(load32(t, right, row * STRIDE), mask);
                            sum = _mm256_add_epi64(sum, _mm256_sad_epu8(a, b));
                        }
                        let folded = _mm_add_epi64(
                            _mm256_castsi256_si128(sum),
                            _mm256_extracti128_si256::<1>(sum),
                        );
                        (_mm_cvtsi128_si64(folded) + _mm_extract_epi64::<1>(folded)) as u32
                    }
                    _ => u32::MAX,
                }
            }};
        }
        macro_rules! neighbours {
            ($(($dy:literal, $dx:literal)),*) => {
                [$(sad!($dy, $dx)),*]
            };
        }
        super::best_offset(
            || sad!(0, 0),
            || {
                neighbours!(
                    (-2, -2),
                    (-2, -1),
                    (-2, 0),
                    (-2, 1),
                    (-2, 2),
                    (-1, -2),
                    (-1, -1),
                    (-1, 0),
                    (-1, 1),
                    (-1, 2),
                    (0, -2),
                    (0, -1),
                    (0, 1),
                    (0, 2),
                    (1, -2),
                    (1, -1),
                    (1, 0),
                    (1, 1),
                    (1, 2),
                    (2, -2),
                    (2, -1),
                    (2, 0),
                    (2, 1),
                    (2, 2)
                )
            },
            (2 * (STRIDE - 4) * SAD_H) as u32,
            allow_center,
        )
    }
}

/// The bilinear windows and blocks in generic `u16` lanes, and the SADs in
/// NEON byte lanes.
#[cfg(target_arch = "aarch64")]
mod neon {
    use fearless_simd::{Neon, Simd, prelude::*, u8x16, u16x8, u64x2};

    /// The source index, from `base`, of the left or right tap of output
    /// columns `first..first + 16` of an `m`-sample row, clamped to the row.
    const fn taps(first: usize, base: usize, m: usize, right: bool) -> [u8; 16] {
        let mut out = [0u8; 16];
        let mut lane = 0;
        while lane < 16 {
            let column = first + lane;
            let index = if right || column == 0 {
                column
            } else {
                column - 1
            };
            out[lane] = ((if index > m - 1 { m - 1 } else { index }) - base) as u8;
            lane += 1;
        }
        out
    }

    /// Left and right taps of columns `0..16` of a 16-wide window, then of
    /// columns `0..16` and `16..24` of a 24-wide one.
    const TAPS: [[[u8; 16]; 2]; 3] = [
        [taps(0, 0, 15, false), taps(0, 0, 15, true)],
        [taps(0, 0, 23, false), taps(0, 0, 23, true)],
        [taps(16, 15, 23, false), taps(16, 15, 23, true)],
    ];

    #[allow(clippy::inline_always, reason = "per refine-MV unit search")]
    #[inline(always)]
    fn load<S: Simd>(simd: S, bytes: &[u8], at: usize) -> u8x16<S> {
        bytes
            .get(at..)
            .and_then(<[u8]>::first_chunk::<16>)
            .map_or(u8x16::splat(simd, 0), |bytes| {
                u8x16::load_array_ref(simd, bytes)
            })
    }

    /// `Round2((16 - phase) * top + phase * bottom, 8)` of two horizontal rows.
    #[allow(clippy::inline_always, reason = "per refine-MV unit search")]
    #[inline(always)]
    fn blend<S: Simd>(simd: S, phase: u8, top: u16x8<S>, bottom: u16x8<S>) -> u16x8<S> {
        let weights = (
            u16x8::splat(simd, 16 - u16::from(phase)),
            u16x8::splat(simd, u16::from(phase)),
        );
        (top * weights.0 + bottom * weights.1 + 128) >> 8
    }

    /// The `w x h` search window, `w` and `h` each 16 or 24, of the rows of
    /// `source` from `origin`, into `output` at stride `w`.
    #[inline(never)]
    pub(super) fn window<S: Simd>(
        simd: S,
        source: &[u8],
        [origin, stride]: [usize; 2],
        [width, height]: [usize; 2],
        [h_phase, v_phase]: [u8; 2],
        output: &mut [u8],
    ) {
        let weights = (
            u8x16::splat(simd, 16 - h_phase),
            u8x16::splat(simd, h_phase),
        );
        let tables = TAPS.map(|pair| pair.map(|taps| u8x16::load_array(simd, taps)));
        if [h_phase, v_phase] == [0, 0] {
            // Zero phases weigh only the left tap of the row above.
            for row in 0..height {
                let at = origin + (row.max(1) - 1) * stride;
                let Some(line) = output.get_mut(row * width..(row + 1) * width) else {
                    return;
                };
                let left = |at: usize, table: usize| {
                    simd.swizzle_dyn_within_blocks_u8x16(load(simd, source, at), tables[table][0])
                };
                if let Some(slot) = line.first_chunk_mut::<16>() {
                    left(at, usize::from(width == 24)).store_array(slot);
                }
                if width == 24
                    && let Some(slot) = line.last_chunk_mut::<8>()
                {
                    *slot = left(at + 15, 2).bitcast::<u64x2<S>>()[0].to_le_bytes();
                }
            }
            return;
        }
        let part =|bytes: u8x16<S>, [left, right]: [u8x16<S>; 2]| {
            let (l0, l1) = simd
                .swizzle_dyn_within_blocks_u8x16(bytes, left)
                .widening_mul(weights.0);
            let (r0, r1) = simd
                .swizzle_dyn_within_blocks_u8x16(bytes, right)
                .widening_mul(weights.1);
            (l0 + r0, l1 + r1)
        };
        let horizontal = |row: usize| -> [u16x8<S>; 3] {
            let at = origin + row * stride;
            if width == 24 {
                let (low, high) = part(load(simd, source, at), tables[1]);
                [low, high, part(load(simd, source, at + 15), tables[2]).0]
            } else {
                let (low, high) = part(load(simd, source, at), tables[0]);
                [low, high, low]
            }
        };
        let mut write = |row: usize, top: [u16x8<S>; 3], bottom: [u16x8<S>; 3]| {
            let [low, high, tail] =
                [0, 1, 2].map(|part| blend(simd, v_phase, top[part], bottom[part]));
            let Some(line) = output.get_mut(row * width..(row + 1) * width) else {
                return;
            };
            if let Some(slot) = line.first_chunk_mut::<16>() {
                simd.narrow_u16x8(low, high).store_array(slot);
            }
            if width == 24
                && let Some(slot) = line.last_chunk_mut::<8>()
            {
                *slot = simd.narrow_u16x8(tail, tail).bitcast::<u64x2<S>>()[0].to_le_bytes();
            }
        };
        let mut top = horizontal(0);
        write(0, top, top);
        for row in 1..height - 1 {
            let bottom = horizontal(row);
            write(row, top, bottom);
            top = bottom;
        }
        write(height - 1, top, top);
    }

    /// The bilinear `w x h` block, `w` a multiple of 8, of the rows of
    /// `source` from `origin`, into `output` at `stride`.
    #[inline(never)]
    pub(super) fn block<S: Simd>(
        simd: S,
        source: &[u8],
        ([origin, source_stride], [w, h]): ([usize; 2], [usize; 2]),
        [h_phase, v_phase]: [u8; 2],
        output: &mut [u16],
        stride: usize,
    ) {
        let weights = (
            u8x16::splat(simd, 16 - h_phase),
            u8x16::splat(simd, h_phase),
        );
        for column in (0..w).step_by(8) {
            let horizontal = |row: usize| {
                let bytes = load(simd, source, origin + column + row * source_stride);
                bytes.widening_mul(weights.0).0 + bytes.slide::<1>(bytes).widening_mul(weights.1).0
            };
            let mut top = horizontal(0);
            for row in 0..h {
                let bottom = horizontal(row + 1);
                let at = row * stride + column;
                if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
                    blend(simd, v_phase, top, bottom).store_array(slot);
                }
                top = bottom;
            }
        }
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn search(
            t: Neon,
            windows: [&[u8]; 2],
            geometry: [usize; 2],
            allow_center: bool,
        ) -> (i32, i32) {
            match geometry {
                [16, 12] => search_for::<16, 12>(t, windows, allow_center),
                [16, _] => search_for::<16, 20>(t, windows, allow_center),
                [_, 12] => search_for::<24, 12>(t, windows, allow_center),
                _ => search_for::<24, 20>(t, windows, allow_center),
            }
        }
    );

    /// [`search`] of `STRIDE`-wide windows and `SAD_H`-row areas: `uabd` and
    /// `uadalp` over each 12-byte row at stride 16, or 20-byte row at stride
    /// 24 (its last four bytes from a second load), then one `uaddlv`.
    #[inline]
    #[target_feature(enable = "neon")]
    fn search_for<const STRIDE: usize, const SAD_H: usize>(
        t: Neon,
        [first, second]: [&[u8]; 2],
        allow_center: bool,
    ) -> (i32, i32) {
        use core::arch::aarch64::{
            uint8x16_t, vabdq_u8, vaddlvq_u16, vandq_u8, vdupq_n_u16, vpadalq_u8,
        };
        let keep = |lanes: core::ops::Range<usize>| -> uint8x16_t {
            u8x16::load_array(
                t,
                core::array::from_fn(|lane| if lanes.contains(&lane) { u8::MAX } else { 0 }),
            )
            .into()
        };
        let (low, high) = (keep(0..12), keep(12..16));
        let sad = |dy: i32, dx: i32| -> u32 {
            let start = |dy: i32, dx: i32| ((2 + dy) * STRIDE as i32 + 2 + dx) as usize;
            let (a, b) = (start(dy, dx), start(-dy, -dx));
            let diff = |at: usize| -> uint8x16_t {
                vabdq_u8(
                    load(t, first, a + at).into(),
                    load(t, second, b + at).into(),
                )
            };
            let mut sum = vdupq_n_u16(0);
            for row in (0..SAD_H).step_by(2) {
                let at = row * STRIDE;
                if STRIDE == 16 {
                    sum = vpadalq_u8(sum, vandq_u8(diff(at), low));
                } else {
                    sum = vpadalq_u8(sum, diff(at));
                    sum = vpadalq_u8(sum, vandq_u8(diff(at + 4), high));
                }
            }
            vaddlvq_u16(sum)
        };
        super::best_offset(
            || sad(0, 0),
            || super::REFINEMV_NEIGHBORS.map(|(dy, dx)| sad(dy, dx)),
            (2 * (STRIDE - 4) * SAD_H) as u32,
            allow_center,
        )
    }
}
