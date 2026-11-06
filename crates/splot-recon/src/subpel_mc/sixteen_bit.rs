// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled § 7.13.3.18 interpolation of 16-bit storage in AVX2 word lanes.
//!
//! The vector layout is the byte-lane one: 16 outputs per vector, one strip
//! row of a block 16 columns wide, two rows of an 8-column strip or four rows
//! of a 4-column strip. Each tap pair interleaves two windows, or two rows,
//! into one `pmaddwd`, so every sum is exact in `i32`; a horizontal value
//! fits `i16` for 8- and 10-bit samples. Other widths and targets without
//! AVX2 decline to the portable kernels.

use super::block::{Block, Sink};
use crate::format::ReconSample;

/// Predicts a 16-bit block and returns `true`, or returns `false` without
/// writing when the block is outside the word-lane subset.
pub(super) fn predict<T: ReconSample>(
    block: &Block<'_, T>,
    sink: &mut Sink<'_>,
    stage: &mut [i16],
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2()
        && let Some(source) = T::u16_slice(block.source)
        && block.w.is_multiple_of(4)
        && block.h.is_multiple_of(16 / width(block.w))
        && !(width(block.w) == 4 && block.horizontal.is_some_and(|taps| taps.count > 2))
    {
        let block = Block {
            source,
            stride: block.stride,
            origin: block.origin,
            top: block.top,
            rows: block.rows,
            w: block.w,
            h: block.h,
            horizontal: block.horizontal,
            vertical: block.vertical,
        };
        avx2::predict(token, &block, sink, stage);
        return true;
    }
    let _ = (block, sink, stage);
    false
}

