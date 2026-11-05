// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.17 deblocking-filter sample math.
//!
//! This module implements the scheduler-free per-edge AV2 deblocking primitives
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)):
//! the § 7.17.7.1 sample filter ([`deblock_sample_filter`], `#s-7-17-7-1`), which
//! modifies up to `maxWidthNeg` samples on the previous (`p`) side and
//! `Max(maxWidthNeg, maxWidthPos)` samples on the current (`q`) side using the
//! `deltaM2` ramp, the `Q_Thresh_Mults` / `W_Mult` § 9.2 weights, `Round2`, and
//! the § 4.8 `Clip1` clamp; the § 7.17.3 filter-maximum-width derivation
//! ([`deblock_filter_max_width`], `#s-7-17-3`), which produces the per-side
//! widths the sample filter consumes; and the § 7.17.5 adaptive filter strength
//! ([`deblock_adaptive_filter_strength`] / [`deblock_side_threshold_index`],
//! `#s-7-17-5`), which produces the `qThr` / `side` thresholds from the filter
//! level; and the § 7.17.7.2 filter-choice process ([`deblock_filter_choice`],
//! `#s-7-17-7-2`), which chooses the filter width from the two perpendicular edge
//! sample lines, the estimated second derivatives, and the `qThr` / `sideThr`
//! threshold cascade over the caller-resolved `Q_First` table.
//!
//! Feature tracking: `RECON-DEBLOCK-SAMPLE-FILTER`,
//! `RECON-DEBLOCK-FILTER-MAX-WIDTH`, `RECON-DEBLOCK-ADAPTIVE-STRENGTH`,
//! `RECON-DEBLOCK-FILTER-CHOICE`.
//!
//! Scope: these are the per-edge sample math and the parameter derivations over
//! caller-resolved spec-derived values. The § 7.17.1 / § 7.17.2 edge traversal,
//! the § 7.17.6 filter-level selection (which needs the `DeblockingTxSizes`,
//! segment/qindex maps, and block state), the per-edge sample gathering into the
//! `s` / `t` lines `deblock_filter_choice` consumes, and the `Q_Thresh_Mults` /
//! `W_Mult` / `Side_Thresholds` / `Q_First` § 9.2 table lookups stay with the
//! caller — it passes the resolved widths, weights, level, thresholds, sample
//! lines, and tables as scalars/slices, exactly as the other `splot-recon`
//! primitives take caller-resolved spec-derived values. It does not read frame,
//! segment, or tile state.

use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::math::round2_i32;
use crate::{BitDepth, ReconError, ReconSample, Result};
use core::num::NonZeroUsize;
use std::simd::{
    Mask, Select, Simd, SimdElement,
    cmp::{SimdOrd, SimdPartialOrd},
    num::SimdInt,
    num::SimdUint,
    simd_swizzle,
};

/// AV2 § 3 `DF_SHIFT`: the deblocking-filter ramp shift
/// (`docs/spec/av2/1.0.0/03-symbols.md`, `DF_SHIFT = 8`).
const DF_SHIFT: u32 = 8;

/// AV2 § 3 `MAX_SIDE_TABLE`: the length of the § 9.2 `Side_Thresholds` array, the
/// upper bound (exclusive) on the § 7.17.5 `qInd`.
const MAX_SIDE_TABLE: usize = 296;

/// AV2 § 3 `QUANT_TABLE_BITS`: the § 7.14.4 / § 7.17.5 quantizer-table shift.
const QUANT_TABLE_BITS: u32 = 3;

/// AV2 § 3 `MAX_DBL_FLT_LEN`: the maximum deblocking-filter length, i.e. the
/// length of `Q_Thresh_Mults` / `W_Mult` and the maximum per-side width.
const MAX_DBL_FLT_LEN: usize = 8;

/// AV2 § 3 `DBL_REG_DECIS_LEN`: the length of the § 9.2 `Q_First` array
/// (`docs/spec/av2/1.0.0/03-symbols.md`, `DBL_REG_DECIS_LEN = 9`).
const DBL_REG_DECIS_LEN: usize = 9;

/// Caller-resolved parameters for the AV2 § 7.17.7.1 deblocking sample filter.
///
/// `boundary` is the index in `line` of the first current-side sample (`q0`); the
/// previous-side samples (`p0`, `p1`, …) are at `boundary - 1`, `boundary - 2`, …
/// `max_width_neg` / `max_width_pos` are the § 7.17 per-side maximum widths
/// (`1..=MAX_DBL_FLT_LEN`); `q_thr` is the filter threshold; `q_thresh_mult` is
/// `Q_Thresh_Mults[Max(max_width_neg, max_width_pos) - 1]` and `w_mult_neg` /
/// `w_mult_pos` are `W_Mult[max_width_neg - 1]` / `W_Mult[max_width_pos - 1]`,
/// resolved by the caller from the § 9.2 tables; `prev_lossless` / `curr_lossless`
/// gate the two sides; `bit_depth` bounds the `Clip1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeblockSampleFilter {
    /// Index in `line` of the first current-side sample (`q0`).
    pub boundary: usize,
    /// § 7.17 filter threshold `qThr`.
    pub q_thr: i32,
    /// Maximum modified previous-side (`p`) samples (`maxWidthNeg`, `1..=8`).
    pub max_width_neg: usize,
    /// Maximum modified current-side (`q`) samples (`maxWidthPos`, `1..=8`).
    pub max_width_pos: usize,
    /// `Q_Thresh_Mults[Max(max_width_neg, max_width_pos) - 1]`.
    pub q_thresh_mult: i32,
    /// `W_Mult[max_width_neg - 1]`.
    pub w_mult_neg: i32,
    /// `W_Mult[max_width_pos - 1]`.
    pub w_mult_pos: i32,
    /// Whether the previous-side samples are in a lossless segment (skips them).
    pub prev_lossless: bool,
    /// Whether the current-side samples are in a lossless segment (skips them).
    pub curr_lossless: bool,
    /// Active decoded bit depth (bounds the `Clip1`).
    pub bit_depth: BitDepth,
}

/// Applies the AV2 § 7.17.7.1 deblocking sample filter to the perpendicular
/// sample `line`, modifying it in place.
///
/// `deltaM2 = Clip3(-qThrClamp, qThrClamp, (p1 - q1 + 3*(q0 - p0)) * 4)` (with
/// `qThrClamp = q_thr * q_thresh_mult`) drives a per-sample ramp: for
/// `i = 0..Max(maxWidthNeg, maxWidthPos)`, the current-side sample at
/// `boundary + i` is `Clip1(sample - Round2(deltaM2 * w_mult_pos * (maxWidthPos -
/// i), 3 + DF_SHIFT))` (unless `curr_lossless`), and, for `i < maxWidthNeg`, the
/// previous-side sample at `boundary - 1 - i` is `Clip1(sample + Round2(deltaM2 *
/// w_mult_neg * (maxWidthNeg - i), 3 + DF_SHIFT))` (unless `prev_lossless`).
/// `q0`/`q1`/`p0`/`p1` are read from the original `line` before any write.
///
/// The computation is total and panic-free for valid inputs: the ramp uses `i32`
/// with saturating arithmetic, the `qThrClamp` bound is clamped non-negative
/// so `Clip3` never inverts, and the line bounds are validated before any sample
/// is read or written.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::DeblockFilterInvalidWidth`] if `max_width_neg` /
/// `max_width_pos` are not in `1..=8`, and [`ReconError::DeblockFilterLineTooShort`]
/// if `line` does not contain the previous- and current-side samples the filter
/// reads and writes around `boundary`. All inputs are validated before any sample
/// is modified.
pub fn deblock_sample_filter<T: ReconSample>(
    line: &mut [T],
    params: &DeblockSampleFilter,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    validate_sample_filter_span(line.len(), params, 1)?;
    deblock_sample_filter_inner(line, params, 1)
}

/// Applies [`deblock_sample_filter`] directly to samples separated by `stride`.
///
/// `params.boundary` is the index of `q0` in `samples`. This is the same sample
/// process as the contiguous API, without gathering a perpendicular line first.
///
/// # Errors
/// Returns the same errors as [`deblock_sample_filter`] when the storage type,
/// widths, or strided sample span are invalid.
#[inline]
pub fn deblock_sample_filter_strided<T: ReconSample>(
    samples: &mut [T],
    stride: NonZeroUsize,
    params: &DeblockSampleFilter,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let stride = stride.get();
    validate_sample_filter_span(samples.len(), params, stride)?;
    deblock_sample_filter_inner(samples, params, stride)
}

/// Applies [`deblock_sample_filter_strided`] to the four adjacent sample lines
/// that form one AV2 deblocking edge.
///
/// `lane_stride` advances from one line's `q0` to the next, while `stride`
/// advances perpendicular to the edge. All four spans are validated before any
/// sample is modified.
///
/// # Errors
/// Returns the same errors as [`deblock_sample_filter_strided`] when the
/// storage type, widths, or a strided sample span is invalid.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
pub fn deblock_sample_filter_strided_4<T: ReconSample>(
    samples: &mut [T],
    stride: NonZeroUsize,
    lane_stride: NonZeroUsize,
    params: &DeblockSampleFilter,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let stride = stride.get();
    validate_sample_filter_span(samples.len(), params, stride)?;
    let last_boundary = lane_stride
        .get()
        .checked_mul(3)
        .and_then(|offset| params.boundary.checked_add(offset))
        .ok_or(ReconError::DeblockFilterLineTooShort {
            boundary: params.boundary,
            max_width_neg: params.max_width_neg,
            width: params.max_width_neg.max(params.max_width_pos),
            len: samples.len(),
        })?;
    validate_sample_filter_span(
        samples.len(),
        &DeblockSampleFilter {
            boundary: last_boundary,
            ..*params
        },
        stride,
    )?;
    deblock_sample_filter_strided_4_validated(samples, stride, lane_stride.get(), params)
}

