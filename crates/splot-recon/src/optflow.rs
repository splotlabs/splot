// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use fearless_simd::{i16x8, i16x16, i32x8, mask16x8, prelude::*, u16x8};

use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2_signed_i32;
use crate::{BitDepth, ReconError, Result};

const GRADIENT_UNIT: usize = 16;
const MAX_LS_BITS: u32 = 26;
const MV_REFINE_PREC_BITS: i32 = 4;
const MV_DELTA_LIMIT: i32 = 1 << MV_REFINE_PREC_BITS;

/// Reusable working storage for optical-flow motion-vector derivation.
#[derive(Debug, Default)]
pub struct OptflowScratch {
    samples: Vec<i16>,
    deltas: Vec<[[i32; 2]; 2]>,
}

impl OptflowScratch {
    /// Creates reusable optical-flow storage with the requested capacities.
    #[must_use]
    pub fn with_capacity(sample_capacity: usize, delta_capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(sample_capacity),
            deltas: Vec::with_capacity(delta_capacity),
        }
    }
}

/// Derives the AV2 § 7.13.3.9 optical-flow motion-vector deltas for each
/// `unit_size` square in a pair of clipped, row-major luma predictors.
///
/// Each returned entry contains `[[row0, col0], [row1, col1]]` in 1/16-pel
/// units. Entries are ordered left-to-right, top-to-bottom. `distances` are the
/// unscaled relative order-hint distances; this function performs the § 7.13.3.9
/// ratio reduction before solving the least-squares system.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either predictor does not
/// contain exactly `width * height` values, [`ReconError::ZeroDimension`] for
/// an empty predictor, or [`ReconError::InvalidOptflowUnitSize`] unless the
/// unit size is 4 or 8 and divides both predictor dimensions.
pub fn derive_optflow_mv_deltas(
    pred0: &[u16],
    pred1: &[u16],
    width: usize,
    height: usize,
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<Vec<[[i32; 2]; 2]>> {
    let mut scratch = OptflowScratch::default();
    derive_optflow_mv_deltas_into(
        pred0,
        pred1,
        width,
        height,
        unit_size,
        bit_depth,
        distances,
        &mut scratch,
    )?;
    Ok(core::mem::take(&mut scratch.deltas))
}

/// Derives AV2 § 7.13.3.9 optical-flow deltas into reusable buffers.
///
/// `scratch` retains its allocations between calls. The returned slice contains
/// exactly one entry per `unit_size` square. Its internal output is cleared
/// before validation and remains empty on error.
///
/// # Errors
///
/// Returns the same errors as [`derive_optflow_mv_deltas`].
#[allow(clippy::too_many_arguments)]
pub fn derive_optflow_mv_deltas_into<'a>(
    pred0: &[u16],
    pred1: &[u16],
    width: usize,
    height: usize,
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &'a mut OptflowScratch,
) -> Result<&'a [[[i32; 2]; 2]]> {
    scratch.deltas.clear();
    if width == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor width",
        });
    }
    if height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor height",
        });
    }
    let expected = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "optical-flow predictor area",
        })?;
    for predictor in [pred0, pred1] {
        if predictor.len() != expected {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: predictor.len(),
            });
        }
    }
    if !matches!(unit_size, 4 | 8)
        || !width.is_multiple_of(unit_size)
        || !height.is_multiple_of(unit_size)
    {
        return Err(ReconError::InvalidOptflowUnitSize {
            unit_size,
            width,
            height,
        });
    }

    let unit_count = (width / unit_size) * (height / unit_size);
    if distances.contains(&0) {
        scratch.deltas.resize(unit_count, [[0; 2]; 2]);
        return Ok(&scratch.deltas);
    }
    let distances = reduce_distances(distances);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let scratch_len = expected
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "optical-flow scratch sample count",
        })?;
    if scratch.samples.len() < scratch_len {
        scratch.samples.resize(scratch_len, 0);
    }
    let (weighted, scratch_samples) = scratch.samples[..scratch_len].split_at_mut(expected);
    let (difference, scratch_samples) = scratch_samples.split_at_mut(expected);
    let (gradient_x, gradient_y) = scratch_samples.split_at_mut(expected);
    let max_sample = bit_depth.max_sample();
    let peak = baseline_simd!(simd => weighted_difference_planes(
        simd, pred0, pred1, weighted, difference, distances, downshift,
    ));
    if peak > max_sample {
        for (sample_index, (&left, &right)) in pred0.iter().zip(pred1).enumerate() {
            for (predictor, value) in [(0, left), (1, right)] {
                if value > max_sample {
                    return Err(ReconError::OptflowPredictorSampleOutOfRange {
                        predictor,
                        sample_index,
                        value,
                        max: max_sample,
                    });
                }
            }
        }
    }

    if let (Ok(weighted), Ok(difference)) = (
        <&[i16; 64]>::try_from(&*weighted),
        <&[i16; 64]>::try_from(&*difference),
    ) && width == 8
    {
        if unit_size == 8 {
            let delta = solve_sums(unit_8x8_sums(weighted, difference), distances)?;
            scratch.deltas.push(delta);
            return Ok(&scratch.deltas);
        }
        push_unit_deltas(
            &mut scratch.deltas,
            unit_4x4_sums(weighted, difference),
            distances,
        )?;
        return Ok(&scratch.deltas);
    }
    if let (8, Ok(weighted), Ok(difference)) = (
        unit_size,
        <&[i16; 256]>::try_from(&*weighted),
        <&[i16; 256]>::try_from(&*difference),
    ) && width == 16
    {
        push_unit_deltas(
            &mut scratch.deltas,
            unit_16x16_sums(weighted, difference),
            distances,
        )?;
        return Ok(&scratch.deltas);
    }
    gradients(weighted, width, height, gradient_x, gradient_y);
    scratch.deltas.reserve(unit_count);
    for unit_y in (0..height).step_by(unit_size) {
        for unit_x in (0..width).step_by(unit_size) {
            let delta = match solve_unit(
                gradient_x, gradient_y, difference, width, unit_x, unit_y, unit_size, distances,
            ) {
                Ok(delta) => delta,
                Err(error) => {
                    scratch.deltas.clear();
                    return Err(error);
                }
            };
            scratch.deltas.push(delta);
        }
    }
    Ok(&scratch.deltas)
}

