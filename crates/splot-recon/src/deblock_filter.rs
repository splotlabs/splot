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

mod edges;

pub use edges::{DeblockEdge, deblock_horizontal_edges, deblock_vertical_edges};

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
    deblock_sample_filter_inner_lanes(samples, params, stride, lane_stride.get(), 4)
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