#[allow(clippy::inline_always, reason = "shared deblock validation hot path")]
#[inline(always)]
fn deblock_sample_filter_strided_4_validated<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    lane_stride: usize,
    params: &DeblockSampleFilter,
) -> Result<()> {
    let max_weight = params.w_mult_neg.max(params.w_mult_pos);
    let bounded_factor =
        (i128::from(max_weight) * params.max_width_neg.max(params.max_width_pos) as i128).max(1);
    let bounded_product =
        i128::from(params.q_thr) * i128::from(params.q_thresh_mult) * bounded_factor;
    if params.q_thr >= 0
        && params.q_thresh_mult >= 0
        && params.w_mult_neg >= 0
        && params.w_mult_pos >= 0
        && bounded_product <= i128::from(i32::MAX - (1 << 10))
    {
        deblock_sample_filter_inner_4_bounded(samples, params, stride, lane_stride)
    } else {
        deblock_sample_filter_inner_lanes(samples, params, stride, lane_stride, 4)
    }
}

#[inline]
fn validate_sample_filter_span(
    len: usize,
    params: &DeblockSampleFilter,
    stride: usize,
) -> Result<()> {
    let DeblockSampleFilter {
        boundary,
        max_width_neg,
        max_width_pos,
        ..
    } = *params;

    if !(1..=MAX_DBL_FLT_LEN).contains(&max_width_neg)
        || !(1..=MAX_DBL_FLT_LEN).contains(&max_width_pos)
    {
        return Err(ReconError::DeblockFilterInvalidWidth {
            max_width_neg,
            max_width_pos,
        });
    }
    let width = max_width_neg.max(max_width_pos);
    let low_extent = max_width_neg.max(2);
    let high_extent = width.max(2);
    let low_span = low_extent.checked_mul(stride);
    let high_span = high_extent
        .checked_sub(1)
        .and_then(|extent| extent.checked_mul(stride));
    let span_is_valid = low_span.is_some_and(|span| boundary >= span)
        && high_span
            .and_then(|span| boundary.checked_add(span))
            .is_some_and(|last| last < len);
    if !span_is_valid {
        return Err(ReconError::DeblockFilterLineTooShort {
            boundary,
            max_width_neg,
            width,
            len,
        });
    }
    Ok(())
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_sample_filter_inner<T: ReconSample>(
    line: &mut [T],
    params: &DeblockSampleFilter,
    stride: usize,
) -> Result<()> {
    deblock_sample_filter_inner_lanes(line, params, stride, 1, 1)
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_sample_filter_inner_lanes<T: ReconSample>(
    line: &mut [T],
    params: &DeblockSampleFilter,
    stride: usize,
    lane_stride: usize,
    lanes: usize,
) -> Result<()> {
    let DeblockSampleFilter {
        boundary,
        q_thr,
        max_width_neg,
        max_width_pos,
        q_thresh_mult,
        w_mult_neg,
        w_mult_pos,
        prev_lossless,
        curr_lossless,
        bit_depth,
    } = *params;
    let shift = 3 + DF_SHIFT;
    let max_sample = i32::from(bit_depth.max_sample());
    let width = max_width_neg.max(max_width_pos);
    for lane in 0..lanes {
        let boundary = boundary + lane * lane_stride;
        let q0 = i32::from(line[boundary].to_u16());
        let q1 = i32::from(line[boundary + stride].to_u16());
        let p0 = i32::from(line[boundary - stride].to_u16());
        let p1 = i32::from(line[boundary - 2 * stride].to_u16());

        let q_thr_clamp = q_thr.saturating_mul(q_thresh_mult).max(0);
        let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
        let delta_m2_neg = delta_m2.saturating_mul(w_mult_neg);
        let delta_m2_pos = delta_m2.saturating_mul(w_mult_pos);

        for i in 0..width {
            let signed_i = i as i32;
            let diff_pos = round2_i32(
                delta_m2_pos.saturating_mul(max_width_pos as i32 - signed_i),
                shift,
            );
            if !curr_lossless {
                let index = boundary + i * stride;
                let value = (i32::from(line[index].to_u16()) - diff_pos).clamp(0, max_sample);
                line[index] = T::try_from_u16(value as u16)?;
            }
            if i < max_width_neg && !prev_lossless {
                let diff_neg = round2_i32(
                    delta_m2_neg.saturating_mul(max_width_neg as i32 - signed_i),
                    shift,
                );
                let index = boundary - (i + 1) * stride;
                let value = (i32::from(line[index].to_u16()) + diff_neg).clamp(0, max_sample);
                line[index] = T::try_from_u16(value as u16)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn load_u16x4(line: &[u16], start: usize) -> Simd<i32, 4> {
    Simd::<u16, 4>::from_slice(&line[start..]).cast::<i32>()
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn load_strided_u16x4(line: &[u16], start: usize, stride: usize) -> Simd<i32, 4> {
    Simd::from_array([
        line[start],
        line[start + stride],
        line[start + 2 * stride],
        line[start + 3 * stride],
    ])
    .cast::<i32>()
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_width_contiguous_rows<const WIDTH: usize>(
    line: &mut [u16],
    params: &DeblockSampleFilter,
    lane_stride: usize,
) {
    let factors_pos = Simd::from_array(core::array::from_fn(|index| (WIDTH - index) as i32));
    let factors_neg = Simd::from_array(core::array::from_fn(|index| (index + 1) as i32));
    let low = Simd::splat(0);
    let high = Simd::splat(i32::from(params.bit_depth.max_sample()));
    let q_thr_clamp = params.q_thr * params.q_thresh_mult;
    for lane in 0..4 {
        let boundary = params.boundary + lane * lane_stride;
        let q0 = i32::from(line[boundary]);
        let q1 = i32::from(line[boundary + 1]);
        let p0 = i32::from(line[boundary - 1]);
        let p1 = i32::from(line[boundary - 2]);
        let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
        if !params.curr_lossless {
            let samples = Simd::<u16, WIDTH>::from_slice(&line[boundary..]).cast::<i32>();
            let diff = (Simd::splat(delta_m2 * params.w_mult_pos) * factors_pos
                + Simd::splat(1 << 10))
                >> 11;
            // splot-copy-ok: publish filtered contiguous SIMD lanes
            line[boundary..boundary + WIDTH].copy_from_slice(
                &(samples - diff)
                    .simd_max(low)
                    .simd_min(high)
                    .cast::<u16>()
                    .to_array(),
            );
        }
        if !params.prev_lossless {
            let start = boundary - WIDTH;
            let samples = Simd::<u16, WIDTH>::from_slice(&line[start..]).cast::<i32>();
            let diff = (Simd::splat(delta_m2 * params.w_mult_neg) * factors_neg
                + Simd::splat(1 << 10))
                >> 11;
            // splot-copy-ok: publish filtered contiguous SIMD lanes
            line[start..boundary].copy_from_slice(
                &(samples + diff)
                    .simd_max(low)
                    .simd_min(high)
                    .cast::<u16>()
                    .to_array(),
            );
        }
    }
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_sample_filter_inner_4_bounded<T: ReconSample>(
    line: &mut [T],
    params: &DeblockSampleFilter,
    stride: usize,
    lane_stride: usize,
) -> Result<()> {
    let DeblockSampleFilter {
        boundary,
        q_thr,
        max_width_neg,
        max_width_pos,
        q_thresh_mult,
        w_mult_neg,
        w_mult_pos,
        prev_lossless,
        curr_lossless,
        bit_depth,
    } = *params;
    let q_thr_clamp = q_thr * q_thresh_mult;
    let max_sample = i32::from(bit_depth.max_sample());
    let width = max_width_neg.max(max_width_pos);
    if stride == 1 {
        if let Some(line) = T::u16_slice_mut(line) {
            if max_width_neg == max_width_pos && lane_stride >= 2 * max_width_neg {
                match max_width_neg {
                    4 => {
                        deblock_width_contiguous_rows::<4>(line, params, lane_stride);
                        return Ok(());
                    }
                    8 => {
                        deblock_width_contiguous_rows::<8>(line, params, lane_stride);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            let q0 = load_strided_u16x4(line, boundary, lane_stride);
            let q1 = load_strided_u16x4(line, boundary + 1, lane_stride);
            let p0 = load_strided_u16x4(line, boundary - 1, lane_stride);
            let p1 = load_strided_u16x4(line, boundary - 2, lane_stride);
            let delta_m2 = ((p1 - q1 + (q0 - p0) * Simd::splat(3)) * Simd::splat(4))
                .simd_max(Simd::splat(-q_thr_clamp))
                .simd_min(Simd::splat(q_thr_clamp));
            let delta_neg = delta_m2 * Simd::splat(w_mult_neg);
            let delta_pos = delta_m2 * Simd::splat(w_mult_pos);
            let low = Simd::splat(0);
            let high = Simd::splat(max_sample);
            if !curr_lossless {
                macro_rules! apply_positive {
                    ($width:expr) => {
                        for i in 0..$width {
                            let start = boundary + i;
                            let factor = Simd::splat(max_width_pos as i32 - i as i32);
                            let diff = (delta_pos * factor + Simd::splat(1 << 10)) >> 11;
                            let values = (load_strided_u16x4(line, start, lane_stride) - diff)
                                .simd_max(low)
                                .simd_min(high)
                                .cast::<u16>()
                                .to_array();
                            for (lane, value) in values.into_iter().enumerate() {
                                line[start + lane * lane_stride] = value; // splot-copy-ok: scatter four SIMD deblock lanes back to their rows
                            }
                        }
                    }
                }
                if width == 3 {
                    apply_positive!(3);
                } else {
                    apply_positive!(width);
                }
            }
            if !prev_lossless {
                macro_rules! apply_negative {
                    ($width:expr) => {
                        for i in 0..$width {
                            let start = boundary - i - 1;
                            let factor = Simd::splat((max_width_neg - i) as i32);
                            let diff = (delta_neg * factor + Simd::splat(1 << 10)) >> 11;
                            let values = (load_strided_u16x4(line, start, lane_stride) + diff)
                                .simd_max(low)
                                .simd_min(high)
                                .cast::<u16>()
                                .to_array();
                            for (lane, value) in values.into_iter().enumerate() {
                                line[start + lane * lane_stride] = value; // splot-copy-ok: scatter four SIMD deblock lanes back to their rows
                            }
                        }
                    }
                }
                if max_width_neg == 3 {
                    apply_negative!(3);
                } else {
                    apply_negative!(max_width_neg);
                }
            }
            return Ok(());
        }
        let low_extent = max_width_neg.max(2);
        for lane in 0..4 {
            let boundary = boundary + lane * lane_stride;
            let window = &mut line[boundary - low_extent..boundary + width.max(2)];
            let q0 = i32::from(window[low_extent].to_u16());
            let q1 = i32::from(window[low_extent + 1].to_u16());
            let p0 = i32::from(window[low_extent - 1].to_u16());
            let p1 = i32::from(window[low_extent - 2].to_u16());
            let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
            let (neg_side, pos_side) = window.split_at_mut(low_extent);
            if !curr_lossless {
                let delta_m2_pos = delta_m2 * w_mult_pos;
                for (i, sample) in pos_side[..width].iter_mut().enumerate() {
                    let diff = (delta_m2_pos * (max_width_pos as i32 - i as i32) + (1 << 10)) >> 11;
                    let value = (i32::from(sample.to_u16()) - diff).clamp(0, max_sample);
                    *sample = T::try_from_u16(value as u16)?;
                }
            }
            if !prev_lossless {
                let delta_m2_neg = delta_m2 * w_mult_neg;
                let ramp = neg_side[low_extent - max_width_neg..].iter_mut();
                for (i, sample) in ramp.enumerate() {
                    let diff = (delta_m2_neg * (i as i32 + 1) + (1 << 10)) >> 11;
                    let value = (i32::from(sample.to_u16()) + diff).clamp(0, max_sample);
                    *sample = T::try_from_u16(value as u16)?;
                }
            }
        }
        return Ok(());
    }
    if lane_stride == 1 && stride >= 4 {
        if let Some(line) = T::u16_slice_mut(line) {
            let q0 = load_u16x4(line, boundary);
            let q1 = load_u16x4(line, boundary + stride);
            let p0 = load_u16x4(line, boundary - stride);
            let p1 = load_u16x4(line, boundary - 2 * stride);
            let delta_m2 = ((p1 - q1 + (q0 - p0) * Simd::splat(3)) * Simd::splat(4))
                .simd_max(Simd::splat(-q_thr_clamp))
                .simd_min(Simd::splat(q_thr_clamp));
            let delta_neg = delta_m2 * Simd::splat(w_mult_neg);
            let delta_pos = delta_m2 * Simd::splat(w_mult_pos);
            let low = Simd::splat(0);
            let high = Simd::splat(max_sample);
            if !curr_lossless {
                macro_rules! apply_positive {
                    ($width:expr) => {
                        for i in 0..$width {
                            let row_start = boundary + i * stride;
                            let factor = Simd::splat(max_width_pos as i32 - i as i32);
                            let diff = (delta_pos * factor + Simd::splat(1 << 10)) >> 11;
                            let value = load_u16x4(line, row_start) - diff; // splot-copy-ok: publish four SIMD lanes back into the in-place deblock row
                            line[row_start..row_start + 4].copy_from_slice(
                                &value
                                    .simd_max(low)
                                    .simd_min(high)
                                    .cast::<u16>()
                                    .to_array(),
                            );
                        }
                    };
                }
                if width == 3 {
                    apply_positive!(3);
                } else {
                    apply_positive!(width);
                }
            }
            if !prev_lossless {
                macro_rules! apply_negative {
                    ($width:expr) => {
                        for i in 0..$width {
                            let row_start = boundary - (i + 1) * stride;
                            let factor = Simd::splat((max_width_neg - i) as i32);
                            let diff = (delta_neg * factor + Simd::splat(1 << 10)) >> 11;
                            let value = load_u16x4(line, row_start) + diff; // splot-copy-ok: publish four SIMD lanes back into the in-place deblock row
                            line[row_start..row_start + 4].copy_from_slice(
                                &value
                                    .simd_max(low)
                                    .simd_min(high)
                                    .cast::<u16>()
                                    .to_array(),
                            );
                        }
                    };
                }
                if max_width_neg == 3 {
                    apply_negative!(3);
                } else {
                    apply_negative!(max_width_neg);
                }
            }
            return Ok(());
        }
        let mut delta_neg = [0i32; 4];
        let mut delta_pos = [0i32; 4];
        for (lane, (neg, pos)) in delta_neg.iter_mut().zip(&mut delta_pos).enumerate() {
            let boundary = boundary + lane;
            let q0 = i32::from(line[boundary].to_u16());
            let q1 = i32::from(line[boundary + stride].to_u16());
            let p0 = i32::from(line[boundary - stride].to_u16());
            let p1 = i32::from(line[boundary - 2 * stride].to_u16());
            let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
            *neg = delta_m2 * w_mult_neg;
            *pos = delta_m2 * w_mult_pos;
        }
        if !curr_lossless {
            for i in 0..width {
                let row_start = boundary + i * stride;
                let factor = max_width_pos as i32 - i as i32;
                for (lane, sample) in line[row_start..row_start + 4].iter_mut().enumerate() {
                    let diff = (delta_pos[lane] * factor + (1 << 10)) >> 11;
                    let value = (i32::from(sample.to_u16()) - diff).clamp(0, max_sample);
                    *sample = T::try_from_u16(value as u16)?;
                }
            }
        }
        if !prev_lossless {
            for i in 0..max_width_neg {
                let row_start = boundary - (i + 1) * stride;
                let factor = (max_width_neg - i) as i32;
                for (lane, sample) in line[row_start..row_start + 4].iter_mut().enumerate() {
                    let diff = (delta_neg[lane] * factor + (1 << 10)) >> 11;
                    let value = (i32::from(sample.to_u16()) + diff).clamp(0, max_sample);
                    *sample = T::try_from_u16(value as u16)?;
                }
            }
        }
        return Ok(());
    }
    for lane in 0..4 {
        let boundary = boundary + lane * lane_stride;
        let q0 = i32::from(line[boundary].to_u16());
        let q1 = i32::from(line[boundary + stride].to_u16());
        let p0 = i32::from(line[boundary - stride].to_u16());
        let p1 = i32::from(line[boundary - 2 * stride].to_u16());
        let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
        let delta_m2_neg = delta_m2 * w_mult_neg;
        let delta_m2_pos = delta_m2 * w_mult_pos;

        for i in 0..width {
            let diff_pos = (delta_m2_pos * (max_width_pos as i32 - i as i32) + (1 << 10)) >> 11;
            if !curr_lossless {
                let index = boundary + i * stride;
                let value = (i32::from(line[index].to_u16()) - diff_pos).clamp(0, max_sample);
                line[index] = T::try_from_u16(value as u16)?;
            }
            if i < max_width_neg && !prev_lossless {
                let diff_neg = (delta_m2_neg * (max_width_neg - i) as i32 + (1 << 10)) >> 11;
                let index = boundary - (i + 1) * stride;
                let value = (i32::from(line[index].to_u16()) + diff_neg).clamp(0, max_sample);
                line[index] = T::try_from_u16(value as u16)?;
            }
        }
    }
    Ok(())
}

/// Derives the AV2 § 7.17.3 deblocking filter maximum per-side widths
/// `(maxWidthNeg, maxWidthPos)`
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-3`).
///
/// `filter_size` is the § 7.17.4 maximum filter size (a transform dimension);
/// `is_chroma` is the spec `plane != 0`; `sb_edge` is whether the edge is at a
/// super-block boundary. The result is the pair of caller-resolved widths the
/// § 7.17.7.1 sample filter ([`deblock_sample_filter`]) takes as `max_width_neg`
/// / `max_width_pos`.
///
/// `maxWidthPos` is `1` for `filter_size <= 4`, `3` for `8`, `is_chroma ? 4 : 6`
/// for `16`, and `is_chroma ? 4 : 8` otherwise; `maxWidthNeg` is
/// `Min(maxWidthPos, is_chroma ? 2 : 6)` at a super-block edge and `maxWidthPos`
/// otherwise. This is a total `const fn`: every input maps to a defined pair.
pub const fn deblock_filter_max_width(
    filter_size: usize,
    is_chroma: bool,
    sb_edge: bool,
) -> (usize, usize) {
    let max_width_pos = if filter_size <= 4 {
        1
    } else if filter_size == 8 {
        3
    } else if filter_size == 16 {
        if is_chroma { 4 } else { 6 }
    } else if is_chroma {
        4
    } else {
        8
    };
    let max_width_neg = if sb_edge {
        let cap = if is_chroma { 2 } else { 6 };
        if max_width_pos < cap {
            max_width_pos
        } else {
            cap
        }
    } else {
        max_width_pos
    };
    (max_width_neg, max_width_pos)
}

const _MAX_WIDTH_CONST_CHECK: () =
    assert!(matches!(deblock_filter_max_width(32, false, false), (8, 8)));

/// Derives the AV2 § 7.17.5 `qInd`, the index into the § 9.2 `Side_Thresholds`
/// array (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-5`):
/// `Clip3(0, MAX_SIDE_TABLE - 1, lvl - 24 * (BitDepth - 8))`.
///
/// `lvl` is the § 7.17.6 adaptive filter level. The caller uses the result to
/// look up `Side_Thresholds[qInd]` and pass it as the `side_threshold` of
/// [`deblock_adaptive_filter_strength`] (`Side_Thresholds` lives in `splot-core`'s
/// generated § 9.2 tables, which `splot-recon` cannot reach). This is a total
/// `const fn`.
pub const fn deblock_side_threshold_index(lvl: u32, bit_depth: BitDepth) -> usize {
    let adjustment = 24 * (bit_depth.bits() - 8) as u32;
    let q = lvl.saturating_sub(adjustment) as usize;
    if q < MAX_SIDE_TABLE {
        q
    } else {
        MAX_SIDE_TABLE - 1
    }
}

/// Derives the AV2 § 7.17.5 adaptive filter strength outputs `(qThr, side)` from
/// the filter level `lvl`, the caller-resolved `side_threshold =
/// Side_Thresholds[qInd]` (with `qInd` from [`deblock_side_threshold_index`]), and
/// the active `bit_depth`
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-5`).
///
/// `qThr = Round2(get_q(lvl, 0), QUANT_TABLE_BITS) >> 6` (the § 7.14.2
/// quantizer-value lookup; [`quantizer_value`](crate::quantizer_value)), and
/// `side = Max(side_threshold + (1 << (12 - BitDepth)), 0) >> (13 - BitDepth)`.
/// `qThr` is the threshold the § 7.17.7.1 sample filter
/// ([`deblock_sample_filter`]) takes as `q_thr`; `side` is the side threshold the
/// § 7.17.7.2 filter-choice process uses.
///
/// The computation is total and panic-free: the quantizer lookup is total, and
/// the `i32` arithmetic with `bit_depth` shifts (`12 - BitDepth` and
/// `13 - BitDepth` are positive for the 8- and 10-bit depths) cannot overflow.
pub fn deblock_adaptive_filter_strength(
    lvl: u32,
    side_threshold: i32,
    bit_depth: BitDepth,
) -> (i32, i32) {
    let bits = u32::from(bit_depth.bits());
    let get_q = quantizer_value(lvl, 0, bit_depth) as i32;
    let q_thr = ((get_q + (1 << (QUANT_TABLE_BITS - 1))) >> QUANT_TABLE_BITS) >> 6;
    let side = side_threshold.saturating_add(1i32 << (12 - bits)).max(0) >> (13 - bits);
    (q_thr, side)
}

/// Caller-resolved parameters for the AV2 § 7.17.7.2 deblocking filter-choice
/// process (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-7-2`).
///
/// `boundary` is the index in the `s` / `t` perpendicular sample lines of the
/// first current-side sample (the spec `s[0]` / `t[0]`, at the edge); the
/// previous-side samples (`s[-1]`, `s[-2]`, …) are at `boundary - 1`,
/// `boundary - 2`, …. `q_thr` and `side_thr` are the § 7.17.5 thresholds.
/// `max_width_neg` / `max_width_pos` are the § 7.17.3 per-side maximum widths
/// (`1..=MAX_DBL_FLT_LEN`). `q_first` is the § 9.2 `Q_First` array, resolved by
/// the caller from `splot-core`'s generated tables (which `splot-recon` cannot
/// reach).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeblockFilterChoice {
    /// Index in `s` / `t` of the first current-side sample (the spec `s[0]`).
    pub boundary: usize,
    /// § 7.17.5 filter threshold `qThr`.
    pub q_thr: i32,
    /// § 7.17.5 side threshold `sideThr`.
    pub side_thr: i32,
    /// Maximum current-side width (`maxWidthPos`, `1..=8`).
    pub max_width_pos: usize,
    /// Maximum previous-side width (`maxWidthNeg`, `1..=8`).
    pub max_width_neg: usize,
    /// The § 9.2 `Q_First` array (`Q_First[dist - 4]` in the width loop).
    pub q_first: [i32; DBL_REG_DECIS_LEN],
}

/// Chooses the AV2 § 7.17.7.2 deblocking filter width from the two perpendicular
/// sample lines `s` (the first row/column of the edge) and `t` (the last,
/// `count - 1` row/column), returning the number of samples to filter
/// (`0..=maxWidthPos`).
///
/// The process estimates the second derivative `secondDeriv[-2..=1]` of the
/// samples at the edge from both lines, then walks a cascade of threshold tests
/// (`sideThr`, `sideThr >> 2`, `sideThr >> 3`, `(sideThr * 3) >> 4`, and the
/// per-distance `(sideThr * dist) >> 4` / `qThr * Q_First[dist - 4]`), widening
/// the chosen width while the samples stay flat enough and stopping at the first
/// threshold the local curvature exceeds. It returns `0` immediately when
/// `q_thr` or `side_thr` is `0`.
///
/// `s` / `t` are read-only; this is the width decision that the § 7.17.7.1
/// [`deblock_sample_filter`] consumes, not the sample modification.
///
/// The computation is total and panic-free: every sample access stays within the
/// `[boundary - maxSamplesNeg, boundary + maxSamplesPos - 1]` window (with the
/// unconditional `s[3]` read covered for every `maxWidthPos > 1`, and the deeper
/// negative reads guarded by the matching `maxWidthNeg` conditions), the line
/// lengths are validated before any sample is read, the `i32` arithmetic cannot
/// overflow, and `q_first` is a fixed-size array so the `Q_First[dist - 4]`
/// lookup (`dist - 4 <= 4`) is always in bounds.
///
/// # Errors
/// Returns [`ReconError::DeblockFilterInvalidWidth`] if `max_width_neg` /
/// `max_width_pos` are not in `1..=8`, and
/// [`ReconError::DeblockFilterLineTooShort`] if `s` or `t` does not contain the
/// samples the cascade reads around `boundary`. All inputs are validated before
/// any sample is read.
pub fn deblock_filter_choice<T: ReconSample>(
    s: &[T],
    t: &[T],
    params: &DeblockFilterChoice,
) -> Result<usize> {
    let DeblockFilterChoice {
        boundary,
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        q_first: _,
    } = *params;

    if q_thr == 0 || side_thr == 0 {
        return Ok(0);
    }

    if !(1..=MAX_DBL_FLT_LEN).contains(&max_width_neg)
        || !(1..=MAX_DBL_FLT_LEN).contains(&max_width_pos)
    {
        return Err(ReconError::DeblockFilterInvalidWidth {
            max_width_neg,
            max_width_pos,
        });
    }

    let max_samples_neg = (max_width_neg + 1).clamp(3, MAX_DBL_FLT_LEN);
    let max_samples_pos = (max_width_pos + 1).clamp(3, MAX_DBL_FLT_LEN);
    let pos_span = if max_width_pos == 1 {
        max_samples_pos
    } else {
        max_samples_pos.max(4)
    };
    for line in [s, t] {
        if boundary < max_samples_neg || boundary + pos_span > line.len() {
            return Err(ReconError::DeblockFilterLineTooShort {
                boundary,
                max_width_neg,
                width: max_width_neg.max(max_width_pos),
                len: line.len(),
            });
        }
    }

    Ok(deblock_filter_choice_progressive(params, |offset| {
        let index = (boundary as isize + offset) as usize;
        (i32::from(s[index].to_u16()), i32::from(t[index].to_u16()))
    }))
}

/// Chooses the deblocking width from two lines inside one strided sample plane.
///
/// `params.boundary` locates the first line's `q0`; `last_boundary` locates the
/// final line's `q0`, and `stride` advances one sample perpendicular to the edge.
///
/// # Errors
/// Returns the same errors as [`deblock_filter_choice`] when widths or either
/// strided sample span are invalid.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
pub fn deblock_filter_choice_strided<T: ReconSample>(
    samples: &[T],
    last_boundary: usize,
    stride: NonZeroUsize,
    params: &DeblockFilterChoice,
) -> Result<usize> {
    let DeblockFilterChoice {
        boundary,
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        q_first: _,
    } = *params;

    if q_thr == 0 || side_thr == 0 {
        return Ok(0);
    }
    if !(1..=MAX_DBL_FLT_LEN).contains(&max_width_neg)
        || !(1..=MAX_DBL_FLT_LEN).contains(&max_width_pos)
    {
        return Err(ReconError::DeblockFilterInvalidWidth {
            max_width_neg,
            max_width_pos,
        });
    }

    let stride = stride.get();
    let max_samples_neg = (max_width_neg + 1).clamp(3, MAX_DBL_FLT_LEN);
    let max_samples_pos = (max_width_pos + 1).clamp(3, MAX_DBL_FLT_LEN);
    let pos_span = if max_width_pos == 1 {
        max_samples_pos
    } else {
        max_samples_pos.max(4)
    };
    let neg_span = max_samples_neg.checked_mul(stride);
    let positive_samples = pos_span;
    let pos_span = positive_samples
        .checked_sub(1)
        .and_then(|span| span.checked_mul(stride));
    for line_boundary in [boundary, last_boundary] {
        let valid = neg_span.is_some_and(|span| line_boundary >= span)
            && pos_span
                .and_then(|span| line_boundary.checked_add(span))
                .is_some_and(|last| last < samples.len());
        if !valid {
            return Err(ReconError::DeblockFilterLineTooShort {
                boundary: line_boundary,
                max_width_neg,
                width: max_width_neg.max(max_width_pos),
                len: samples.len(),
            });
        }
    }

    Ok(deblock_filter_choice_progressive(params, |offset| {
        let distance = offset.unsigned_abs() * stride;
        let first_index = if offset < 0 {
            boundary - distance
        } else {
            boundary + distance
        };
        let last_index = if offset < 0 {
            last_boundary - distance
        } else {
            last_boundary + distance
        };
        (
            i32::from(samples[first_index].to_u16()),
            i32::from(samples[last_index].to_u16()),
        )
    }))
}

/// Chooses and applies a four-lane strided deblocking filter with one shared
/// sample-span validation.
///
/// The table arguments are the caller-resolved AV2 § 9.2
/// `Q_Thresh_Mults` and `W_Mult` arrays. The width-choice span covers every
/// sample subsequently read or written by the selected filter width.
///
/// # Errors
/// Returns the same errors as [`deblock_filter_choice_strided`] and
/// [`deblock_sample_filter_strided_4`].
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
pub fn deblock_filter_choice_and_sample_strided_4<T: ReconSample>(
    samples: &mut [T],
    last_boundary: usize,
    stride: NonZeroUsize,
    lane_stride: NonZeroUsize,
    choice: &DeblockFilterChoice,
    q_thresh_mults: &[i32; MAX_DBL_FLT_LEN],
    w_mults: &[i32; MAX_DBL_FLT_LEN],
    prev_lossless: bool,
    curr_lossless: bool,
    bit_depth: BitDepth,
) -> Result<usize> {
    let width = deblock_filter_choice_strided(samples, last_boundary, stride, choice)?;
    apply_deblock_choice_strided_4(
        samples,
        stride.get(),
        lane_stride.get(),
        choice,
        q_thresh_mults,
        w_mults,
        prev_lossless,
        curr_lossless,
        bit_depth,
        width,
    )
}

/// Chooses and applies `edges` consecutive four-line § 7.17.7 edges whose
/// lines are sample rows (vertical edges) and which share one edge decision:
/// `choice.boundary` is the first row's `q0`, `stride` steps from one row to
/// the next, and each edge starts four rows below the previous one.
///
/// Per edge, the filter choice reads its first and last rows around the edge
/// as vectors, and the sample filter updates each row as one vector. The
/// kernel skips an edge that the § 9.2 `W_Mult` weights do not change, so
/// `w_mults` must be that table.
///
/// # Errors
/// Returns [`ReconError::DeblockFilterInvalidWidth`] for widths outside
/// `1..=8`, [`ReconError::DeblockFilterLineTooShort`] when a row's eight
/// samples either side of the edge fall outside `samples`,
/// [`ReconError::StrideTooSmall`] when `stride` is below the 16 samples of a
/// row, so rows would overlap, and
/// [`ReconError::SampleTypeUnsupportedBitDepth`] when `T` cannot hold
/// `bit_depth`.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn deblock_edge_rows<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    choice: &DeblockFilterChoice,
    edges: usize,
    q_thresh_mults: &[i32; MAX_DBL_FLT_LEN],
    w_mults: &[i32; MAX_DBL_FLT_LEN],
    prev_lossless: bool,
    curr_lossless: bool,
    bit_depth: BitDepth,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    reject_overlapping_lines(stride, 2 * EDGE_REACH)?;
    if choice.q_thr == 0 || choice.side_thr == 0 {
        return Ok(());
    }
    let edge = EdgeKernel::new(
        choice,
        q_thresh_mults,
        w_mults,
        prev_lossless,
        curr_lossless,
        bit_depth,
    )?;
    let len = samples.len();
    let first = choice
        .boundary
        .checked_sub(EDGE_REACH)
        .filter(|first| {
            edges
                .checked_mul(MI_LINES)
                .and_then(|lines| lines.checked_sub(1))
                .and_then(|last| stride.checked_mul(last))
                .and_then(|offset| first.checked_add(offset))
                .and_then(|last| last.checked_add(2 * EDGE_REACH))
                .is_some_and(|end| end <= len)
        })
        .ok_or_else(|| edge.too_short(len))?;
    if let Some(samples) = T::u16_slice_mut(samples) {
        edge.rows(samples, first, stride, edges);
    } else if let Some(samples) = T::u8_slice_mut(samples) {
        edge.rows(samples, first, stride, edges);
    } else {
        return Err(edge.too_short(len));
    }
    Ok(())
}

/// Chooses and applies `edges` consecutive four-line § 7.17.7 edges whose
/// lines are sample columns (horizontal edges) and which share one edge
/// decision: `choice.boundary` is the first column's `q0`, `stride` steps
/// across the edge, and each edge starts four columns right of the previous.
/// `w_mults` must be the § 9.2 `W_Mult` table, as for [`deblock_edge_rows`].
///
/// # Errors
/// Returns the same errors as [`deblock_edge_rows`], with
/// [`ReconError::StrideTooSmall`] when `stride` is below the run's
/// `4 * edges` columns, so columns would overlap.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn deblock_edge_columns<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    choice: &DeblockFilterChoice,
    edges: usize,
    q_thresh_mults: &[i32; MAX_DBL_FLT_LEN],
    w_mults: &[i32; MAX_DBL_FLT_LEN],
    prev_lossless: bool,
    curr_lossless: bool,
    bit_depth: BitDepth,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    reject_overlapping_lines(stride, edges.saturating_mul(MI_LINES).max(1))?;
    if choice.q_thr == 0 || choice.side_thr == 0 {
        return Ok(());
    }
    let edge = EdgeKernel::new(
        choice,
        q_thresh_mults,
        w_mults,
        prev_lossless,
        curr_lossless,
        bit_depth,
    )?;
    let len = samples.len();
    let first = stride
        .checked_mul(EDGE_REACH)
        .and_then(|reach| choice.boundary.checked_sub(reach))
        .filter(|first| {
            stride
                .checked_mul(2 * EDGE_REACH - 1)
                .and_then(|offset| first.checked_add(offset))
                .and_then(|last| edges.checked_mul(MI_LINES)?.checked_add(last))
                .is_some_and(|end| end <= len)
        })
        .ok_or_else(|| edge.too_short(len))?;
    if let Some(samples) = T::u16_slice_mut(samples) {
        edge.column_run(samples, first, stride, edges);
    } else if let Some(samples) = T::u8_slice_mut(samples) {
        edge.column_run(samples, first, stride, edges);
    } else {
        return Err(edge.too_short(len));
    }
    Ok(())
}

/// Rejects a `stride` below `span`, the samples one line covers along it, at
/// which lines of one run would share samples.
fn reject_overlapping_lines(stride: usize, span: usize) -> Result<()> {
    if stride < span {
        return Err(ReconError::StrideTooSmall {
            stride_samples: stride,
            storage_width: span,
        });
    }
    Ok(())
}

/// `secondDeriv[-2..=1]` from the absolute second differences of the first
/// line (lanes 0..4) and the last line (lanes 4..8) at the same positions.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn combine_line_derivatives(differences: Simd<i16, 8>) -> [i32; 4] {
    let last = simd_swizzle!(differences, [4, 5, 6, 7, 0, 1, 2, 3]);
    let combined = ((differences + last + Simd::splat(1)) >> 1).to_array();
    core::array::from_fn(|index| i32::from(combined[index]))
}

/// Samples the widest § 7.17.7 filter reads on either side of an edge.
const EDGE_REACH: usize = MAX_DBL_FLT_LEN;

/// Lines one edge segment spans (`MI_SIZE`).
const MI_LINES: usize = 4;

/// Sample storage the edge kernels filter in place.
trait EdgeSample: SimdElement {
    /// Reads `N` samples as signed lanes.
    fn widen<const N: usize>(samples: &[Self]) -> Simd<i16, N>;

    /// Writes `N` lanes already clipped to the sample range.
    fn narrow<const N: usize>(values: Simd<i16, N>, samples: &mut [Self]);
}

impl EdgeSample for u8 {
    fn widen<const N: usize>(samples: &[Self]) -> Simd<i16, N> {
        Simd::<u8, N>::from_slice(samples).cast()
    }

    fn narrow<const N: usize>(values: Simd<i16, N>, samples: &mut [Self]) {
        values.cast::<u8>().copy_to_slice(samples);
    }
}

impl EdgeSample for u16 {
    fn widen<const N: usize>(samples: &[Self]) -> Simd<i16, N> {
        Simd::<u16, N>::from_slice(samples).cast()
    }

    fn narrow<const N: usize>(values: Simd<i16, N>, samples: &mut [Self]) {
        values.cast::<u16>().copy_to_slice(samples);
    }
}

/// One edge's filter-choice inputs and sample-filter weights.
struct EdgeKernel<'a> {
    choice: &'a DeblockFilterChoice,
    q_thresh_mults: &'a [i32; MAX_DBL_FLT_LEN],
    w_mults: &'a [i32; MAX_DBL_FLT_LEN],
    prev_lossless: bool,
    curr_lossless: bool,
    max_sample: i16,
    /// Largest `|p1 - q1 + 3 * (q0 - p0)|` on every line for which § 7.17.7.1
    /// moves no sample. A tap weight is at most `W_Mult[w - 1] * w`, which is
    /// 85 for `w = 1` and at most 120 for any `w`, `Round2(x, 11)` is zero for
    /// `|x| <= 1023`, and `maxWidthPos == 1` chooses `w = 1`.
    noop_delta: i16,
}

impl<'a> EdgeKernel<'a> {
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn new(
        choice: &'a DeblockFilterChoice,
        q_thresh_mults: &'a [i32; MAX_DBL_FLT_LEN],
        w_mults: &'a [i32; MAX_DBL_FLT_LEN],
        prev_lossless: bool,
        curr_lossless: bool,
        bit_depth: BitDepth,
    ) -> Result<Self> {
        let (max_width_neg, max_width_pos) = (choice.max_width_neg, choice.max_width_pos);
        if !(1..=MAX_DBL_FLT_LEN).contains(&max_width_neg)
            || !(1..=MAX_DBL_FLT_LEN).contains(&max_width_pos)
        {
            return Err(ReconError::DeblockFilterInvalidWidth {
                max_width_neg,
                max_width_pos,
            });
        }
        Ok(Self {
            choice,
            q_thresh_mults,
            w_mults,
            prev_lossless,
            curr_lossless,
            max_sample: bit_depth.max_sample() as i16,
            noop_delta: if max_width_pos == 1 { 3 } else { 2 },
        })
    }

    #[cold]
    fn too_short(&self, len: usize) -> ReconError {
        ReconError::DeblockFilterLineTooShort {
            boundary: self.choice.boundary,
            max_width_neg: self.choice.max_width_neg,
            width: self.choice.max_width_neg.max(self.choice.max_width_pos),
            len,
        }
    }

    /// The chosen per-side widths, the `deltaM2` clamp, and the per-side
    /// `W_Mult` weights (zero on a lossless side).
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn weights(&self, width: usize) -> (usize, usize, i16, i16, i16) {
        let width_neg = width.min(self.choice.max_width_neg);
        let width_pos = width.min(self.choice.max_width_pos);
        let q_thr_clamp = (i64::from(self.choice.q_thr)
            * i64::from(self.q_thresh_mults[width_neg.max(width_pos) - 1]))
        .clamp(0, i64::from(i16::MAX)) as i16;
        let weight = |lossless: bool, width: usize| {
            if lossless {
                0
            } else {
                self.w_mults[width - 1] as i16
            }
        };
        (
            width_neg,
            width_pos,
            q_thr_clamp,
            weight(self.prev_lossless, width_neg),
            weight(self.curr_lossless, width_pos),
        )
    }

    /// Whether § 7.17.7.1 leaves all four lines with these
    /// `p1 - q1 + 3 * (q0 - p0)` unchanged; the § 7.17.7.2 width then does
    /// not matter.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn unchanged(&self, deltas: Simd<i16, MI_LINES>) -> bool {
        deltas.abs().simd_le(Simd::splat(self.noop_delta)).all()
    }

    /// The § 7.17.7.1 `deltaM2` of the four lines.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn delta_m2<const N: usize>(deltas: Simd<i16, N>, q_thr_clamp: i16) -> Simd<i32, N> {
        (deltas * Simd::splat(4))
            .simd_max(Simd::splat(-q_thr_clamp))
            .simd_min(Simd::splat(q_thr_clamp))
            .cast::<i32>()
    }

    /// Filters `edges` consecutive row edges in four-edge chunks. The chunk
    /// derives every edge's width from vector compares, so the per-edge
    /// `unchanged` / `width == 0` exits of a scalar walk become mask bits and
    /// the cascade runs once per four edges.
    fn rows<E: EdgeSample>(&self, samples: &mut [E], first: usize, stride: usize, edges: usize) {
        let mut done = 0;
        while done < edges {
            let count = (edges - done).min(MI_LINES);
            self.rows_chunk::<E>(samples, first + done * MI_LINES * stride, stride, count);
            done += count;
        }
    }

    /// Filters up to [`MI_LINES`] consecutive row edges starting at `first`.
    ///
    /// Each edge contributes one lane of the cascade vectors; lanes at or
    /// past `count` hold values that fail every test, so they are never
    /// filtered and nothing past the run is read.
    fn rows_chunk<E: EdgeSample>(
        &self,
        samples: &mut [E],
        first: usize,
        stride: usize,
        count: usize,
    ) {
        let mut deltas = [Simd::<i16, MI_LINES>::splat(0); MI_LINES];
        let mut second = [Simd::<i32, MI_LINES>::splat(i32::MAX); 4];
        for (edge, slot) in deltas.iter_mut().enumerate().take(count) {
            let base = first + edge * MI_LINES * stride;
            let line = |row: usize| E::widen::<{ 2 * EDGE_REACH }>(&samples[base + row * stride..]);
            let (s, u, v, t) = (line(0), line(1), line(2), line(3));
            *slot = row_line_deltas(s, u, v, t);
            let [sd_m2, sd_m1, sd_0, sd_1] = row_second_derivatives(s, t);
            second[0].as_mut_array()[edge] = sd_m2;
            second[1].as_mut_array()[edge] = sd_m1;
            second[2].as_mut_array()[edge] = sd_0;
            second[3].as_mut_array()[edge] = sd_1;
        }
        let noop = Simd::splat(self.noop_delta);
        let mut changes = 0u8;
        for (edge, deltas) in deltas.iter().enumerate() {
            changes |= (u8::from(!deltas.abs().simd_le(noop).all())) << edge;
        }
        if changes == 0 {
            return;
        }
        let (mut widths, survivors) = self.cascade_widths(&second, count);
        if self.choice.max_width_pos > 2 {
            let mut left = survivors;
            while left != 0 {
                let edge = left.trailing_zeros() as usize;
                left &= left - 1;
                let base = first + edge * MI_LINES * stride;
                let s = E::widen::<{ 2 * EDGE_REACH }>(&samples[base..]);
                let t = E::widen::<{ 2 * EDGE_REACH }>(&samples[base + 3 * stride..]);
                widths[edge] = deblock_filter_choice_cascade(
                    self.choice,
                    [
                        second[0].as_array()[edge],
                        second[1].as_array()[edge],
                        second[2].as_array()[edge],
                        second[3].as_array()[edge],
                    ],
                    || row_cascade_derivatives(s, t),
                );
            }
        }
        let mut filter = 0u8;
        for (edge, width) in widths.iter().enumerate() {
            filter |= (u8::from(*width > 0)) << edge;
        }
        filter &= changes;
        while filter != 0 {
            let edge = filter.trailing_zeros() as usize;
            filter &= filter - 1;
            let width = widths[edge];
            let weights = self.weights(width);
            let delta = Self::delta_m2(deltas[edge], weights.2);
            let base = first + edge * MI_LINES * stride;
            if weights.0.max(weights.1) <= EDGE_REACH / 2 {
                self.filter_rows::<E, EDGE_REACH>(
                    samples,
                    base + EDGE_REACH / 2,
                    stride,
                    weights,
                    delta,
                );
            } else {
                self.filter_rows::<E, { 2 * EDGE_REACH }>(samples, base, stride, weights, delta);
            }
        }
    }

    /// The § 7.17.7.2 width of up to [`MI_LINES`] edges from their
    /// second-derivative lanes, without branches: lane `edge` of each vector
    /// is that edge's value. Returns the widths capped at `max_width_pos` and
    /// a bitmask of the edges that passed the second-derivative stage and so
    /// owe the directional cascade their final width.
    fn cascade_widths(
        &self,
        second: &[Simd<i32, MI_LINES>; 4],
        count: usize,
    ) -> ([usize; MI_LINES], u32) {
        let DeblockFilterChoice {
            q_thr,
            side_thr,
            max_width_pos,
            ..
        } = *self.choice;
        let outer = second[0].simd_max(second[3]);
        let inner = second[1] + second[2];
        let pass0 = outer.simd_le(Simd::splat(side_thr));
        let mut width = pass0.select(Simd::splat(1), Simd::splat(0));
        let mut pass2: Mask<i32, MI_LINES> = Mask::splat(false);
        if max_width_pos > 1 {
            let pass1 = pass0
                & outer.simd_le(Simd::splat(side_thr >> 2))
                & inner.simd_le(Simd::splat(q_thr * 4));
            width = pass1.select(Simd::splat(2), width);
            if max_width_pos > 2 {
                pass2 = pass1
                    & outer.simd_le(Simd::splat(side_thr >> 3))
                    & inner.simd_le(Simd::splat(q_thr * 3));
                width = pass2.select(Simd::splat(2), width);
            }
        }
        let capped = width.simd_min(Simd::splat(max_width_pos as i32));
        let mut widths = [0usize; MI_LINES];
        for (edge, width) in capped.as_array()[..count].iter().enumerate() {
            widths[edge] = *width as usize;
        }
        (widths, (pass2.to_bitmask() & ((1u64 << count) - 1)) as u32)
    }

    /// Applies § 7.17.7.1 to four rows of `N` samples centred on the edge.
    ///
    /// The `Round2` of the current side is negated into the coefficient:
    /// `-((x + 1024) >> 11) == (-x + 1023) >> 11`.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn filter_rows<E: EdgeSample, const N: usize>(
        &self,
        samples: &mut [E],
        first: usize,
        stride: usize,
        (width_neg, width_pos, _, w_neg, w_pos): (usize, usize, i16, i16, i16),
        delta: Simd<i32, MI_LINES>,
    ) {
        let half = (N / 2) as i16;
        let lane = Simd::<i16, N>::from_array(core::array::from_fn(|lane| lane as i16));
        let current = lane.simd_ge(Simd::splat(half));
        let zero = Simd::splat(0);
        let neg = (Simd::splat(width_neg as i16 + 1 - half) + lane).simd_max(zero);
        let pos = (Simd::splat(width_pos as i16 + half) - lane).simd_max(zero);
        let coefficient = current
            .select(-(pos * Simd::splat(w_pos)), neg * Simd::splat(w_neg))
            .cast::<i32>();
        let round = current
            .cast::<i32>()
            .select(Simd::splat(1023), Simd::splat(1024));
        let high = Simd::splat(self.max_sample);
        for row in 0..MI_LINES {
            let start = first + row * stride;
            let line = &mut samples[start..start + N];
            let values = E::widen::<N>(line);
            let diff = ((coefficient * Simd::splat(delta[row]) + round) >> 11).cast::<i16>();
            E::narrow((values + diff).simd_max(zero).simd_min(high), line);
        }
    }

    /// Filters `edges` column edges in four-edge chunks, then any last two
    /// (a pair) or one.
    fn column_run<E: EdgeSample>(
        &self,
        samples: &mut [E],
        first: usize,
        stride: usize,
        edges: usize,
    ) {
        let mut done = 0;
        while edges - done >= MI_LINES {
            self.columns4::<E>(samples, first + done * MI_LINES, stride);
            done += MI_LINES;
        }
        let mut pair = done / 2;
        while 2 * (pair + 1) <= edges {
            self.column_pair(samples, first + 2 * pair * MI_LINES, stride);
            pair += 1;
        }
        if !edges.is_multiple_of(2) {
            self.columns(samples, first + (edges - 1) * MI_LINES, stride);
        }
    }

    /// Filters four consecutive column edges side by side at `first`, the
    /// four-edge analogue of [`Self::column_pair`]: the shared sample rows
    /// load once for all four, the widths come from vector compares, and
    /// each distinct chosen width applies in one sixteen-lane pass whose
    /// mask leaves the other edges' samples unchanged.
    fn columns4<E: EdgeSample>(&self, samples: &mut [E], first: usize, stride: usize) {
        let win = &mut samples[first..first + (2 * EDGE_REACH - 1) * stride + 4 * MI_LINES];
        let inner: [Simd<i16, { 4 * MI_LINES }>; 4] = core::array::from_fn(|k| {
            column_row::<E, { 4 * MI_LINES }>(win, stride, k as isize - 2)
        });
        let deltas = inner[0] - inner[3] + (inner[2] - inner[1]) * Simd::splat(3);
        let changed_lanes = deltas
            .abs()
            .simd_gt(Simd::splat(self.noop_delta))
            .to_bitmask();
        let mut changes = 0u8;
        for edge in 0..MI_LINES {
            changes |= (u8::from(changed_lanes >> (4 * edge) & 0xF != 0)) << edge;
        }
        if changes == 0 {
            return;
        }
        let above = column_row::<E, { 4 * MI_LINES }>(win, stride, -3);
        let below = column_row::<E, { 4 * MI_LINES }>(win, stride, 2);
        let rows = [above, inner[0], inner[1], inner[2], inner[3], below];
        let second = |k: usize| (rows[k] - rows[k + 1] - rows[k + 1] + rows[k + 2]).abs();
        let ends = |a: Simd<i16, { 4 * MI_LINES }>, b: Simd<i16, { 4 * MI_LINES }>| {
            simd_swizzle!(
                a,
                b,
                [0, 16, 3, 19, 4, 20, 7, 23, 8, 24, 11, 27, 12, 28, 15, 31]
            )
        };
        let firsts = ends(second(0), second(1)).to_array();
        let lasts = ends(second(2), second(3)).to_array();
        let mut derivatives = [Simd::<i32, MI_LINES>::splat(i32::MAX); 4];
        for edge in 0..MI_LINES {
            let (m2, m1, s0, s1) = (
                i32::from((firsts[4 * edge] + firsts[4 * edge + 2] + 1) >> 1),
                i32::from((firsts[4 * edge + 1] + firsts[4 * edge + 3] + 1) >> 1),
                i32::from((lasts[4 * edge] + lasts[4 * edge + 2] + 1) >> 1),
                i32::from((lasts[4 * edge + 1] + lasts[4 * edge + 3] + 1) >> 1),
            );
            let lanes = derivatives.as_mut();
            (
                lanes[0].as_mut_array()[edge],
                lanes[1].as_mut_array()[edge],
                lanes[2].as_mut_array()[edge],
                lanes[3].as_mut_array()[edge],
            ) = (m2, m1, s0, s1);
        }
        let (mut widths, survivors) = self.cascade_widths(&derivatives, MI_LINES);
        if self.choice.max_width_pos > 2 {
            let mut left = survivors;
            while left != 0 {
                let edge = left.trailing_zeros() as usize;
                left &= left - 1;
                widths[edge] = deblock_filter_choice_cascade(
                    self.choice,
                    [
                        derivatives[0].as_array()[edge],
                        derivatives[1].as_array()[edge],
                        derivatives[2].as_array()[edge],
                        derivatives[3].as_array()[edge],
                    ],
                    || {
                        cascade_reach(|offset| {
                            let values =
                                column_row::<E, MI_LINES>(&win[edge * MI_LINES..], stride, offset);
                            (i32::from(values[0]), i32::from(values[MI_LINES - 1]))
                        })
                    },
                );
            }
        }
        for (edge, width) in widths.iter_mut().enumerate() {
            if changes & (1 << edge) == 0 {
                *width = 0;
            }
        }
        let mut present = 0u8;
        for width in &widths {
            if *width > 0 {
                present |= 1 << (*width - 1);
            }
        }
        let mut left = present;
        while left != 0 {
            let width = left.trailing_zeros() as usize + 1;
            left &= left - 1;
            let mut pass = 0u8;
            for (edge, edge_width) in widths.iter().enumerate() {
                pass |= (u8::from(*edge_width == width)) << edge;
            }
            let mut lane_flags = [false; { 4 * MI_LINES }];
            for (lane, flag) in lane_flags.iter_mut().enumerate() {
                *flag = pass >> (lane / MI_LINES) & 1 != 0;
            }
            let lanes = Mask::<i16, { 4 * MI_LINES }>::from_array(lane_flags);
            let (wn, wp, qtc, wneg, wpos) = self.weights(width);
            let delta = Self::delta_m2(deltas, qtc);
            let zero = Simd::splat(0i16);
            let high = Simd::splat(self.max_sample);
            let mut filter = |offset: isize, coefficient: i32, round: i32| {
                let start = (EDGE_REACH as isize + offset) as usize * stride;
                let line = &mut win[start..start + 4 * MI_LINES];
                let values = E::widen::<{ 4 * MI_LINES }>(line);
                let diff =
                    ((delta * Simd::splat(coefficient) + Simd::splat(round)) >> 11).cast::<i16>();
                let diff = lanes.select(diff, zero);
                E::narrow((values + diff).simd_max(zero).simd_min(high), line);
            };
            if wpos != 0 {
                for i in 0..wp {
                    filter(i as isize, -i32::from(wpos) * (wp - i) as i32, 1023);
                }
            }
            if wneg != 0 {
                for i in 0..wn {
                    filter(-(i as isize) - 1, i32::from(wneg) * (wn - i) as i32, 1024);
                }
            }
        }
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn columns<E: EdgeSample>(&self, samples: &mut [E], first: usize, stride: usize) {
        let samples = &mut samples[first..first + (2 * EDGE_REACH - 1) * stride + MI_LINES];
        let inner: [Simd<i16, MI_LINES>; 4] =
            core::array::from_fn(|k| column_row(samples, stride, k as isize - 2));
        let deltas = inner[0] - inner[3] + (inner[2] - inner[1]) * Simd::splat(3);
        if self.unchanged(deltas) {
            return;
        }
        let (above, below) = (
            column_row(samples, stride, -3),
            column_row(samples, stride, 2),
        );
        let rows = [above, inner[0], inner[1], inner[2], inner[3], below];
        let second = |k: usize| (rows[k] - rows[k + 1] - rows[k + 1] + rows[k + 2]).abs();
        let (d0, d1, d2, d3) = (second(0), second(1), second(2), second(3));
        let pairs =
            |a: Simd<i16, MI_LINES>, b: Simd<i16, MI_LINES>| simd_swizzle!(a, b, [0, 4, 3, 7]);
        let ends = simd_swizzle!(pairs(d0, d1), pairs(d2, d3), [0, 1, 4, 5, 2, 3, 6, 7]);
        let width = self.column_width(samples, stride, combine_line_derivatives(ends));
        if width != 0 {
            self.filter_columns(samples, stride, width, deltas);
        }
    }

    /// [`Self::columns`] for two edges side by side: one eight-lane unchanged
    /// test and, when both edges choose one width, one eight-lane filter. An
    /// edge's choice reads only its own columns, which the other's filter
    /// does not write.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn column_pair<E: EdgeSample>(&self, samples: &mut [E], first: usize, stride: usize) {
        let samples = &mut samples[first..first + (2 * EDGE_REACH - 1) * stride + 2 * MI_LINES];
        let inner: [Simd<i16, 8>; 4] =
            core::array::from_fn(|k| column_row(samples, stride, k as isize - 2));
        let deltas = inner[0] - inner[3] + (inner[2] - inner[1]) * Simd::splat(3);
        let size = deltas.abs();
        if size.reduce_max() <= self.noop_delta {
            return;
        }
        let (above, below) = (
            column_row(samples, stride, -3),
            column_row(samples, stride, 2),
        );
        let rows = [above, inner[0], inner[1], inner[2], inner[3], below];
        let second = |k: usize| (rows[k] - rows[k + 1] - rows[k + 1] + rows[k + 2]).abs();
        let (d0, d1, d2, d3) = (second(0), second(1), second(2), second(3));
        let p01 = simd_swizzle!(d0, d1, [0, 8, 3, 11, 4, 12, 7, 15]);
        let p23 = simd_swizzle!(d2, d3, [0, 8, 3, 11, 4, 12, 7, 15]);
        let firsts = simd_swizzle!(p01, p23, [0, 1, 8, 9, 4, 5, 12, 13]);
        let lasts = simd_swizzle!(p01, p23, [2, 3, 10, 11, 6, 7, 14, 15]);
        let combined = ((firsts + lasts + Simd::splat(1)) >> 1).to_array();
        let halves = |v: Simd<i16, 8>| {
            [
                simd_swizzle!(v, [0, 1, 2, 3]),
                simd_swizzle!(v, [4, 5, 6, 7]),
            ]
        };
        let sizes = halves(size);
        let widths: [usize; 2] = core::array::from_fn(|half| {
            if sizes[half].reduce_max() <= self.noop_delta {
                return 0;
            }
            let derivatives = core::array::from_fn(|k| i32::from(combined[half * MI_LINES + k]));
            self.column_width(&samples[half * MI_LINES..], stride, derivatives)
        });
        if widths[0] == widths[1] {
            if widths[0] != 0 {
                self.filter_columns(samples, stride, widths[0], deltas);
            }
            return;
        }
        for (half, width) in widths.into_iter().enumerate() {
            if width != 0 {
                let samples = &mut samples[half * MI_LINES..];
                self.filter_columns(samples, stride, width, halves(deltas)[half]);
            }
        }
    }

    /// The § 7.17.7.2 width of the column edge whose first line is column 0.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn column_width<E: EdgeSample>(
        &self,
        samples: &[E],
        stride: usize,
        derivatives: [i32; 4],
    ) -> usize {
        deblock_filter_choice_cascade(self.choice, derivatives, || {
            cascade_reach(|offset| {
                let values = column_row::<E, MI_LINES>(samples, stride, offset);
                (i32::from(values[0]), i32::from(values[MI_LINES - 1]))
            })
        })
    }

    /// Applies § 7.17.7.1 to `N` columns at `width` across the edge.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn filter_columns<E: EdgeSample, const N: usize>(
        &self,
        samples: &mut [E],
        stride: usize,
        width: usize,
        deltas: Simd<i16, N>,
    ) {
        let (width_neg, width_pos, q_thr_clamp, w_neg, w_pos) = self.weights(width);
        let delta = Self::delta_m2(deltas, q_thr_clamp);
        let (zero, high) = (Simd::splat(0), Simd::splat(self.max_sample));
        let mut filter = |offset: isize, coefficient: i32, round: i32| {
            let start = (EDGE_REACH as isize + offset) as usize * stride;
            let line = &mut samples[start..start + N];
            let values = E::widen::<N>(line);
            let diff =
                ((delta * Simd::splat(coefficient) + Simd::splat(round)) >> 11).cast::<i16>();
            E::narrow((values + diff).simd_max(zero).simd_min(high), line);
        };
        if w_pos != 0 {
            for i in 0..width_pos {
                filter(i as isize, -i32::from(w_pos) * (width_pos - i) as i32, 1023);
            }
        }
        if w_neg != 0 {
            for i in 0..width_neg {
                filter(
                    -(i as isize) - 1,
                    i32::from(w_neg) * (width_neg - i) as i32,
                    1024,
                );
            }
        }
    }
}