/// Solves four units' sums into `deltas`, which an error leaves empty.
fn push_unit_deltas(
    deltas: &mut Vec<[[i32; 2]; 2]>,
    units: [[i32; 5]; 4],
    distances: [i32; 2],
) -> Result<()> {
    for sums in units {
        match solve_sums(sums, distances) {
            Ok(delta) => deltas.push(delta),
            Err(error) => {
                deltas.clear();
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Fills the § 7.13.3.9 `weighted` and `difference` planes of two row-major
/// predictors and returns their largest sample for the caller's range check.
#[inline]
fn weighted_difference_planes<S: Simd>(
    simd: S,
    pred0: &[u16],
    pred1: &[u16],
    weighted: &mut [i16],
    difference: &mut [i16],
    distances: [i32; 2],
    downshift: u32,
) -> u16 {
    let (pred0_chunks, pred0_remainder) = pred0.as_chunks::<8>();
    let (pred1_chunks, pred1_remainder) = pred1.as_chunks::<8>();
    let (weighted_chunks, weighted_remainder) = weighted.as_chunks_mut::<8>();
    let (difference_chunks, difference_remainder) = difference.as_chunks_mut::<8>();
    debug_assert!(pred0_remainder.is_empty());
    debug_assert!(pred1_remainder.is_empty());
    debug_assert!(weighted_remainder.is_empty());
    debug_assert!(difference_remainder.is_empty());
    let mut peak = u16x8::splat(simd, 0);
    for (((left, right), weighted), difference) in pred0_chunks
        .iter()
        .zip(pred1_chunks)
        .zip(weighted_chunks)
        .zip(difference_chunks)
    {
        let left = u16x8::load_array_ref(simd, left);
        let right = u16x8::load_array_ref(simd, right);
        peak = peak.max(left).max(right);
        let rows = weighted_difference_row(left, right, distances, downshift);
        rows.0.store_array(weighted);
        rows.1.store_array(difference);
    }
    peak.reduce_max()
}

/// Derives one 8x8 optical-flow delta from strided predictors.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either predictor does not
/// contain the requested 8x8 region, or the same arithmetic errors as
/// [`derive_optflow_mv_deltas`].
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn derive_optflow_mv_delta_8x8_strided_into(
    pred0: &[u16],
    start0: usize,
    pred1: &[u16],
    start1: usize,
    stride: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    if stride < 8 {
        return Err(ReconError::BufferLengthMismatch {
            expected: 8,
            actual: stride,
        });
    }
    let span_len = stride
        .checked_mul(7)
        .and_then(|offset| offset.checked_add(8))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided optical-flow predictor span",
        })?;
    let pred0 = pred0.get(start0..start0.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start0.saturating_add(span_len),
            actual: pred0.len(),
        },
    )?;
    let pred1 = pred1.get(start1..start1.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start1.saturating_add(span_len),
            actual: pred1.len(),
        },
    )?;
    if distances.contains(&0) {
        return Ok([[0; 2]; 2]);
    }

    let distances = reduce_distances(distances);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let max_sample = bit_depth.max_sample();
    baseline_simd!(simd => strided_8x8_delta(
        simd, pred0, pred1, stride, distances, downshift, max_sample,
    ))
}

/// [`derive_optflow_mv_delta_8x8_strided_into`] over its validated spans.
#[inline]
fn strided_8x8_delta<S: Simd>(
    simd: S,
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    distances: [i32; 2],
    downshift: u32,
    max_sample: u16,
) -> Result<[[i32; 2]; 2]> {
    let mut peak = u16x8::splat(simd, 0);
    let mut weighted = [i16x8::splat(simd, 0); 8];
    let mut difference = weighted;
    for row in 0..8 {
        let source = row * stride;
        let (Some(left), Some(right)) = (
            pred0.get(source..).and_then(<[u16]>::first_chunk),
            pred1.get(source..).and_then(<[u16]>::first_chunk),
        ) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: source + 8,
                actual: pred0.len().min(pred1.len()),
            });
        };
        let left = u16x8::load_array_ref(simd, left);
        let right = u16x8::load_array_ref(simd, right);
        peak = peak.max(left).max(right);
        (weighted[row], difference[row]) =
            weighted_difference_row(left, right, distances, downshift);
    }
    if peak.reduce_max() > max_sample {
        return Err(strided_sample_out_of_range(
            pred0, pred1, stride, max_sample,
        ));
    }
    let sums = whole_8x8(rows_8x8_sums(simd, &weighted, &difference));
    solve_sums(sums, distances)
}

/// The error for the first strided 8x8 predictor sample above `max_sample`,
/// in logical sample order.
#[cold]
#[inline(never)]
fn strided_sample_out_of_range(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    max_sample: u16,
) -> ReconError {
    for row in 0..8 {
        for lane in 0..8 {
            let source = row * stride + lane;
            for (predictor, value) in [(0, pred0[source]), (1, pred1[source])] {
                if value > max_sample {
                    return ReconError::OptflowPredictorSampleOutOfRange {
                        predictor,
                        sample_index: row * 8 + lane,
                        value,
                        max: max_sample,
                    };
                }
            }
        }
    }
    ReconError::ArithmeticOverflow {
        context: "strided optical-flow predictor range",
    }
}