/// The strip width of a block: 16 columns, else strips of 8 columns in
/// pairs of rows, else strips of 4 in quads of rows.
const fn width(w: usize) -> usize {
    if w.is_multiple_of(16) {
        16
    } else if w.is_multiple_of(8) {
        8
    } else {
        4
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::super::block::{Block, Sink, Taps};
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i16x16, i32x4, i32x8, u16x8, u16x16};

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn chunk<const N: usize, T>(samples: &[T], at: usize) -> Option<&[T; N]> {
        samples.get(at..)?.first_chunk::<N>()
    }

    /// Calls `$f::<W, P>` for the run-time strip width and pair count.
    macro_rules! specialize {
        ($width:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
            match ($width, $pairs) {
                (4, 1) => $f::<4, 1>($($arg),*),
                (4, 2) => $f::<4, 2>($($arg),*),
                (4, 3) => $f::<4, 3>($($arg),*),
                (4, _) => $f::<4, 4>($($arg),*),
                (8, 1) => $f::<8, 1>($($arg),*),
                (8, 2) => $f::<8, 2>($($arg),*),
                (8, 3) => $f::<8, 3>($($arg),*),
                (8, _) => $f::<8, 4>($($arg),*),
                (_, 1) => $f::<16, 1>($($arg),*),
                (_, 2) => $f::<16, 2>($($arg),*),
                (_, 3) => $f::<16, 3>($($arg),*),
                (_, _) => $f::<16, 4>($($arg),*),
            }
        };
    }

    /// `InterRound1`-free rounding of a class's final pass: `(bias, shift)`.
    type Rounding = (i32, i32);

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn predict(
            t: Avx2,
            block: &Block<'_, u16>,
            sink: &mut Sink<'_>,
            stage: &mut [i16],
        ) {
            let width = super::width(block.w);
            let put = sink.is_put();
            match (block.horizontal, block.vertical) {
                (None, None) => match width {
                    4 => copy::<4>(t, block, sink),
                    8 => copy::<8>(t, block, sink),
                    _ => copy::<16>(t, block, sink),
                },
                (Some(taps), None) => {
                    specialize!(width, taps.count, horizontal(t, block, taps, sink));
                }
                (None, Some(taps)) => {
                    let rounding = if put { (64, 7) } else { (4, 3) };
                    for strip in 0..block.w / width {
                        match width {
                            4 => stage_samples::<4>(
                                t,
                                block,
                                strip,
                                block.h + taps.span() - 1,
                                stage,
                            ),
                            8 => stage_samples::<8>(
                                t,
                                block,
                                strip,
                                block.h + taps.span() - 1,
                                stage,
                            ),
                            _ => stage_samples::<16>(
                                t,
                                block,
                                strip,
                                block.h + taps.span() - 1,
                                stage,
                            ),
                        }
                        specialize!(
                            width,
                            taps.count,
                            filter_stage(t, block, taps, strip, rounding, stage, sink)
                        );
                    }
                }
                (Some(h_taps), Some(v_taps)) => {
                    let rows = block.h + v_taps.span() - 1;
                    let rounding = if put { (1 << 10, 11) } else { (1 << 6, 7) };
                    for strip in 0..block.w / width {
                        specialize!(
                            width,
                            h_taps.count,
                            stage_rows(t, block, h_taps, rows, strip, stage)
                        );
                        specialize!(
                            width,
                            v_taps.count,
                            filter_stage(t, block, v_taps, strip, rounding, stage, sink)
                        );
                    }
                }
            }
        }
    );

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load8(t: Avx2, samples: &[u16; 8]) -> __m128i {
        u16x8::load_array_ref(t, samples).into()
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load4(samples: &[u16; 4]) -> i64 {
        i64::from(samples[0])
            | i64::from(samples[1]) << 16
            | i64::from(samples[2]) << 32
            | i64::from(samples[3]) << 48
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn weights<const P: usize>(taps: &Taps) -> [__m256i; 4] {
        core::array::from_fn(|pair| _mm256_set1_epi32(taps.full[pair.min(P - 1)]))
    }

    /// The 16 samples of the group whose rows start at `start`, in output
    /// order.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn group_samples<const W: usize>(
        t: Avx2,
        source: &[u16],
        starts: [usize; 4],
    ) -> Option<__m256i> {
        Some(match W {
            4 => _mm256_set_epi64x(
                load4(chunk(source, starts[3])?),
                load4(chunk(source, starts[2])?),
                load4(chunk(source, starts[1])?),
                load4(chunk(source, starts[0])?),
            ),
            8 => _mm256_set_m128i(
                load8(t, chunk(source, starts[1])?),
                load8(t, chunk(source, starts[0])?),
            ),
            _ => u16x16::load_array_ref(t, chunk(source, starts[0])?).into(),
        })
    }

    /// The horizontal pair sums of the group whose rows start at `starts`:
    /// each lane's outputs `0..4` (`low`) and `4..8` (`high`).
    #[inline]
    #[target_feature(enable = "avx2")]
    fn horizontal_sums<const W: usize, const P: usize>(
        t: Avx2,
        source: &[u16],
        starts: [usize; 4],
        weights: &[__m256i; 4],
    ) -> Option<(__m256i, __m256i)> {
        let windows: [__m256i; 8] = if W == 4 {
            let rows = |a: usize, b: usize| -> Option<__m256i> {
                Some(_mm256_set_m128i(
                    load8(t, chunk(source, starts[b])?),
                    load8(t, chunk(source, starts[a])?),
                ))
            };
            let (first, second) = (rows(0, 2)?, rows(1, 3)?);
            let window = |k: i32| -> __m256i {
                let shift = |v: __m256i| match k {
                    0 => v,
                    1 => _mm256_bsrli_epi128::<2>(v),
                    2 => _mm256_bsrli_epi128::<4>(v),
                    _ => _mm256_bsrli_epi128::<6>(v),
                };
                _mm256_unpacklo_epi64(shift(first), shift(second))
            };
            [
                window(0),
                window(1),
                window(2),
                window(3),
                window(3),
                window(3),
                window(3),
                window(3),
            ]
        } else {
            let (low, high) = if W == 8 {
                (
                    _mm256_set_m128i(
                        load8(t, chunk(source, starts[1])?),
                        load8(t, chunk(source, starts[0])?),
                    ),
                    _mm256_set_m128i(
                        load8(t, chunk(source, starts[1] + 8)?),
                        load8(t, chunk(source, starts[0] + 8)?),
                    ),
                )
            } else {
                (
                    u16x16::load_array_ref(t, chunk(source, starts[0])?).into(),
                    u16x16::load_array_ref(t, chunk(source, starts[0] + 8)?).into(),
                )
            };
            [
                low,
                _mm256_alignr_epi8::<2>(high, low),
                _mm256_alignr_epi8::<4>(high, low),
                _mm256_alignr_epi8::<6>(high, low),
                _mm256_alignr_epi8::<8>(high, low),
                _mm256_alignr_epi8::<10>(high, low),
                _mm256_alignr_epi8::<12>(high, low),
                _mm256_alignr_epi8::<14>(high, low),
            ]
        };
        let (mut low, mut high) = (_mm256_setzero_si256(), _mm256_setzero_si256());
        for (pair, &weight) in weights.iter().enumerate().take(P) {
            let (a, b) = (windows[2 * pair], windows[2 * pair + 1]);
            low = _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weight));
            high = _mm256_add_epi32(high, _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weight));
        }
        Some((low, high))
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn round((low, high): (__m256i, __m256i), (bias, shift): Rounding) -> (__m256i, __m256i) {
        let (bias, shift) = (_mm256_set1_epi32(bias), _mm_cvtsi32_si128(shift));
        (
            _mm256_sra_epi32(_mm256_add_epi32(low, bias), shift),
            _mm256_sra_epi32(_mm256_add_epi32(high, bias), shift),
        )
    }

    /// The group row starts of a group at window row `row` of strip `strip`,
    /// clamped to window row `last`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn starts<const W: usize>(
        block: &Block<'_, u16>,
        row: usize,
        strip: usize,
        last: usize,
    ) -> [usize; 4] {
        let start = |q: usize| block.start((row + q).min(last)) + strip * W;
        [start(0), start(1), start(2), start(3)]
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn copy<const W: usize>(t: Avx2, block: &Block<'_, u16>, sink: &mut Sink<'_>) {
        let shift = _mm_cvtsi32_si128(if sink.is_put() { 0 } else { 4 });
        for strip in 0..block.w / W {
            for row in (0..block.h).step_by(16 / W) {
                let starts = starts::<W>(block, row, strip, usize::MAX);
                let Some(samples) = group_samples::<W>(t, block.source, starts) else {
                    return;
                };
                let first = _mm256_sll_epi32(
                    _mm256_cvtepu16_epi32(_mm256_castsi256_si128(samples)),
                    shift,
                );
                let second = _mm256_sll_epi32(
                    _mm256_cvtepu16_epi32(_mm256_extracti128_si256::<1>(samples)),
                    shift,
                );
                emit::<W>(t, sink, block.w, row, strip, first, second);
            }
        }
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn horizontal<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u16>,
        taps: &Taps,
        sink: &mut Sink<'_>,
    ) {
        let weights = weights::<P>(taps);
        let put = sink.is_put();
        for strip in 0..block.w / W {
            for row in (0..block.h).step_by(16 / W) {
                let starts = starts::<W>(block, row, strip, usize::MAX);
                let Some(sums) = horizontal_sums::<W, P>(t, block.source, starts, &weights) else {
                    return;
                };
                let intermediate = round(sums, (4, 3));
                let (low, high) = if put {
                    round(intermediate, (8, 4))
                } else {
                    intermediate
                };
                emit_lanes::<W>(t, sink, block.w, row, strip, low, high);
            }
        }
    }

    /// `rows` window rows of `W` samples of strip `strip` into `stage`, as the
    /// vertical-only pass's lines.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn stage_samples<const W: usize>(
        t: Avx2,
        block: &Block<'_, u16>,
        strip: usize,
        rows: usize,
        stage: &mut [i16],
    ) {
        let last = rows - 1;
        for (group, slot) in stage
            .chunks_exact_mut(16)
            .take(rows.div_ceil(16 / W))
            .enumerate()
        {
            let row = group * (16 / W);
            let starts = starts::<W>(block, row, strip, last);
            let samples = group_samples::<W>(t, block.source, starts);
            let (Some(samples), Some(slot)) = (samples, slot.first_chunk_mut::<16>()) else {
                return;
            };
            i16x16::<Avx2>::simd_from(t, samples).store_array(slot);
        }
    }

    /// The horizontal pass of strip `strip`: `rows` intermediate rows,
    /// `Round2(sum, InterRound0)`, into `stage`; a ragged last group repeats
    /// the last row.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn stage_rows<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u16>,
        taps: &Taps,
        rows: usize,
        strip: usize,
        stage: &mut [i16],
    ) {
        let weights = weights::<P>(taps);
        for (group, slot) in stage
            .chunks_exact_mut(16)
            .take(rows.div_ceil(16 / W))
            .enumerate()
        {
            let starts = starts::<W>(block, group * (16 / W), strip, rows - 1);
            let Some(sums) = horizontal_sums::<W, P>(t, block.source, starts, &weights) else {
                return;
            };
            let (low, high) = round(sums, (4, 3));
            if let Some(slot) = slot.first_chunk_mut::<16>() {
                i16x16::<Avx2>::simd_from(t, _mm256_packs_epi32(low, high)).store_array(slot);
            }
        }
    }

    /// The vertical pass of strip `strip` over the staged lines.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn filter_stage<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u16>,
        taps: &Taps,
        strip: usize,
        rounding: Rounding,
        stage: &[i16],
        sink: &mut Sink<'_>,
    ) {
        let weights = weights::<P>(taps);
        let span = (2 * P - 1) * W + 16;
        for row in (0..block.h).step_by(16 / W) {
            let Some(window) = stage.get(row * W..row * W + span) else {
                return;
            };
            let line = |row: usize| -> __m256i {
                let lanes = window
                    .get(row * W..row * W + 16)
                    .and_then(<[i16]>::first_chunk::<16>);
                lanes.map_or(_mm256_setzero_si256(), |lanes| {
                    i16x16::load_array_ref(t, lanes).into()
                })
            };
            let (mut low, mut high) = (_mm256_setzero_si256(), _mm256_setzero_si256());
            for (pair, &weight) in weights.iter().enumerate().take(P) {
                let (a, b) = (line(2 * pair), line(2 * pair + 1));
                low = _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weight));
                high =
                    _mm256_add_epi32(high, _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weight));
            }
            let (low, high) = round((low, high), rounding);
            emit_lanes::<W>(t, sink, block.w, row, strip, low, high);
        }
    }

    /// [`emit`] of each lane's outputs `0..4` (`low`) and `4..8` (`high`).
    #[inline]
    #[target_feature(enable = "avx2")]
    fn emit_lanes<const W: usize>(
        t: Avx2,
        sink: &mut Sink<'_>,
        w: usize,
        row: usize,
        strip: usize,
        low: __m256i,
        high: __m256i,
    ) {
        let first = _mm256_permute2x128_si256::<0x20>(low, high);
        let second = _mm256_permute2x128_si256::<0x31>(low, high);
        emit::<W>(t, sink, w, row, strip, first, second);
    }

    /// Stores the 16 final-pass values of the group at `row`, `strip`:
    /// outputs `0..8` in `first`, `8..16` in `second`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn emit<const W: usize>(
        t: Avx2,
        sink: &mut Sink<'_>,
        w: usize,
        row: usize,
        strip: usize,
        first: __m256i,
        second: __m256i,
    ) {
        let column = strip * W;
        let clip = |first: __m256i, second: __m256i, max: i32| -> __m256i {
            let (zero, max) = (_mm256_setzero_si256(), _mm256_set1_epi32(max));
            let first = _mm256_min_epi32(_mm256_max_epi32(first, zero), max);
            let second = _mm256_min_epi32(_mm256_max_epi32(second, zero), max);
            _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packus_epi32(first, second))
        };
        let blend = |pred0: &[i32], forward: i32| -> (__m256i, __m256i) {
            let at = row * w + column;
            let zero4 = [0i32; 4];
            let zero8 = [0i32; 8];
            let load8 = |at: usize| -> __m256i {
                i32x8::load_array_ref(t, chunk::<8, i32>(pred0, at).unwrap_or(&zero8)).into()
            };
            let load4 = |at: usize| -> __m128i {
                i32x4::load_array_ref(t, chunk::<4, i32>(pred0, at).unwrap_or(&zero4)).into()
            };
            let (pred_first, pred_second) = match W {
                4 => (
                    _mm256_set_m128i(load4(at + w), load4(at)),
                    _mm256_set_m128i(load4(at + 3 * w), load4(at + 2 * w)),
                ),
                8 => (load8(at), load8(at + w)),
                _ => (load8(at), load8(at + 8)),
            };
            let mix = |pred: __m256i, value: __m256i| -> __m256i {
                if forward == 8 {
                    _mm256_srai_epi32::<5>(_mm256_add_epi32(
                        _mm256_add_epi32(pred, value),
                        _mm256_set1_epi32(16),
                    ))
                } else {
                    let weighted = _mm256_add_epi32(
                        _mm256_mullo_epi32(pred, _mm256_set1_epi32(forward)),
                        _mm256_mullo_epi32(value, _mm256_set1_epi32(16 - forward)),
                    );
                    _mm256_srai_epi32::<8>(_mm256_add_epi32(weighted, _mm256_set1_epi32(128)))
                }
            };
            (mix(pred_first, first), mix(pred_second, second))
        };
        match sink {
            Sink::Put { output, stride } => {
                store_bytes::<W>(
                    t,
                    output,
                    row * *stride + column,
                    *stride,
                    clip(first, second, 255),
                );
            }
            Sink::PutWide {
                output,
                stride,
                max,
            } => {
                store_words::<W>(
                    t,
                    output,
                    row * *stride + column,
                    *stride,
                    clip(first, second, i32::from(*max)),
                );
            }
            Sink::Prep { output, stride } => {
                let stride = *stride;
                let at = row * stride + column;
                let mut put8 = |at: usize, lanes: __m256i| {
                    if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>)
                    {
                        i32x8::<Avx2>::simd_from(t, lanes).store_array(slot);
                    }
                };
                match W {
                    4 => {
                        let mut put4 = |at: usize, lanes: __m128i| {
                            if let Some(slot) =
                                output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>)
                            {
                                i32x4::<Avx2>::simd_from(t, lanes).store_array(slot);
                            }
                        };
                        put4(at, _mm256_castsi256_si128(first));
                        put4(at + stride, _mm256_extracti128_si256::<1>(first));
                        put4(at + 2 * stride, _mm256_castsi256_si128(second));
                        put4(at + 3 * stride, _mm256_extracti128_si256::<1>(second));
                    }
                    8 => {
                        put8(at, first);
                        put8(at + stride, second);
                    }
                    _ => {
                        put8(at, first);
                        put8(at + 8, second);
                    }
                }
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (first, second) = blend(pred0, *forward);
                store_bytes::<W>(
                    t,
                    output,
                    row * *stride + column,
                    *stride,
                    clip(first, second, 255),
                );
            }
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                let (first, second) = blend(pred0, *forward);
                store_words::<W>(
                    t,
                    output,
                    row * *stride + column,
                    *stride,
                    clip(first, second, i32::from(*max)),
                );
            }
        }
    }

    /// The 16 clipped words `values` of a group into `output` from `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_words<const W: usize>(
        t: Avx2,
        output: &mut [u16],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        let lanes = u16x16::<Avx2>::simd_from(t, values).to_array();
        let mut put = |at: usize, lanes: &[u16]| {
            if let Some(slot) = output.get_mut(at..at + lanes.len()) {
                for (slot, &lane) in slot.iter_mut().zip(lanes) {
                    *slot = lane;
                }
            }
        };
        match W {
            4 => {
                for (row, lanes) in lanes.chunks_exact(4).enumerate() {
                    put(at + row * stride, lanes);
                }
            }
            8 => {
                put(at, &lanes[..8]);
                put(at + stride, &lanes[8..]);
            }
            _ => {
                if let Some(slot) = output
                    .get_mut(at..)
                    .and_then(<[u16]>::first_chunk_mut::<16>)
                {
                    *slot = lanes;
                }
            }
        }
    }

    /// The 16 clipped words `values` of a group as bytes into `output`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_bytes<const W: usize>(
        t: Avx2,
        output: &mut [u8],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        let lanes = u16x16::<Avx2>::simd_from(t, values)
            .to_array()
            .map(|lane| lane as u8);
        let mut put = |at: usize, lanes: &[u8]| {
            if let Some(slot) = output.get_mut(at..at + lanes.len()) {
                for (slot, &lane) in slot.iter_mut().zip(lanes) {
                    *slot = lane;
                }
            }
        };
        match W {
            4 => {
                for (row, lanes) in lanes.chunks_exact(4).enumerate() {
                    put(at + row * stride, lanes);
                }
            }
            8 => {
                put(at, &lanes[..8]);
                put(at + stride, &lanes[8..]);
            }
            _ => put(at, &lanes),
        }
    }
}