/// The `p1 - q1 + 3 * (q0 - p0)` of the four lines of one row edge, held in
/// lanes.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn row_line_deltas(
    s: Simd<i16, { 2 * EDGE_REACH }>,
    u: Simd<i16, { 2 * EDGE_REACH }>,
    v: Simd<i16, { 2 * EDGE_REACH }>,
    t: Simd<i16, { 2 * EDGE_REACH }>,
) -> Simd<i16, MI_LINES> {
    let taps = Simd::from_array([1, -3, 3, -1, 1, -3, 3, -1]);
    let top = simd_swizzle!(s, u, [6, 7, 8, 9, 22, 23, 24, 25]) * taps;
    let bottom = simd_swizzle!(v, t, [6, 7, 8, 9, 22, 23, 24, 25]) * taps;
    let pairs = simd_swizzle!(top, bottom, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(top, bottom, [1, 3, 5, 7, 9, 11, 13, 15]);
    simd_swizzle!(pairs, [0, 2, 4, 6]) + simd_swizzle!(pairs, [1, 3, 5, 7])
}

/// `secondDeriv[-2..=1]` of one row edge's first and last lines, per the
/// § 7.17.7.2 cascade's second-derivative stage.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn row_second_derivatives(
    s: Simd<i16, { 2 * EDGE_REACH }>,
    t: Simd<i16, { 2 * EDGE_REACH }>,
) -> [i32; 4] {
    let centre = simd_swizzle!(s, t, [6, 7, 8, 9, 22, 23, 24, 25]);
    let left = simd_swizzle!(s, t, [5, 6, 7, 8, 21, 22, 23, 24]);
    let right = simd_swizzle!(s, t, [7, 8, 9, 10, 23, 24, 25, 26]);
    combine_line_derivatives((left - centre - centre + right).abs())
}