/// The § 7.13.3.8 refinement of an eight-bit 8x8 optical-flow unit whose two
/// predictions are the eight-sample rows of `preds` from `starts`, 16 bytes
/// apart: `None` when their SAD is below `sad_threshold`, else the
/// § 7.13.3.12 delta of [`derive_optflow_mv_delta_8x8_strided_into`].
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either prediction does
/// not hold its 8x8 rows, or the arithmetic errors of the solve.
#[doc(hidden)]
pub fn optflow_unit_8x8_u8(
    preds: [&[u8]; 2],
    starts: [usize; 2],
    distances: [i32; 2],
    sad_threshold: Option<u32>,
) -> Result<Option<[[i32; 2]; 2]>> {
    const SPAN: usize = 7 * 16 + 8;
    let mut rows = [[0u64; 8]; 2];
    for ((rows, pred), start) in rows.iter_mut().zip(preds).zip(starts) {
        let span = pred
            .get(start..)
            .and_then(<[u8]>::first_chunk::<SPAN>)
            .ok_or(ReconError::BufferLengthMismatch {
                expected: start.saturating_add(SPAN),
                actual: pred.len(),
            })?;
        for (index, row) in rows.iter_mut().enumerate() {
            if let Some(bytes) = span[16 * index..].first_chunk::<8>() {
                *row = u64::from_le_bytes(*bytes);
            }
        }
    }
    let threshold = sad_threshold.unwrap_or(0);
    let reduced = if distances.contains(&0) {
        [0; 2]
    } else {
        reduce_distances(distances)
    };
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = fearless_simd::Level::baseline().as_avx2() {
        let Some(sums) = avx2::unit_8x8(t, &rows, reduced, threshold) else {
            return Ok(None);
        };
        if reduced == [0; 2] {
            return Ok(Some([[0; 2]; 2]));
        }
        return solve_sums(unit_sums(sums, 64), reduced).map(Some);
    }
    let samples = rows.map(|rows| {
        let mut samples = [0u16; 64];
        for (lanes, row) in samples.chunks_exact_mut(8).zip(rows) {
            for (lane, byte) in lanes.iter_mut().zip(row.to_le_bytes()) {
                *lane = u16::from(byte);
            }
        }
        samples
    });
    let sad = samples[0]
        .iter()
        .zip(&samples[1])
        .map(|(&a, &b)| u32::from(a.abs_diff(b)))
        .sum::<u32>();
    if sad < threshold {
        return Ok(None);
    }
    derive_optflow_mv_delta_8x8_strided_into(
        &samples[0],
        0,
        &samples[1],
        0,
        8,
        BitDepth::Eight,
        distances,
    )
    .map(Some)
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, u8x32};

    /// `pshufb` bytes picking, for every `i16` lane of an eight-lane row,
    /// the lane `OFFSET` away clamped to the row.
    const fn neighbours<const OFFSET: i32>() -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut lane = 0;
        while lane < 16 {
            let mut source = (lane % 8) as i32 + OFFSET;
            if source < 0 {
                source = 0;
            }
            if source > 7 {
                source = 7;
            }
            out[2 * lane] = 2 * source as u8;
            out[2 * lane + 1] = 2 * source as u8 + 1;
            lane += 1;
        }
        out
    }

    const NEXT: [u8; 32] = neighbours::<1>();
    const PREV: [u8; 32] = neighbours::<-1>();
    const NEXT2: [u8; 32] = neighbours::<2>();
    const PREV2: [u8; 32] = neighbours::<-2>();

    #[inline]
    #[target_feature(enable = "avx2")]
    fn bytes(t: Avx2, bytes: [u8; 32]) -> __m256i {
        u8x32::load_array(t, bytes).into()
    }

    /// `Round2Signed(42 * (next - prev) - 5 * (next2 - prev2), 7)` of `i16`
    /// lanes, the edge lanes doubled first: the magnitude rounds unsigned as
    /// `(m + add) * mul >> 16`, with `(add, mul)` `(64, 512)` for `>> 7` and
    /// `(32, 1024)` for the doubled `>> 6`, and takes its sign back after.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn gradient(taps: [__m256i; 4], add: __m256i, mul: __m256i) -> __m256i {
        let [next, prev, next2, prev2] = taps;
        let near = _mm256_mullo_epi16(_mm256_sub_epi16(next, prev), _mm256_set1_epi16(42));
        let far = _mm256_mullo_epi16(_mm256_sub_epi16(next2, prev2), _mm256_set1_epi16(5));
        let magnitude = _mm256_sub_epi16(_mm256_max_epi16(near, far), _mm256_min_epi16(near, far));
        let rounded = _mm256_mulhi_epu16(_mm256_add_epi16(magnitude, add), mul);
        let sign = _mm256_cmpgt_epi16(far, near);
        _mm256_sub_epi16(_mm256_xor_si256(rounded, sign), sign)
    }

    fearless_simd::kernel!(
        /// The 8x8 SAD test and raw `[su2, sv2, suv, suw, svw]` sums of two
        /// predictions held as eight-byte rows, row pairs `(2j, 2j + 1)` in
        /// the two lanes of one `i16` vector.
        #[inline]
        pub(super) fn unit_8x8(
            t: Avx2,
            rows: &[[u64; 8]; 2],
            distances: [i32; 2],
            sad_threshold: u32,
        ) -> Option<[i32; 5]> {
            let pairs = |rows: &[u64; 8]| -> [__m128i; 4] {
                core::array::from_fn(|j| _mm_set_epi64x(rows[2 * j + 1] as i64, rows[2 * j] as i64))
            };
            let (first, second) = (pairs(&rows[0]), pairs(&rows[1]));
            let mut sad = _mm_setzero_si128();
            for (&a, &b) in first.iter().zip(&second) {
                sad = _mm_add_epi64(sad, _mm_sad_epu8(a, b));
            }
            if ((_mm_cvtsi128_si64(sad) + _mm_extract_epi64::<1>(sad)) as u32) < sad_threshold {
                return None;
            }
            let [near, far] = distances.map(|distance| _mm256_set1_epi16(distance as i16));
            let mut weighted = [_mm256_setzero_si256(); 4];
            let mut difference = weighted;
            for j in 0..4 {
                let (a, b) = (
                    _mm256_cvtepu8_epi16(first[j]),
                    _mm256_cvtepu8_epi16(second[j]),
                );
                weighted[j] =
                    _mm256_sub_epi16(_mm256_mullo_epi16(a, near), _mm256_mullo_epi16(b, far));
                difference[j] = _mm256_sub_epi16(a, b);
            }
            let edge = |lanes: __m256i, edge: i16, inner: i16| {
                _mm256_blendv_epi8(_mm256_set1_epi16(inner), _mm256_set1_epi16(edge), lanes)
            };
            let row_edges = _mm256_setr_epi16(-1, 0, 0, 0, 0, 0, 0, -1, -1, 0, 0, 0, 0, 0, 0, -1);
            let (h_add, h_mul) = (edge(row_edges, 32, 64), edge(row_edges, 1024, 512));
            let top = _mm256_permute2x128_si256::<0x00>(weighted[0], weighted[0]);
            let bottom = _mm256_permute2x128_si256::<0x11>(weighted[3], weighted[3]);
            let middle: [__m256i; 3] = core::array::from_fn(|j| {
                _mm256_permute2x128_si256::<0x21>(weighted[j], weighted[j + 1])
            });
            let (inner_add, inner_mul) = (_mm256_set1_epi16(64), _mm256_set1_epi16(512));
            let first_rows =
                _mm256_setr_epi16(-1, -1, -1, -1, -1, -1, -1, -1, 0, 0, 0, 0, 0, 0, 0, 0);
            let last_rows =
                _mm256_setr_epi16(0, 0, 0, 0, 0, 0, 0, 0, -1, -1, -1, -1, -1, -1, -1, -1);
            let mut sums = [_mm256_setzero_si256(); 5];
            for j in 0..4 {
                let row = weighted[j];
                let shuffle = |pattern: [u8; 32]| _mm256_shuffle_epi8(row, bytes(t, pattern));
                let u = gradient(
                    [shuffle(NEXT), shuffle(PREV), shuffle(NEXT2), shuffle(PREV2)],
                    h_add,
                    h_mul,
                );
                let taps = [
                    if j < 3 { middle[j] } else { bottom },
                    if j > 0 { middle[j - 1] } else { top },
                    if j < 3 { weighted[j + 1] } else { bottom },
                    if j > 0 { weighted[j - 1] } else { top },
                ];
                let v = match j {
                    0 => gradient(taps, edge(first_rows, 32, 64), edge(first_rows, 1024, 512)),
                    3 => gradient(taps, edge(last_rows, 32, 64), edge(last_rows, 1024, 512)),
                    _ => gradient(taps, inner_add, inner_mul),
                };
                let w = difference[j];
                sums[0] = _mm256_add_epi32(sums[0], _mm256_madd_epi16(u, u));
                sums[1] = _mm256_add_epi32(sums[1], _mm256_madd_epi16(v, v));
                sums[2] = _mm256_add_epi32(sums[2], _mm256_madd_epi16(u, v));
                sums[3] = _mm256_add_epi32(sums[3], _mm256_madd_epi16(u, w));
                sums[4] = _mm256_add_epi32(sums[4], _mm256_madd_epi16(v, w));
            }
            let quad = _mm256_hadd_epi32(
                _mm256_hadd_epi32(sums[0], sums[1]),
                _mm256_hadd_epi32(sums[2], sums[3]),
            );
            let quad = _mm_add_epi32(
                _mm256_castsi256_si128(quad),
                _mm256_extracti128_si256::<1>(quad),
            );
            let last = _mm_add_epi32(
                _mm256_castsi256_si128(sums[4]),
                _mm256_extracti128_si256::<1>(sums[4]),
            );
            let last = _mm_hadd_epi32(last, last);
            let last = _mm_hadd_epi32(last, last);
            Some([
                _mm_cvtsi128_si32(quad),
                _mm_extract_epi32::<1>(quad),
                _mm_extract_epi32::<2>(quad),
                _mm_extract_epi32::<3>(quad),
                _mm_cvtsi128_si32(last),
            ])
        }
    );
}

/// The § 7.13.3.9 `weighted` and `difference` rows of eight predictor sample
/// pairs, in `i16` lanes.
///
/// The reduced distances are at most 2 in magnitude, so validated samples keep
/// `weighted` within `±4092` and `difference` within `±1023` before the
/// `Round2Signed`. Rows of samples past the bit depth are discarded by the
/// caller's range check, so their wrapped values never reach a solve.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn weighted_difference_row<S: Simd>(
    left: u16x8<S>,
    right: u16x8<S>,
    distances: [i32; 2],
    downshift: u32,
) -> (i16x8<S>, i16x8<S>) {
    let (left, right): (i16x8<S>, i16x8<S>) = (left.bitcast(), right.bitcast());
    let round = |value: i16x8<S>| {
        if downshift == 0 {
            return value;
        }
        (value + (1 << (downshift - 1)) + (value >> 15)) >> downshift
    };
    let [near, far] = distances.map(|distance| distance as i16);
    (round(left * near - right * far), round(left - right))
}

/// `Round2Signed` lane-wise: a negative value rounds its half away from zero
/// by adding one less before the arithmetic shift. The callers' sums stay far
/// below the `i32` limits, so the addition cannot wrap.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn round2_signed_simd<S: Simd, W: SimdInt<S, Element = i32>>(value: W, shift: u32) -> W {
    if shift == 0 {
        return value;
    }
    (value + (1 << (shift - 1)) + (value >> 31)) >> shift
}

