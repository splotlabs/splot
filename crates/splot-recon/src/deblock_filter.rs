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
//! segment, or tile state or wire into the runtime decode path.

use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::math::round2_i32;
use crate::{BitDepth, ReconError, ReconSample, Result};
use core::num::NonZeroUsize;

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
#[inline]
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
        deblock_sample_filter_inner_4_bounded(samples, params, stride, lane_stride.get())
    } else {
        deblock_sample_filter_inner_lanes(samples, params, stride, lane_stride.get(), 4)
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
#[inline]
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

#[inline]
fn deblock_filter_choice_progressive(
    params: &DeblockFilterChoice,
    mut load: impl FnMut(isize) -> (i32, i32),
) -> usize {
    let DeblockFilterChoice {
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        q_first,
        ..
    } = *params;
    let m3 = load(-3);
    let m2 = load(-2);
    let m1 = load(-1);
    let zero = load(0);
    let p1 = load(1);
    let p2 = load(2);
    let sd_m2 = choice_second_deriv(m3, m2, m1);
    let sd_m1 = choice_second_deriv(m2, m1, zero);
    let sd_0 = choice_second_deriv(m1, zero, p1);
    let sd_1 = choice_second_deriv(zero, p1, p2);
    if sd_m2 > side_thr || sd_1 > side_thr {
        return 0;
    }
    if max_width_pos == 1 {
        return 1;
    }
    let side_thr2 = side_thr >> 2;
    if sd_m2 > side_thr2 || sd_1 > side_thr2 || sd_m1 + sd_0 > q_thr * 4 {
        return 1;
    }
    let side_thr3 = side_thr >> 3;
    if sd_m2 > side_thr3 || sd_1 > side_thr3 || sd_m1 + sd_0 > q_thr * 3 {
        return 2;
    }

    let end_thr = (side_thr * 3) >> 4;
    if max_width_neg > 2 && choice_directional(m1, load(-4), m2, 3) > end_thr {
        return 2;
    }
    if choice_directional(zero, load(3), p1, 3) > end_thr {
        return 2;
    }
    if max_width_pos == 3 {
        return 3;
    }
    let transition = (sd_m1 + sd_0) << 4;
    let mut prev_dist = 3usize;
    let mut dist = 4usize;
    while dist <= max_width_pos {
        let q_thr4 = q_thr.saturating_mul(q_first[dist - 4]);
        let end_thr4 = side_thr.saturating_mul(dist as i32) >> 4;
        if transition > q_thr4 {
            return prev_dist;
        }
        let dist2 = dist.min(7);
        let n = dist2 as i32;
        if max_width_neg >= dist2
            && choice_directional(m1, load(-(n as isize + 1)), m2, n) > end_thr4
        {
            return prev_dist;
        }
        if choice_directional(zero, load(n as isize), p1, n) > end_thr4 {
            return prev_dist;
        }
        prev_dist = dist;
        dist += 2;
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::math::round2;

    #[allow(clippy::too_many_arguments)]
    fn params(
        boundary: usize,
        q_thr: i32,
        max_width_neg: usize,
        max_width_pos: usize,
        q_thresh_mult: i32,
        w_mult_neg: i32,
        w_mult_pos: i32,
        prev_lossless: bool,
        curr_lossless: bool,
    ) -> DeblockSampleFilter {
        DeblockSampleFilter {
            boundary,
            q_thr,
            max_width_neg,
            max_width_pos,
            q_thresh_mult,
            w_mult_neg,
            w_mult_pos,
            prev_lossless,
            curr_lossless,
            bit_depth: BitDepth::Eight,
        }
    }

    fn reference(line: &mut [u8], p: &DeblockSampleFilter) {
        let width = p.max_width_neg.max(p.max_width_pos);
        let q0 = i64::from(line[p.boundary]);
        let q1 = i64::from(line[p.boundary + 1]);
        let p0 = i64::from(line[p.boundary - 1]);
        let p1 = i64::from(line[p.boundary - 2]);
        let bound = (i64::from(p.q_thr) * i64::from(p.q_thresh_mult)).max(0);
        let delta = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-bound, bound);
        let dn = delta * i64::from(p.w_mult_neg);
        let dp = delta * i64::from(p.w_mult_pos);
        let max = i64::from(BitDepth::Eight.max_sample());
        for i in 0..width {
            let diff_pos = round2(dp * (p.max_width_pos as i64 - i as i64), 3 + DF_SHIFT);
            if !p.curr_lossless {
                let idx = p.boundary + i;
                line[idx] = (i64::from(line[idx]) - diff_pos).clamp(0, max) as u8;
            }
            if i < p.max_width_neg && !p.prev_lossless {
                let diff_neg = round2(dn * (p.max_width_neg as i64 - i as i64), 3 + DF_SHIFT);
                let idx = p.boundary - 1 - i;
                line[idx] = (i64::from(line[idx]) + diff_neg).clamp(0, max) as u8;
            }
        }
    }

    #[test]
    fn matches_hand_computed_symmetric_width_2() {
        let mut line = [10u8, 20, 60, 50];
        deblock_sample_filter(&mut line, &params(2, 100, 2, 2, 25, 51, 51, false, false)).unwrap();
        assert_eq!(line, [18, 36, 44, 42]);
    }

    #[test]
    fn matches_reference_across_configs() {
        let base = [40u8, 60, 50, 70, 55, 80, 45, 90, 35, 100];
        let configs = [
            params(4, 80, 3, 2, 19, 37, 51, false, false),
            params(4, 200, 2, 3, 19, 51, 37, false, false), // maxWidthPos > maxWidthNeg
            params(5, 20, 4, 4, 19, 28, 28, false, false),  // small q_thr clamps deltaM2
            params(4, 80, 2, 2, 25, 51, 51, true, false),   // prev lossless: p-side skipped
            params(4, 80, 2, 2, 25, 51, 51, false, true),   // curr lossless: q-side skipped
        ];
        for p in &configs {
            let mut produced = base;
            deblock_sample_filter(&mut produced, p).unwrap();
            let mut expected = base;
            reference(&mut expected, p);
            assert_eq!(produced, expected, "config {p:?}");
        }
    }

    #[test]
    fn strided_sample_filter_matches_contiguous_line() {
        let source = [
            40u16, 60, 50, 70, 55, 80, 45, 90, 35, 100, 30, 110, 25, 120, 20, 130, 15,
        ];
        let params = DeblockSampleFilter {
            boundary: 8,
            bit_depth: BitDepth::Ten,
            ..params(8, 80, 6, 8, 17, 20, 15, false, false)
        };
        let mut expected = source;
        deblock_sample_filter(&mut expected, &params).unwrap();

        let stride = 23;
        let boundary = 8 * stride + 4;
        let mut plane = vec![0u16; 17 * stride];
        for (index, sample) in source.into_iter().enumerate() {
            let position = if index < 8 {
                boundary - (8 - index) * stride
            } else {
                boundary + (index - 8) * stride
            };
            plane[position] = sample;
        }
        deblock_sample_filter_strided(
            &mut plane,
            NonZeroUsize::new(stride).unwrap(),
            &DeblockSampleFilter { boundary, ..params },
        )
        .unwrap();
        for (index, expected) in expected.into_iter().enumerate() {
            let position = if index < 8 {
                boundary - (8 - index) * stride
            } else {
                boundary + (index - 8) * stride
            };
            assert_eq!(plane[position], expected);
        }
    }

    #[test]
    fn four_lane_strided_filter_matches_individual_lanes() {
        let stride = 32;
        let boundary = 8 * stride + 4;
        let mut source = vec![0u16; 17 * stride];
        for row in 0..17 {
            for lane in 0..4 {
                source[row * stride + 4 + lane] = (40 + row * 7 + lane * 3) as u16;
            }
        }
        let cases = [
            params(boundary, 80, 6, 8, 17, 20, 15, false, false),
            params(boundary, i32::MAX, 6, 8, i32::MAX, 0, 0, false, false),
            params(
                boundary,
                i32::MAX,
                6,
                8,
                i32::MAX,
                i32::MAX,
                i32::MAX,
                false,
                false,
            ),
        ];
        for params in cases {
            let mut expected = source.clone();
            for lane in 0..4 {
                deblock_sample_filter_strided(
                    &mut expected,
                    NonZeroUsize::new(stride).unwrap(),
                    &DeblockSampleFilter {
                        boundary: boundary + lane,
                        ..params
                    },
                )
                .unwrap();
            }
            let mut actual = source.clone();
            deblock_sample_filter_strided_4(
                &mut actual,
                NonZeroUsize::new(stride).unwrap(),
                NonZeroUsize::new(1).unwrap(),
                &params,
            )
            .unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn lossless_sides_are_untouched() {
        let base = [40u8, 60, 50, 70, 55, 80];
        let mut both = base;
        deblock_sample_filter(&mut both, &params(3, 100, 2, 2, 25, 51, 51, true, true)).unwrap();
        assert_eq!(both, base);
    }

    #[test]
    fn clip1_clamps_to_bit_depth() {
        let mut line = [10u8, 250, 5, 240, 0, 255];
        deblock_sample_filter(
            &mut line,
            &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
        )
        .unwrap();
        let mut reference_line = [10u8, 250, 5, 240, 0, 255];
        reference(
            &mut reference_line,
            &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
        );
        assert_eq!(line, reference_line);
        let mut wide = [10u16, 1000, 5, 1020, 0, 1023];
        deblock_sample_filter(
            &mut wide,
            &DeblockSampleFilter {
                bit_depth: BitDepth::Ten,
                ..params(3, 10_000, 2, 2, 32, 85, 85, false, false)
            },
        )
        .unwrap();
        assert!(wide.iter().all(|&v| v <= 1023));
    }

    #[test]
    fn rejects_invalid_width_and_short_line() {
        let mut line = [0u8; 8];
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(4, 0, 0, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterInvalidWidth {
                max_width_neg: 0,
                max_width_pos: 2
            })
        ));
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(4, 0, 2, 9, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterInvalidWidth { .. })
        ));
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(1, 0, 2, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterLineTooShort { .. })
        ));
        let mut short = [0u8; 5];
        assert!(matches!(
            deblock_sample_filter(&mut short, &params(4, 0, 2, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterLineTooShort { .. })
        ));
    }

    #[test]
    fn is_total_for_extreme_inputs() {
        for &(neg, pos) in &[(1usize, 8usize), (8, 1), (8, 8)] {
            let mut line = [0u8; 24];
            for (i, s) in line.iter_mut().enumerate() {
                *s = if i % 2 == 0 { 0 } else { 255 };
            }
            deblock_sample_filter(
                &mut line,
                &params(10, i32::MAX, neg, pos, 32, 85, 85, false, false),
            )
            .unwrap();
        }
    }

    #[test]
    fn max_width_covers_every_spec_branch() {
        assert_eq!(deblock_filter_max_width(4, false, false), (1, 1));
        assert_eq!(deblock_filter_max_width(2, true, false), (1, 1)); // <= 4
        assert_eq!(deblock_filter_max_width(8, false, false), (3, 3));
        assert_eq!(deblock_filter_max_width(8, true, false), (3, 3));
        assert_eq!(deblock_filter_max_width(16, false, false), (6, 6)); // luma
        assert_eq!(deblock_filter_max_width(16, true, false), (4, 4)); // chroma
        assert_eq!(deblock_filter_max_width(32, false, false), (8, 8)); // luma > 16
        assert_eq!(deblock_filter_max_width(64, true, false), (4, 4)); // chroma > 16

        assert_eq!(deblock_filter_max_width(32, false, true), (6, 8)); // luma: cap 6
        assert_eq!(deblock_filter_max_width(64, true, true), (2, 4)); // chroma: cap 2
        assert_eq!(deblock_filter_max_width(8, true, true), (2, 3)); // chroma: cap 2 < 3
        assert_eq!(deblock_filter_max_width(4, false, true), (1, 1)); // cap 6 but pos 1
    }

    #[test]
    fn side_threshold_index_clips_to_table_range() {
        assert_eq!(deblock_side_threshold_index(10, BitDepth::Eight), 10);
        assert_eq!(deblock_side_threshold_index(0, BitDepth::Eight), 0);
        assert_eq!(deblock_side_threshold_index(400, BitDepth::Eight), 295);
        assert_eq!(deblock_side_threshold_index(10, BitDepth::Ten), 0); // 10 - 48 < 0
        assert_eq!(deblock_side_threshold_index(100, BitDepth::Ten), 52); // 100 - 48
    }

    #[test]
    fn adaptive_filter_strength_matches_spec() {
        assert_eq!(
            deblock_adaptive_filter_strength(40, 100, BitDepth::Eight).1,
            3
        ); // (100+16)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, -16, BitDepth::Eight).1,
            0
        ); // (0)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, 1678, BitDepth::Eight).1,
            52
        ); // (1694)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, 100, BitDepth::Ten).1,
            13
        ); // (104)>>3

        for &(lvl, bd) in &[
            (40u32, BitDepth::Eight),
            (128, BitDepth::Eight),
            (200, BitDepth::Ten),
        ] {
            let expected = (((i64::from(quantizer_value(lvl, 0, bd)) + 4) >> 3) >> 6) as i32;
            assert_eq!(deblock_adaptive_filter_strength(lvl, 0, bd).0, expected);
        }
    }

    /// The § 9.2 `Q_First` array (`docs/spec/.../03-symbols.md`, DBL_REG_DECIS_LEN).
    const Q_FIRST: [i32; DBL_REG_DECIS_LEN] = [45, 43, 40, 35, 32, 32, 32, 32, 32];

    fn choice(
        boundary: usize,
        q_thr: i32,
        side_thr: i32,
        max_width_neg: usize,
        max_width_pos: usize,
    ) -> DeblockFilterChoice {
        DeblockFilterChoice {
            boundary,
            q_thr,
            side_thr,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reference_choice(
        s: &[u16],
        t: &[u16],
        boundary: usize,
        q_thr: i64,
        side_thr: i64,
        max_width_neg: usize,
        max_width_pos: usize,
        q_first: &[i32; DBL_REG_DECIS_LEN],
    ) -> usize {
        if q_thr == 0 || side_thr == 0 {
            return 0;
        }
        let g = |a: &[u16], k: i64| -> i64 { i64::from(a[(boundary as i64 + k) as usize]) };
        let mut sd = [0i64; 4]; // index dist + 2, for dist in -2..=1
        for dist in -2i64..=1 {
            let ds = (g(s, dist - 1) - 2 * g(s, dist) + g(s, dist + 1)).abs();
            let dt = (g(t, dist - 1) - 2 * g(t, dist) + g(t, dist + 1)).abs();
            sd[(dist + 2) as usize] = (ds + dt + 1) >> 1;
        }
        let sdv = |d: i64| sd[(d + 2) as usize];
        if sdv(-2) > side_thr || sdv(1) > side_thr {
            return 0;
        }
        if max_width_pos == 1 {
            return 1;
        }
        if sdv(-2) > (side_thr >> 2) || sdv(1) > (side_thr >> 2) {
            return 1;
        }
        if sdv(-1) + sdv(0) > q_thr * 4 {
            return 1;
        }
        if sdv(-2) > (side_thr >> 3) || sdv(1) > (side_thr >> 3) {
            return 2;
        }
        if sdv(-1) + sdv(0) > q_thr * 3 {
            return 2;
        }
        let end_thr = (side_thr * 3) >> 4;
        if max_width_neg > 2 {
            let ds = (g(s, -1) - g(s, -4) - 3 * (g(s, -1) - g(s, -2))).abs();
            let dt = (g(t, -1) - g(t, -4) - 3 * (g(t, -1) - g(t, -2))).abs();
            if ((ds + dt + 1) >> 1) > end_thr {
                return 2;
            }
        }
        let ds = (g(s, 0) - g(s, 3) - 3 * (g(s, 0) - g(s, 1))).abs();
        let dt = (g(t, 0) - g(t, 3) - 3 * (g(t, 0) - g(t, 1))).abs();
        if ((ds + dt + 1) >> 1) > end_thr {
            return 2;
        }
        if max_width_pos == 3 {
            return 3;
        }
        let transition = (sdv(-1) + sdv(0)) << 4;
        let mut prev_dist = 3usize;
        let mut dist = 4usize;
        while dist <= max_width_pos {
            let q_thr4 = q_thr * i64::from(q_first[dist - 4]);
            let end_thr4 = (side_thr * dist as i64) >> 4;
            if transition > q_thr4 {
                return prev_dist;
            }
            let dist2 = dist.min(7) as i64;
            if max_width_neg >= dist2 as usize {
                let ds = (g(s, -1) - g(s, -dist2 - 1) - dist2 * (g(s, -1) - g(s, -2))).abs();
                let dt = (g(t, -1) - g(t, -dist2 - 1) - dist2 * (g(t, -1) - g(t, -2))).abs();
                if ((ds + dt + 1) >> 1) > end_thr4 {
                    return prev_dist;
                }
            }
            let ds = (g(s, 0) - g(s, dist2) - dist2 * (g(s, 0) - g(s, 1))).abs();
            let dt = (g(t, 0) - g(t, dist2) - dist2 * (g(t, 0) - g(t, 1))).abs();
            if ((ds + dt + 1) >> 1) > end_thr4 {
                return prev_dist;
            }
            prev_dist = dist;
            dist += 2;
        }
        max_width_pos
    }

    #[test]
    fn filter_choice_zero_threshold_returns_zero() {
        let line = [128u16; 17];
        assert_eq!(
            deblock_filter_choice(&line, &line, &choice(8, 0, 500, 8, 8)).unwrap(),
            0
        );
        assert_eq!(
            deblock_filter_choice(&line, &line, &choice(8, 50, 0, 8, 8)).unwrap(),
            0
        );
    }

    #[test]
    fn filter_choice_flat_returns_full_width() {
        let line = [128u16; 17];
        for pos in [1usize, 3, 4, 6, 8] {
            let neg = pos.min(6);
            let got = deblock_filter_choice(&line, &line, &choice(8, 10, 2000, neg, pos)).unwrap();
            assert_eq!(got, pos, "flat width pos={pos}");
        }
    }

    #[test]
    fn filter_choice_high_curvature_returns_zero() {
        let mut line = [128u16; 17];
        line[8 - 2] = 320; // boundary = 8 -> index 6 is s[-2]
        assert_eq!(
            deblock_filter_choice(&line, &line, &choice(8, 50, 100, 8, 8)).unwrap(),
            0
        );
    }

    #[test]
    fn filter_choice_width_three_flat_returns_three() {
        let line = [128u16; 17];
        assert_eq!(
            deblock_filter_choice(&line, &line, &choice(8, 10, 2000, 3, 3)).unwrap(),
            3
        );
    }

    #[test]
    fn filter_choice_invalid_width_and_short_line_error() {
        let line = [128u16; 17];
        assert!(matches!(
            deblock_filter_choice(&line, &line, &choice(8, 50, 500, 8, 9)),
            Err(ReconError::DeblockFilterInvalidWidth { .. })
        ));
        let short = [128u16; 10];
        assert!(matches!(
            deblock_filter_choice(&short, &short, &choice(8, 50, 500, 8, 8)),
            Err(ReconError::DeblockFilterLineTooShort { .. })
        ));
    }

    #[test]
    fn filter_choice_matches_independent_reference() {
        let mut state = 0x0123_4567_89ab_cdefu64;
        let mut next = |bound: u32| -> u32 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as u32) % bound
        };
        let widths = [1usize, 2, 3, 4, 6, 8];
        let thresholds = [0i32, 5, 50, 500, 5000];
        let mut checked = 0u32;
        for _ in 0..4000 {
            let s: Vec<u16> = (0..17).map(|_| next(512) as u16).collect();
            let t: Vec<u16> = (0..17).map(|_| next(512) as u16).collect();
            let max_width_pos = widths[next(widths.len() as u32) as usize];
            let neg_choice = widths[next(widths.len() as u32) as usize];
            let max_width_neg = neg_choice.min(max_width_pos).max(1);
            let q_thr = thresholds[next(thresholds.len() as u32) as usize];
            let side_thr = thresholds[next(thresholds.len() as u32) as usize];
            let params = choice(8, q_thr, side_thr, max_width_neg, max_width_pos);
            let got = deblock_filter_choice(&s, &t, &params).unwrap();
            let expected = reference_choice(
                &s,
                &t,
                8,
                i64::from(q_thr),
                i64::from(side_thr),
                max_width_neg,
                max_width_pos,
                &Q_FIRST,
            );
            assert_eq!(
                got, expected,
                "pos={max_width_pos} neg={max_width_neg} q={q_thr} side={side_thr} s={s:?} t={t:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 4000);
    }

    #[test]
    fn strided_filter_choice_matches_contiguous_lines() {
        let s = [
            40u16, 60, 50, 70, 55, 80, 45, 90, 35, 100, 30, 110, 25, 120, 20, 130, 15,
        ];
        let t = [
            42u16, 58, 53, 68, 57, 77, 49, 86, 39, 96, 34, 106, 29, 116, 24, 126, 19,
        ];
        let params = choice(8, 80, 200, 6, 8);
        let expected = deblock_filter_choice(&s, &t, &params).unwrap();

        let stride = 23;
        let boundary = 8 * stride + 4;
        let last_boundary = boundary + 3;
        let mut plane = vec![0u16; 17 * stride];
        for index in 0..s.len() {
            let offset = if index < 8 {
                -((8 - index) as isize)
            } else {
                (index - 8) as isize
            };
            let row = boundary
                .checked_add_signed(offset * stride as isize)
                .unwrap();
            plane[row] = s[index];
            plane[row + 3] = t[index];
        }
        let got = deblock_filter_choice_strided(
            &plane,
            last_boundary,
            NonZeroUsize::new(stride).unwrap(),
            &DeblockFilterChoice { boundary, ..params },
        )
        .unwrap();
        assert_eq!(got, expected);
    }
}