/// The [`CascadeDerivatives`] of one edge's first and last sample rows, held
/// in registers: the neg-side pair for tap `n` reads the edge's `m1`, `m2`
/// and the sample `n + 1` rows before it, the pos-side pair its `q0`, `p1`
/// and the sample `n` rows past it.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn row_cascade_derivatives(s: Simd<i16, 16>, t: Simd<i16, 16>) -> CascadeDerivatives {
    let taps = Simd::from_array([3, 4, 6, 7]);
    let neg = |line: Simd<i16, 16>| {
        let edge = simd_swizzle!(line, [7, 7, 7, 7]);
        let inner = simd_swizzle!(line, [6, 6, 6, 6]);
        let outer = simd_swizzle!(line, [4, 3, 1, 0]);
        (edge - outer - (edge - inner) * taps).abs()
    };
    let pos = |line: Simd<i16, 16>| {
        let edge = simd_swizzle!(line, [8, 8, 8, 8]);
        let inner = simd_swizzle!(line, [9, 9, 9, 9]);
        let outer = simd_swizzle!(line, [11, 12, 14, 15]);
        (edge - outer - (edge - inner) * taps).abs()
    };
    let neg = (neg(s).cast::<i32>() + neg(t).cast::<i32>() + Simd::splat(1)) >> Simd::splat(1);
    let pos = (pos(s).cast::<i32>() + pos(t).cast::<i32>() + Simd::splat(1)) >> Simd::splat(1);
    core::array::from_fn(|index| (neg[index], pos[index]))
}