/// `Round2Signed(42 * (next - prev) - 5 * (next2 - prev2), 7)` of
/// `[next, prev, next2, prev2]` in `i32` lanes, with the taps of the lanes
/// whose `double` shift is 1 doubled before rounding.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_lanes<S: Simd, W: SimdInt<S, Element = i32>>(
    [next, prev, next2, prev2]: [W; 4],
    double: W,
) -> W {
    let value = (next - prev) * 42 - (next2 - prev2) * 5;
    round2_signed_simd(value << double, 7)
}

/// `[next, prev, next2, prev2]` neighbours of every lane of a row of
/// `LAST + 1` samples, repeating the row's end samples past its edges: lane
/// `LAST` of `first` holds the first sample and lane 0 of `last` the last.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn clamped_neighbours<S: Simd, V: SimdBase<S>, const LAST: usize>(
    samples: V,
    first: V,
    last: V,
) -> [V; 4] {
    debug_assert_eq!(LAST + 1, V::LEN);
    let next = samples.slide::<1>(last);
    let prev = first.slide::<LAST>(samples);
    [next, prev, next.slide::<1>(last), first.slide::<LAST>(prev)]
}

/// [`gradient_lanes`] of eight `i16` lanes in one `i32x8`.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_lanes_8<S: Simd>(taps: [i16x8<S>; 4], double: bool) -> i16x8<S> {
    let taps = taps.map(widen_8);
    narrow_8(gradient_lanes(
        taps,
        i32x8::splat(taps[0].simd, i32::from(double)),
    ))
}

/// Eight `i16` lanes sign-extended into one `i32x8`.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn widen_8<S: Simd>(lanes: i16x8<S>) -> i32x8<S> {
    let (low, high) = lanes.widen();
    low.combine(high)
}

/// One `i32x8` truncated back to eight `i16` lanes.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn narrow_8<S: Simd>(lanes: i32x8<S>) -> i16x8<S> {
    let (low, high) = lanes.split();
    low.narrow(high)
}

/// [`gradient_lanes`] of sixteen `i16` lanes in two `i32x8`s.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_lanes_16<S: Simd>(taps: [i16x16<S>; 4], double: bool) -> i16x16<S> {
    let taps = taps.map(SimdWiden::widen);
    let double = i32x8::splat(taps[0].0.simd, i32::from(double));
    gradient_lanes(taps.map(|taps| taps.0), double)
        .narrow(gradient_lanes(taps.map(|taps| taps.1), double))
}

fn reduce_distances(distances: [i32; 2]) -> [i32; 2] {
    let magnitudes = match distances[0]
        .unsigned_abs()
        .cmp(&distances[1].unsigned_abs())
    {
        core::cmp::Ordering::Equal => [1, 1],
        core::cmp::Ordering::Greater => [2, 1],
        core::cmp::Ordering::Less => [1, 2],
    };
    [
        if distances[0] < 0 {
            -magnitudes[0]
        } else {
            magnitudes[0]
        },
        if distances[1] < 0 {
            -magnitudes[1]
        } else {
            magnitudes[1]
        },
    ]
}

fn gradients(
    values: &[i16],
    width: usize,
    height: usize,
    horizontal: &mut [i16],
    vertical: &mut [i16],
) {
    baseline_simd!(simd => gradient_planes(
        simd, values, width, height, horizontal, vertical,
    ));
}

#[inline]
fn gradient_planes<S: Simd>(
    simd: S,
    values: &[i16],
    width: usize,
    height: usize,
    horizontal: &mut [i16],
    vertical: &mut [i16],
) {
    for (source, output) in values
        .chunks_exact(width)
        .zip(horizontal.chunks_exact_mut(width))
    {
        let (source_units, source_remainder) = source.as_chunks::<GRADIENT_UNIT>();
        let (output_units, output_remainder) = output.as_chunks_mut::<GRADIENT_UNIT>();
        for (source, output) in source_units.iter().zip(output_units) {
            horizontal_gradient_16(simd, source).store_array(output);
        }
        if let (Ok(source), Ok(output)) = (
            <&[i16; 8]>::try_from(source_remainder),
            <&mut [i16; 8]>::try_from(&mut *output_remainder),
        ) {
            horizontal_gradient_8(simd, source).store_array(output);
        } else {
            for (col, output) in output_remainder.iter_mut().enumerate() {
                *output = horizontal_gradient_scalar(source_remainder, col);
            }
        }
    }
    for row in 0..height {
        let row_start = (row / GRADIENT_UNIT) * GRADIENT_UNIT;
        let row_end = (row_start + GRADIENT_UNIT).min(height) - 1;
        let row_prev = row.saturating_sub(1).max(row_start);
        let row_prev2 = row.saturating_sub(2).max(row_start);
        let row_next = (row + 1).min(row_end);
        let row_next2 = (row + 2).min(row_end);
        let double = row + 1 > row_end || row < row_start + 1;
        let row_slice = |row| &values[row * width..][..width];
        let output = &mut vertical[row * width..][..width];
        let rows = [
            row_slice(row_next),
            row_slice(row_prev),
            row_slice(row_next2),
            row_slice(row_prev2),
        ];
        let wide = vertical_gradient_chunks(simd, rows, output, |taps: [i16x16<S>; 4]| {
            gradient_lanes_16(taps, double)
        });
        let narrow = wide
            + vertical_gradient_chunks(
                simd,
                rows.map(|row| &row[wide..]),
                &mut output[wide..],
                |taps: [i16x8<S>; 4]| gradient_lanes_8(taps, double),
            );
        for col in narrow..width {
            let [next, prev, next2, prev2] = rows.map(|row| i32::from(row[col]));
            let mut value = 42 * (next - prev) - 5 * (next2 - prev2);
            if double {
                value *= 2;
            }
            output[col] = round2_signed_i32(value, 7) as i16;
        }
    }
}

/// Horizontal § 7.13.3.9 gradients of one whole gradient unit, with the
/// clamped edge taps and doubled edge columns of [`horizontal_gradient_scalar`]
/// applied lane-wise.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn horizontal_gradient_16<S: Simd>(simd: S, samples: &[i16; 16]) -> i16x16<S> {
    let [first, last] = edge_lanes(simd, samples);
    let (low, high) = i16x16::load_array_ref(simd, samples).widen();
    let low_taps = [
        low.slide::<1>(high),
        first.slide::<7>(low),
        low.slide::<2>(high),
        first.slide::<6>(low),
    ];
    let high_taps = [
        high.slide::<1>(last),
        low.slide::<7>(high),
        high.slide::<2>(last),
        low.slide::<6>(high),
    ];
    let edge = |lanes| i32x8::load_array(simd, lanes);
    gradient_lanes(low_taps, edge([1, 0, 0, 0, 0, 0, 0, 0]))
        .narrow(gradient_lanes(high_taps, edge([0, 0, 0, 0, 0, 0, 0, 1])))
}

/// [`horizontal_gradient_16`] of an eight-sample gradient unit.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn horizontal_gradient_8<S: Simd>(simd: S, samples: &[i16; 8]) -> i16x8<S> {
    let [first, last] = edge_lanes(simd, samples);
    let samples = widen_8(i16x8::load_array_ref(simd, samples));
    let taps = clamped_neighbours::<S, _, 7>(samples, first, last);
    let edges = i32x8::load_array(simd, [1, 0, 0, 0, 0, 0, 0, 1]);
    narrow_8(gradient_lanes(taps, edges))
}

/// The first and last samples of a gradient unit, each splatted to `i32` lanes.
fn edge_lanes<S: Simd, const N: usize>(simd: S, samples: &[i16; N]) -> [i32x8<S>; 2] {
    [samples.first(), samples.last()]
        .map(|sample| i32x8::splat(simd, sample.copied().map_or(0, i32::from)))
}

fn horizontal_gradient_scalar(source: &[i16], col: usize) -> i16 {
    let col_end = source.len() - 1;
    let col_prev = col.saturating_sub(1);
    let col_prev2 = col.saturating_sub(2);
    let col_next = (col + 1).min(col_end);
    let col_next2 = (col + 2).min(col_end);
    let mut value = 42 * (i32::from(source[col_next]) - i32::from(source[col_prev]))
        - 5 * (i32::from(source[col_next2]) - i32::from(source[col_prev2]));
    if col + 1 > col_end || col < 1 {
        value *= 2;
    }
    round2_signed_i32(value, 7) as i16
}

/// Vertical § 7.13.3.9 gradients of `[next, prev, next2, prev2]` rows in
/// `N`-lane groups, returning how many leading columns it wrote.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn vertical_gradient_chunks<S: Simd, V: SimdBase<S, Array = [i16; N]>, const N: usize>(
    simd: S,
    rows: [&[i16]; 4],
    output: &mut [i16],
    lanes: impl Fn([V; 4]) -> V,
) -> usize {
    let [next, prev, next2, prev2] = rows.map(|row| row.as_chunks::<N>().0);
    let (output, _) = output.as_chunks_mut::<N>();
    for ((((next, prev), next2), prev2), output) in next
        .iter()
        .zip(prev)
        .zip(next2)
        .zip(prev2)
        .zip(&mut *output)
    {
        lanes([next, prev, next2, prev2].map(|row| V::load_array_ref(simd, row)))
            .store_array(output);
    }
    output.len() * N
}

#[allow(clippy::too_many_arguments)]
fn solve_unit(
    gradient_x: &[i16],
    gradient_y: &[i16],
    difference: &[i16],
    stride: usize,
    unit_x: usize,
    unit_y: usize,
    unit_size: usize,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    let sums = match unit_size {
        4 => solve_unit_sums::<4>(gradient_x, gradient_y, difference, stride, unit_x, unit_y),
        8 => solve_unit_sums::<8>(gradient_x, gradient_y, difference, stride, unit_x, unit_y),
        _ => {
            return Err(ReconError::InvalidOptflowUnitSize {
                unit_size,
                width: stride,
                height: gradient_x.len() / stride,
            });
        }
    };
    solve_sums(sums, distances)
}