/// Reads `N` samples of the line `offset` rows from the edge of a column
/// edge window.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn column_row<E: EdgeSample, const N: usize>(
    samples: &[E],
    stride: usize,
    offset: isize,
) -> Simd<i16, N> {
    let start = (EDGE_REACH as isize + offset) as usize * stride;
    E::widen::<N>(&samples[start..start + N])
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "shared fused deblock hot path")]
#[inline(always)]
fn apply_deblock_choice_strided_4<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    lane_stride: usize,
    choice: &DeblockFilterChoice,
    q_thresh_mults: &[i32; MAX_DBL_FLT_LEN],
    w_mults: &[i32; MAX_DBL_FLT_LEN],
    prev_lossless: bool,
    curr_lossless: bool,
    bit_depth: BitDepth,
    width: usize,
) -> Result<usize> {
    if width == 0 {
        return Ok(0);
    }
    let max_width_neg = width.min(choice.max_width_neg);
    let max_width_pos = width.min(choice.max_width_pos);
    let max_width = max_width_neg.max(max_width_pos);
    let params = DeblockSampleFilter {
        boundary: choice.boundary,
        q_thr: choice.q_thr,
        max_width_neg,
        max_width_pos,
        q_thresh_mult: q_thresh_mults[max_width - 1],
        w_mult_neg: w_mults[max_width_neg - 1],
        w_mult_pos: w_mults[max_width_pos - 1],
        prev_lossless,
        curr_lossless,
        bit_depth,
    };
    validate_sample_type::<T>(bit_depth)?;
    deblock_sample_filter_strided_4_validated(samples, stride, lane_stride, &params)?;
    Ok(width)
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_filter_choice_progressive(
    params: &DeblockFilterChoice,
    mut load: impl FnMut(isize) -> (i32, i32),
) -> usize {
    let (m3, m2, m1) = (load(-3), load(-2), load(-1));
    let (zero, p1, p2) = (load(0), load(1), load(2));
    let derivatives = [
        choice_second_deriv(m3, m2, m1),
        choice_second_deriv(m2, m1, zero),
        choice_second_deriv(m1, zero, p1),
        choice_second_deriv(zero, p1, p2),
    ];
    deblock_filter_choice_cascade(params, derivatives, || cascade_reach(&mut load))
}

/// The directional derivative pairs the § 7.17.7.2 cascade consumes past its
/// second-derivative stage, indexed by tap `n`: lane 0 holds the pre-loop
/// `n = 3` pair and lanes 1..4 the `n = 4, 6, 7` pairs the widths
/// `dist = 4, 6, 8` read (`n = Min(dist, 7)`).
type CascadeDerivatives = [(i32, i32); 4];

/// The [`CascadeDerivatives`] a `load` closure produces over the two lines'
/// samples at an offset from the edge. Only the cascade's wide stage calls
/// this, so an edge that stops at a narrower width never reads them.
fn cascade_reach(mut load: impl FnMut(isize) -> (i32, i32)) -> CascadeDerivatives {
    let (m2, m1, zero, p1) = (load(-2), load(-1), load(0), load(1));
    [
        (
            choice_directional(m1, load(-4), m2, 3),
            choice_directional(zero, load(3), p1, 3),
        ),
        (
            choice_directional(m1, load(-5), m2, 4),
            choice_directional(zero, load(4), p1, 4),
        ),
        (
            choice_directional(m1, load(-7), m2, 6),
            choice_directional(zero, load(6), p1, 6),
        ),
        (
            choice_directional(m1, load(-8), m2, 7),
            choice_directional(zero, load(7), p1, 7),
        ),
    ]
}

/// The § 7.17.7.2 threshold cascade over `secondDeriv[-2..=1]`. `reach`
/// produces the directional derivatives of the wide stage; it runs only when
/// the cascade gets there, so an edge that stops earlier never pays for it.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_filter_choice_cascade(
    params: &DeblockFilterChoice,
    [sd_m2, sd_m1, sd_0, sd_1]: [i32; 4],
    reach: impl FnOnce() -> CascadeDerivatives,
) -> usize {
    let DeblockFilterChoice {
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        ..
    } = *params;
    let max_outer_deriv = sd_m2.max(sd_1);
    if max_outer_deriv > side_thr {
        return 0;
    }
    if max_width_pos == 1 {
        return 1;
    }
    let side_thr2 = side_thr >> 2;
    if max_outer_deriv > side_thr2 || sd_m1 + sd_0 > q_thr * 4 {
        return 1;
    }
    let side_thr3 = side_thr >> 3;
    if max_outer_deriv > side_thr3 || sd_m1 + sd_0 > q_thr * 3 {
        return 2;
    }

    let [dir3, wide0, wide1, wide2] = reach();
    let end_thr = (side_thr * 3) >> 4;
    if max_width_neg > 2 && dir3.0 > end_thr {
        return 2;
    }
    if dir3.1 > end_thr {
        return 2;
    }
    if max_width_pos == 3 {
        return 3;
    }
    deblock_filter_choice_wide(params, sd_m1 + sd_0, [wide0, wide1, wide2])
}