/// Solves the § 7.13.3.9 least-squares system from one unit's
/// `[su2, sv2, suv, suw, svw]` sums.
fn solve_sums(sums: [i32; 5], distances: [i32; 2]) -> Result<[[i32; 2]; 2]> {
    let [mut su2, mut sv2, mut suv, mut suw, mut svw] = sums;
    let max_product_bits = (1 + msb(su2 as u32)) + (1 + msb(sv2 as u32));
    let max_product_bits = max_product_bits
        .max((1 + msb(sv2 as u32)) + (1 + msb(suw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(su2 as u32)) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(suw.unsigned_abs())));
    let reduction = max_product_bits.saturating_sub(MAX_LS_BITS - 3) >> 1;
    su2 = round2_signed_i32(su2, reduction);
    sv2 = round2_signed_i32(sv2, reduction);
    suv = round2_signed_i32(suv, reduction);
    suw = round2_signed_i32(suw, reduction);
    svw = round2_signed_i32(svw, reduction);

    let determinant = su2 * sv2 - suv * suv;
    if determinant <= 0 {
        return Ok([[0; 2]; 2]);
    }
    let solution = divide_and_round(
        [sv2 * suw - suv * svw, su2 * svw - suv * suw],
        determinant,
        MV_REFINE_PREC_BITS - 1,
    )?;
    let col = -solution[0];
    let row = -solution[1];
    Ok([
        [
            (row * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
        [
            (row * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
    ])
}

/// The `[su2, sv2, suv, suw, svw]` sums of one `N`x`N` unit of stored gradient
/// planes, packed `16 / N` rows to a 16-lane group.
fn solve_unit_sums<const N: usize>(
    gradient_x: &[i16],
    gradient_y: &[i16],
    difference: &[i16],
    stride: usize,
    unit_x: usize,
    unit_y: usize,
) -> [i32; 5] {
    debug_assert!(stride.is_multiple_of(N));
    let row_chunks = stride / N;
    let first_chunk = unit_y * row_chunks + unit_x / N;
    let planes = [gradient_x, gradient_y, difference]
        .map(|plane| unit_rows::<N>(plane, first_chunk, row_chunks));
    baseline_simd!(simd => unit_lane_sums(simd, planes))
}

/// The `N` rows of one unit whose first row starts at `first_chunk` in a plane
/// of `row_chunks` `N`-sample chunks per row.
fn unit_rows<const N: usize>(
    plane: &[i16],
    first_chunk: usize,
    row_chunks: usize,
) -> [&[i16; N]; N] {
    let (chunks, remainder) = plane.as_chunks::<N>();
    debug_assert!(remainder.is_empty());
    core::array::from_fn(|row| &chunks[first_chunk + row * row_chunks])
}

#[inline]
fn unit_lane_sums<S: Simd, const N: usize>(simd: S, planes: [[&[i16; N]; N]; 3]) -> [i32; 5] {
    let mut sums = [i32x8::splat(simd, 0); 5];
    for first_row in (0..N).step_by(16 / N) {
        let [u, v, w] =
            planes.map(|rows| i16x16::from_fn(simd, |lane| rows[first_row + lane / N][lane % N]));
        accumulate_unit_sums(&mut sums, u, v, w);
    }
    unit_sums(sums.map(SimdBase::reduce_sum), (N * N) as i32)
}

/// [`solve_unit_sums`] of an 8x8 predictor pair, which is one gradient unit,
/// with both gradients computed in registers instead of stored planes.
///
/// `weighted` comes from validated samples: `Round2Signed(d0 * s0 - d1 * s1,
/// BitDepth - 8)` with `|d| <= 2` keeps one row's values within a span of 768
/// whatever the signs of the distances, so both `42 * Δ` and `5 * Δ` fit `i16`
/// lanes and only their difference needs the separate sign of
/// [`gradient_i16`].
fn unit_8x8_sums(weighted: &[i16; 64], difference: &[i16; 64]) -> [i32; 5] {
    baseline_simd!(simd => whole_8x8(rows_8x8_sums(
        simd,
        &plane_rows_8x8(simd, weighted),
        &plane_rows_8x8(simd, difference),
    )))
}

/// [`solve_unit_sums`] of the four 4x4 units of an 8x8 predictor pair, as
/// [`unit_8x8_sums`] derives its gradients, in the order of
/// [`derive_optflow_mv_deltas_into`].
fn unit_4x4_sums(weighted: &[i16; 64], difference: &[i16; 64]) -> [[i32; 5]; 4] {
    baseline_simd!(simd => {
        let halves = rows_8x8_sums(
            simd,
            &plane_rows_8x8(simd, weighted),
            &plane_rows_8x8(simd, difference),
        );
        let mut units = [[0; 5]; 4];
        for (half, sums) in halves.iter().enumerate() {
            for (sum, &lanes) in sums.iter().enumerate() {
                let (first, second) = lanes.split();
                let [a, b, c, d] = *(first + second);
                units[2 * half][sum] = a + b;
                units[2 * half + 1][sum] = c + d;
            }
        }
        units.map(|sums| unit_sums(sums, 16))
    })
}

fn plane_rows_8x8<S: Simd>(simd: S, plane: &[i16; 64]) -> [i16x8<S>; 8] {
    let (rows, _) = plane.as_chunks::<8>();
    core::array::from_fn(|row| i16x8::load_array_ref(simd, &rows[row]))
}

/// The lane sums of the top and bottom halves of an 8x8 unit held in lanes,
/// one row per call so that every row's clamped neighbours are constants.
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn rows_8x8_sums<S: Simd>(
    simd: S,
    weighted: &[i16x8<S>; 8],
    difference: &[i16x8<S>; 8],
) -> [[i32x8<S>; 5]; 2] {
    let gradients = [
        row_8x8_gradients::<S, 0>(weighted),
        row_8x8_gradients::<S, 1>(weighted),
        row_8x8_gradients::<S, 2>(weighted),
        row_8x8_gradients::<S, 3>(weighted),
        row_8x8_gradients::<S, 4>(weighted),
        row_8x8_gradients::<S, 5>(weighted),
        row_8x8_gradients::<S, 6>(weighted),
        row_8x8_gradients::<S, 7>(weighted),
    ];
    let mut halves = [[i32x8::splat(simd, 0); 5]; 2];
    for pair in 0..4 {
        let (top, bottom) = (gradients[2 * pair], gradients[2 * pair + 1]);
        accumulate_unit_sums(
            &mut halves[pair / 2],
            top.0.combine(bottom.0),
            top.1.combine(bottom.1),
            difference[2 * pair].combine(difference[2 * pair + 1]),
        );
    }
    halves
}

/// The unit sums of a whole 8x8 unit from the lane sums of its halves.
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn whole_8x8<S: Simd>([top, bottom]: [[i32x8<S>; 5]; 2]) -> [i32; 5] {
    let sums = core::array::from_fn(|sum| (top[sum] + bottom[sum]).reduce_sum());
    unit_sums(sums, 64)
}

/// Adds one lane group's `[u², v², uv, uw, vw]` products to `sums`, adjacent
/// `i16` products summed in `i32` (`pmaddwd`). The unit sums only need every
/// lane's product once, so pairing lanes leaves them unchanged.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn accumulate_unit_sums<S: Simd>(
    sums: &mut [i32x8<S>; 5],
    u: i16x16<S>,
    v: i16x16<S>,
    w: i16x16<S>,
) {
    sums[0] += u.pairwise_dot_product(u);
    sums[1] += v.pairwise_dot_product(v);
    sums[2] += u.pairwise_dot_product(v);
    sums[3] += u.pairwise_dot_product(w);
    sums[4] += v.pairwise_dot_product(w);
}

/// The horizontal and vertical gradients of one row of an 8x8 unit; a doubled
/// edge tap rounds by 6 bits instead of doubling before 7.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn row_8x8_gradients<S: Simd, const ROW: usize>(rows: &[i16x8<S>; 8]) -> (i16x8<S>, i16x8<S>) {
    let reversed = rows[ROW].reverse();
    let [next, prev, next2, prev2] = clamped_neighbours::<S, _, 7>(rows[ROW], reversed, reversed);
    let u = gradient_i16([next, prev], [next2, prev2], |magnitude| {
        let edge = mask16x8::from_bitmask(magnitude.simd, 0b1000_0001);
        edge.select((magnitude + 32) >> 6, (magnitude + 64) >> 7)
    });
    let shift = if ROW == 0 || ROW == 7 { 6 } else { 7 };
    let v = gradient_i16(
        [rows[(ROW + 1).min(7)], rows[ROW.saturating_sub(1)]],
        [rows[(ROW + 2).min(7)], rows[ROW.saturating_sub(2)]],
        |magnitude| (magnitude + (1 << (shift - 1))) >> shift,
    );
    (u, v)
}

/// `Round2Signed(42 * (near[0] - near[1]) - 5 * (far[0] - far[1]), shift)` in
/// `i16` lanes: the magnitude of the difference fits `u16`, so `round` rounds
/// it unsigned and it takes its sign back afterwards.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_i16<S: Simd>(
    near: [i16x8<S>; 2],
    far: [i16x8<S>; 2],
    round: impl FnOnce(u16x8<S>) -> u16x8<S>,
) -> i16x8<S> {
    let near = (near[0] - near[1]) * 42;
    let far = (far[0] - far[1]) * 5;
    let magnitude: i16x8<S> = round((near.max(far) - near.min(far)).bitcast()).bitcast();
    let sign = far.simd_gt(near).to_vector();
    (magnitude ^ sign) - sign
}

/// [`solve_unit_sums`] of the four 8x8 units of a 16x16 predictor pair, which
/// is one gradient unit, in the order of [`derive_optflow_mv_deltas_into`].
fn unit_16x16_sums(weighted: &[i16; 256], difference: &[i16; 256]) -> [[i32; 5]; 4] {
    baseline_simd!(simd => unit_16x16_lane_sums(simd, weighted, difference))
}

/// [`unit_16x16_sums`] with whole 16-lane rows: `pmaddwd` keeps the left
/// unit's products in the low four `i32` lanes and the right unit's in the
/// high four, so the halves separate only in the final reduction.
#[inline]
fn unit_16x16_lane_sums<S: Simd>(
    simd: S,
    weighted: &[i16; 256],
    difference: &[i16; 256],
) -> [[i32; 5]; 4] {
    let (rows, _) = weighted.as_chunks::<16>();
    let (diff_rows, _) = difference.as_chunks::<16>();
    let half_sums = |half: core::ops::Range<usize>| {
        let mut sums = [i32x8::splat(simd, 0); 5];
        for row in half {
            let (u, v) = gradient_row(simd, rows, row);
            let w = i16x16::load_array_ref(simd, &diff_rows[row]);
            accumulate_unit_sums(&mut sums, u, v, w);
        }
        sums.map(SimdSplit::split)
    };
    let (top, bottom) = (half_sums(0..8), half_sums(8..16));
    [
        top.map(|(left, _)| left.reduce_sum()),
        top.map(|(_, right)| right.reduce_sum()),
        bottom.map(|(left, _)| left.reduce_sum()),
        bottom.map(|(_, right)| right.reduce_sum()),
    ]
    .map(|sums| unit_sums(sums, 64))
}

/// The horizontal and vertical § 7.13.3.9 gradients of `row` in one gradient
/// unit of `rows`, with its clamped and doubled edges.
///
/// `weighted` comes from validated samples, so it stays within about
/// `±1024` and every gradient fits the `i16` the planes would hold.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_row<S: Simd>(simd: S, rows: &[[i16; 16]], row: usize) -> (i16x16<S>, i16x16<S>) {
    let last = rows.len() - 1;
    let lanes = |row: usize| i16x16::load_array_ref(simd, &rows[row]);
    let vertical = gradient_lanes_16(
        [
            lanes((row + 1).min(last)),
            lanes(row.saturating_sub(1)),
            lanes((row + 2).min(last)),
            lanes(row.saturating_sub(2)),
        ],
        row == 0 || row == last,
    );
    (horizontal_gradient_16(simd, &rows[row]), vertical)
}

/// Adds the regularizing unit `area` to the `su2` and `sv2` diagonal of raw
/// `[su2, sv2, suv, suw, svw]` sums.
fn unit_sums(raw: [i32; 5], area: i32) -> [i32; 5] {
    [raw[0] + area, raw[1] + area, raw[2], raw[3], raw[4]]
}

fn divide_and_round(values: [i32; 2], denominator: i32, shift: i32) -> Result<[i32; 2]> {
    let (denominator_shift, inverse) = if denominator == 1 {
        (0i32, 1i32)
    } else {
        let (denominator_shift, inverse) = resolve_divisor_32(denominator as u32)?;
        (i32::from(denominator_shift), i32::from(inverse))
    };
    let inverse_msb = msb(inverse as u32);
    let mut result = [0i32; 2];
    for (output, value) in result.iter_mut().zip(values) {
        if value == 0 {
            continue;
        }
        let sign = value.signum();
        let mut magnitude = value.abs();
        let reduction = (msb(magnitude as u32) + inverse_msb + 4).saturating_sub(MAX_LS_BITS);
        magnitude = round2_signed_i32(magnitude, reduction);
        let increase = shift + reduction as i32 - denominator_shift;
        magnitude = if increase <= -31 {
            let reduced = round2_signed_i32(magnitude, (-increase - 30) as u32);
            round2_signed_i32(reduced * inverse, 30)
        } else if increase >= 0 {
            magnitude * inverse * (1i32 << increase)
        } else {
            round2_signed_i32(magnitude * inverse, (-increase) as u32)
        };
        *output = sign * magnitude;
    }
    Ok(result)
}