/// The widths past 3 of [`deblock_filter_choice_cascade`]. `wide` holds the
/// directional pairs for `dist = 4, 6, 8`; indexes past `max_width_pos` are
/// never read. Kept out of line: inlined, its per-width threshold products
/// were hoisted into the setup of every edge run, which most runs never
/// reach.
#[inline(never)]
fn deblock_filter_choice_wide(
    params: &DeblockFilterChoice,
    inner: i32,
    wide: [(i32, i32); 3],
) -> usize {
    let DeblockFilterChoice {
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        q_first,
        ..
    } = *params;
    let transition = inner << 4;
    let mut prev_dist = 3usize;
    for (index, dist) in [4usize, 6, 8].into_iter().enumerate() {
        if dist > max_width_pos {
            break;
        }
        let q_thr4 = q_thr.saturating_mul(q_first[dist - 4]);
        let end_thr4 = side_thr.saturating_mul(dist as i32) >> 4;
        if transition > q_thr4 {
            return prev_dist;
        }
        let dist2 = dist.min(7);
        if max_width_neg >= dist2 && wide[index].0 > end_thr4 {
            return prev_dist;
        }
        if wide[index].1 > end_thr4 {
            return prev_dist;
        }
        prev_dist = dist;
    }
    max_width_pos
}

#[inline]
fn choice_second_deriv(left: (i32, i32), center: (i32, i32), right: (i32, i32)) -> i32 {
    let deriv_s = (left.0 - (center.0 << 1) + right.0).abs();
    let deriv_t = (left.1 - (center.1 << 1) + right.1).abs();
    (deriv_s + deriv_t + 1) >> 1
}

#[inline]
fn choice_directional(i: (i32, i32), j: (i32, i32), g: (i32, i32), n: i32) -> i32 {
    let deriv_s = (i.0 - j.0 - n * (i.0 - g.0)).abs();
    let deriv_t = (i.1 - j.1 - n * (i.1 - g.1)).abs();
    (deriv_s + deriv_t + 1) >> 1
}

#[cfg(test)]
#[path = "deblock_filter_tests.rs"]
mod tests;