fn msb(value: u32) -> u32 {
    u32::BITS - 1 - value.max(1).leading_zeros()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The byte unit matches the `u16` strided derivation for every distance
    /// pair over textured, extreme-contrast and flat predictions, and skips
    /// the refinement exactly when the SAD is below the threshold.
    #[test]
    fn byte_8x8_units_match_the_strided_derivation() {
        let distance_pairs = [1i32, -1, 3, -4, 0]
            .into_iter()
            .flat_map(|near| [1, -1, 2, -3].map(|far| [near, far]));
        let stride = 16;
        let fills: [fn(usize) -> u8; 4] = [
            |i| ((i * 73 + i / 13 * 29 + (i * i) % 7) % 256) as u8,
            |i| if (i + i / 13) % 3 == 0 { 255 } else { 0 },
            |i| ((i * 7919) % 256) as u8,
            |_| 90,
        ];
        for distances in distance_pairs {
            for (first, second) in [(0, 1), (1, 2), (2, 0), (3, 3), (0, 3)] {
                let preds: [Vec<u8>; 2] = [
                    (0..stride * 9 + 2).map(fills[first]).collect(),
                    (0..stride * 9 + 2).map(|i| fills[second](i + 5)).collect(),
                ];
                let starts = [1, stride + 2];
                let widened = |pred: &[u8], start: usize| -> Vec<u16> {
                    (0..64)
                        .map(|i| u16::from(pred[start + i / 8 * stride + i % 8]))
                        .collect()
                };
                let (a, b) = (widened(&preds[0], starts[0]), widened(&preds[1], starts[1]));
                let sad: u32 = a
                    .iter()
                    .zip(&b)
                    .map(|(&x, &y)| u32::from(x.abs_diff(y)))
                    .sum();
                let delta = derive_optflow_mv_delta_8x8_strided_into(
                    &a,
                    0,
                    &b,
                    0,
                    8,
                    BitDepth::Eight,
                    distances,
                )
                .unwrap();
                for threshold in [None, Some(sad), Some(sad + 1)] {
                    let expected =
                        (threshold.is_none_or(|threshold| sad >= threshold)).then_some(delta);
                    let actual =
                        optflow_unit_8x8_u8([&preds[0], &preds[1]], starts, distances, threshold)
                            .unwrap();
                    assert_eq!(
                        actual, expected,
                        "{distances:?} {first}/{second} {threshold:?}"
                    );
                }
            }
        }
        assert!(optflow_unit_8x8_u8([&[0; 120], &[0; 119]], [0, 0], [1, 1], None).is_err());
    }

    /// The `i16` lane sums match the stored `i32`-rounded gradient planes for
    /// every reduced distance pair over in-range samples, including the
    /// extreme steps that bound the lane arithmetic.
    #[test]
    fn fused_8x8_sums_match_the_stored_gradient_planes() {
        let distance_pairs = [1i32, -1, 2, -2]
            .into_iter()
            .flat_map(|near| [1, -1, 2, -2].map(|far| [near, far]))
            .filter(|[near, far]| near.abs() + far.abs() <= 3);
        for distances in distance_pairs {
            for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
                let max = u32::from(bit_depth.max_sample());
                let patterns: [&dyn Fn(usize, usize) -> u32; 4] = [
                    &|i, seed| ((i * 7919 * seed + 31) % (max as usize + 1)) as u32,
                    &|i, seed| if (i + seed) % 3 == 0 { max } else { 0 },
                    &|i, seed| {
                        if (i / 8 + i % 8 + seed) % 2 == 0 {
                            max
                        } else {
                            0
                        }
                    },
                    &|i, seed| max * u32::from((i * 104_729 * seed) % 5 < 2),
                ];
                for pattern in patterns {
                    for seed in [1usize, 2, 7] {
                        let plane = |seed: usize| -> [u16; 64] {
                            core::array::from_fn(|i| pattern(i, seed) as u16)
                        };
                        let (left, right) = (plane(seed), plane(seed + 3));
                        let downshift = u32::from(bit_depth.bits() - 8);
                        let (mut weighted, mut difference) = ([0i16; 64], [0i16; 64]);
                        baseline_simd!(simd => weighted_difference_planes(
                            simd, &left, &right, &mut weighted, &mut difference, distances,
                            downshift,
                        ));
                        let (mut gradient_x, mut gradient_y) = ([0i16; 64], [0i16; 64]);
                        gradients(&weighted, 8, 8, &mut gradient_x, &mut gradient_y);
                        let want = [(0, 0), (4, 0), (0, 4), (4, 4)].map(|(x, y)| {
                            solve_unit_sums::<4>(&gradient_x, &gradient_y, &difference, 8, x, y)
                        });
                        assert_eq!(
                            unit_4x4_sums(&weighted, &difference),
                            want,
                            "{distances:?} {bit_depth:?}"
                        );
                        assert_eq!(
                            unit_8x8_sums(&weighted, &difference),
                            solve_unit_sums::<8>(&gradient_x, &gradient_y, &difference, 8, 0, 0),
                            "{distances:?} {bit_depth:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn i16_weighted_difference_rows_match_the_i32_rounding() {
        let left = [0, 1023, 1, 512, 1023, 7, 0, 333];
        let right = [1023, 0, 2, 511, 1023, 9, 0, 1000];
        for distances in [[1, 1], [2, -1], [-1, 2], [-2, -1], [1, -2]] {
            for downshift in [0, 2] {
                let (weighted, difference) = baseline_simd!(simd => {
                    let (weighted, difference) = weighted_difference_row(
                        u16x8::load_array(simd, left),
                        u16x8::load_array(simd, right),
                        distances,
                        downshift,
                    );
                    (weighted.to_array(), difference.to_array())
                });
                for lane in 0..8 {
                    let (l, r) = (i32::from(left[lane]), i32::from(right[lane]));
                    let want = |value: i32| round2_signed_i32(value, downshift) as i16;
                    assert_eq!(weighted[lane], want(distances[0] * l - distances[1] * r));
                    assert_eq!(difference[lane], want(l - r));
                }
            }
        }
    }

    #[test]
    fn fused_16x16_sums_match_the_stored_gradient_planes() {
        for seed in [1usize, 7, 9973] {
            let plane = |scale: usize| -> [i16; 256] {
                core::array::from_fn(|i| ((i * scale * seed + 31) % 2049) as i16 - 1024)
            };
            let (weighted, difference) = (plane(7919), plane(104_729));
            let (mut gradient_x, mut gradient_y) = ([0i16; 256], [0i16; 256]);
            gradients(&weighted, 16, 16, &mut gradient_x, &mut gradient_y);
            let want = [(0, 0), (8, 0), (0, 8), (8, 8)].map(|(x, y)| {
                solve_unit_sums::<8>(&gradient_x, &gradient_y, &difference, 16, x, y)
            });
            assert_eq!(unit_16x16_sums(&weighted, &difference), want);
        }
    }

    #[test]
    fn lane_horizontal_gradients_match_the_clamped_scalar_taps() {
        let row = |len: usize| {
            (0..len)
                .map(|index| ((index * 7919 + 13) % 4001) as i16 - 2000)
                .collect::<Vec<i16>>()
        };
        let source8: [i16; 8] = row(8).try_into().unwrap();
        let source16: [i16; 16] = row(16).try_into().unwrap();
        let want8: Vec<i16> = (0..8)
            .map(|col| horizontal_gradient_scalar(&source8, col))
            .collect();
        let want16: Vec<i16> = (0..16)
            .map(|col| horizontal_gradient_scalar(&source16, col))
            .collect();
        let (got8, got16) = baseline_simd!(simd => (
            horizontal_gradient_8(simd, &source8).to_array(),
            horizontal_gradient_16(simd, &source16).to_array(),
        ));
        assert_eq!(got8[..], want8[..]);
        assert_eq!(got16[..], want16[..]);
    }

    #[test]
    fn equal_predictors_produce_zero_deltas() {
        let predictor = vec![128; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&predictor, &predictor, 8, 8, 8, BitDepth::Eight, [3, -3],)
                .unwrap(),
            vec![[[0; 2]; 2]],
        );
    }

    #[test]
    fn four_by_four_units_cover_an_eight_by_eight_predictor() {
        let predictor = vec![80; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&predictor, &predictor, 8, 8, 4, BitDepth::Eight, [1, -2],)
                .unwrap()
                .len(),
            4,
        );
    }

    #[test]
    fn into_reuses_scratch_and_delta_allocations() {
        let predictor = vec![80; 8 * 8];
        let mut scratch = OptflowScratch::default();
        derive_optflow_mv_deltas_into(
            &predictor,
            &predictor,
            8,
            8,
            4,
            BitDepth::Eight,
            [1, -2],
            &mut scratch,
        )
        .unwrap();
        let samples_ptr = scratch.samples.as_ptr();
        let deltas_ptr = scratch.deltas.as_ptr();

        derive_optflow_mv_deltas_into(
            &predictor,
            &predictor,
            8,
            8,
            4,
            BitDepth::Eight,
            [1, -2],
            &mut scratch,
        )
        .unwrap();

        assert_eq!(scratch.samples.as_ptr(), samples_ptr);
        assert_eq!(scratch.deltas.as_ptr(), deltas_ptr);
        assert_eq!(scratch.deltas, vec![[[0; 2]; 2]; 4]);
    }

    #[test]
    fn into_clears_deltas_when_validation_fails() {
        let predictor = vec![80; 8 * 8];
        let mut scratch = OptflowScratch {
            samples: Vec::new(),
            deltas: vec![[[1; 2]; 2]],
        };

        assert!(matches!(
            derive_optflow_mv_deltas_into(
                &predictor[..63],
                &predictor,
                8,
                8,
                8,
                BitDepth::Eight,
                [1, -1],
                &mut scratch,
            ),
            Err(ReconError::BufferLengthMismatch {
                expected: 64,
                actual: 63,
            })
        ));
        assert!(scratch.deltas.is_empty());
    }

    #[test]
    fn linear_horizontal_difference_solves_to_opposite_fractional_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1],).unwrap(),
            vec![[[0, -14], [0, 14]]],
        );
    }

    #[test]
    fn ten_bit_normalization_matches_eight_bit_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        let left_10bit: Vec<u16> = left.iter().map(|sample| sample * 4).collect();
        let right_10bit: Vec<u16> = right.iter().map(|sample| sample * 4).collect();

        assert_eq!(
            derive_optflow_mv_deltas(&left_10bit, &right_10bit, 8, 8, 8, BitDepth::Ten, [1, -1],)
                .unwrap(),
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]).unwrap(),
        );
    }

    #[test]
    fn zero_distance_disables_refinement() {
        let left = vec![0; 8 * 8];
        let right = vec![255; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [0, -1],).unwrap(),
            vec![[[0; 2]; 2]],
        );
    }

    #[test]
    fn maximum_ten_bit_contrast_fits_i16_scratch_and_i32_solver() {
        let left: Vec<u16> = (0..8 * 8)
            .map(|index| if index & 1 == 0 { 1023 } else { 0 })
            .collect();
        let right: Vec<u16> = left.iter().map(|&value| 1023 - value).collect();

        let deltas =
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Ten, [2, -1]).unwrap();
        assert!(deltas[0].iter().flatten().all(|&value| value.abs() <= 16));
    }

    #[test]
    fn vector_sums_match_scalar_sums_across_unit_grids() {
        for (width, height, unit_size) in [(8, 8, 4), (16, 12, 4), (16, 16, 8), (24, 16, 8)] {
            let len = width * height;
            let gradient_x: Vec<i16> = (0..len).map(|index| (index % 17) as i16 - 8).collect();
            let gradient_y: Vec<i16> = (0..len).map(|index| (index % 13) as i16 - 6).collect();
            let difference: Vec<i16> = (0..len).map(|index| (index % 11) as i16 - 5).collect();
            for unit_y in (0..height).step_by(unit_size) {
                for unit_x in (0..width).step_by(unit_size) {
                    let area = (unit_size * unit_size) as i32;
                    let mut expected = [area, area, 0, 0, 0];
                    for row in unit_y..unit_y + unit_size {
                        for col in unit_x..unit_x + unit_size {
                            let index = row * width + col;
                            let u = i32::from(gradient_x[index]);
                            let v = i32::from(gradient_y[index]);
                            let w = i32::from(difference[index]);
                            expected[0] += u * u;
                            expected[1] += v * v;
                            expected[2] += u * v;
                            expected[3] += u * w;
                            expected[4] += v * w;
                        }
                    }
                    let actual = match unit_size {
                        4 => solve_unit_sums::<4>(
                            &gradient_x,
                            &gradient_y,
                            &difference,
                            width,
                            unit_x,
                            unit_y,
                        ),
                        8 => solve_unit_sums::<8>(
                            &gradient_x,
                            &gradient_y,
                            &difference,
                            width,
                            unit_x,
                            unit_y,
                        ),
                        _ => unreachable!(),
                    };
                    assert_eq!(
                        actual, expected,
                        "{width}x{height} unit at {unit_x},{unit_y}"
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_predictors_outside_active_bit_depth() {
        let mut predictor = vec![0; 8 * 8];
        predictor[7] = 256;
        let other = vec![0; 8 * 8];

        assert!(matches!(
            derive_optflow_mv_deltas(&predictor, &other, 8, 8, 8, BitDepth::Eight, [1, -1],),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 0,
                sample_index: 7,
                value: 256,
                max: 255,
            })
        ));
    }

    #[test]
    fn predictor_range_error_keeps_sample_then_predictor_order() {
        let mut left = vec![0; 8 * 8];
        let mut right = vec![0; 8 * 8];
        right[2] = 256;
        left[3] = 257;

        assert!(matches!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 1,
                sample_index: 2,
                value: 256,
                max: 255,
            })
        ));

        left[2] = 258;
        assert!(matches!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 0,
                sample_index: 2,
                value: 258,
                max: 255,
            })
        ));
    }

    #[test]
    fn strided_predictor_range_error_uses_logical_sample_order() {
        const STRIDE: usize = 12;
        let mut left = vec![0; STRIDE * 8];
        let mut right = vec![0; STRIDE * 8];
        right[STRIDE] = 256;
        left[STRIDE + 1] = 257;

        assert!(matches!(
            derive_optflow_mv_delta_8x8_strided_into(
                &left,
                0,
                &right,
                0,
                STRIDE,
                BitDepth::Eight,
                [1, -1],
            ),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 1,
                sample_index: 8,
                value: 256,
                max: 255,
            })
        ));
    }
}
