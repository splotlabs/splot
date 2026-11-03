// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.19 block-warp and § 7.13.3.20 extended-warp prediction kernels.
//!
//! This module implements the narrow source-backed single-reference block-warp
//! predictor used after the caller has already decoded `IsWarp` / `WarpMv` and
//! derived the six affine warp parameters. It owns the § 7.13.3.19 8x8
//! convolution, the scaled and unscaled § 7.13.3.20 extended-warp paths, and
//! the § 7.13.3.21 setup-shear check; model derivation, motion mode selection,
//! `SubMvs` storage, compound blending, and decode entropy remain caller
//! responsibilities.
//!
//! Feature tracking: `DECODE-FIRST-INTER-FRAME-FRONTIER`,
//! `DECODE-INTER-COMPOUND-LOCALWARP`.

use splot_core::tables::warp_filter::{EXT_WARPED_FILTERS, WARPED_FILTERS};
use std::simd::{
    Simd, SimdElement, ToBytes,
    cmp::SimdOrd,
    num::{SimdInt, SimdUint},
    simd_swizzle,
};

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2;
use crate::math::{round2_i32, round2_signed, round2_signed_i32};
use crate::subpel_mc::{ReferencePlaneView, reference_lanes};

/// AV2 § 7.13.3.19 block-warp predictor side length in samples.
pub const WARPED_BLOCK_SIZE: usize = 8;

const WARPEDMODEL_PREC_BITS: u32 = 16;
const REF_SCALE_SHIFT: u32 = 14;
const SCALE_SUBPEL_BITS: u32 = 10;
const WARPEDDIFF_PREC_BITS: u32 = 10;
const WARP_PARAM_REDUCE_BITS: u32 = 6;
const WARPEDPIXEL_PREC_SHIFTS: i32 = 1 << 6;
const WARP_FILTER_CENTER: i32 = 3 * WARPEDPIXEL_PREC_SHIFTS;
const INTER_ROUND0: u32 = 3;
const INTER_ROUND1_NON_COMPOUND: u32 = 11;
const INTER_ROUND1_COMPOUND: u32 = 7;
const WARP_INTERMEDIATE_ROWS: usize = 15;
const WARP_FILTER_TAPS: usize = 8;
const WARP_PARAM_CLIP_LOW: i32 = -32_768;
const WARP_PARAM_CLIP_HIGH: i32 = 32_767 - (1 << (WARP_PARAM_REDUCE_BITS - 1));
const WARP_SHEAR_LIMIT: i32 = 3i32 << WARPEDMODEL_PREC_BITS;

/// AV2 `Default_Warp_Params[6]` identity affine model (§ 7.12.2.11 / § 7.13.3.19).
pub const IDENTITY_WARP_PARAMS: [i32; 6] = [
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
];

/// AV2 § 7.13.3.19 block-warp prediction parameters for one 8x8 section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WarpPredictBlockParams {
    /// `warpParams[0..6]`, a six-parameter affine model at
    /// `WARPEDMODEL_PREC_BITS` precision.
    pub warp_params: [i32; 6],
    /// Top-left x coordinate of this 8x8 prediction section in the current plane.
    pub block_x: i32,
    /// Top-left y coordinate of this 8x8 prediction section in the current plane.
    pub block_y: i32,
    /// Plane horizontal chroma subsampling (`subX`): `0` for luma, otherwise the
    /// sequence `SubsamplingX` value.
    pub subsampling_x: u8,
    /// Plane vertical chroma subsampling (`subY`): `0` for luma, otherwise the
    /// sequence `SubsamplingY` value.
    pub subsampling_y: u8,
    /// Horizontal reference-to-current scale in `REF_SCALE_SHIFT` precision.
    pub reference_scale_x: i32,
    /// Vertical reference-to-current scale in `REF_SCALE_SHIFT` precision.
    pub reference_scale_y: i32,
    /// Inclusive left reference-sampling bound (`firstX`).
    pub first_x: i32,
    /// Inclusive top reference-sampling bound (`firstY`).
    pub first_y: i32,
    /// Inclusive right reference-sampling bound (`lastX`).
    pub last_x: i32,
    /// Inclusive bottom reference-sampling bound (`lastY`).
    pub last_y: i32,
    /// Active bit depth used by the final § 4.8 `Clip1` clamp.
    pub bit_depth: BitDepth,
}

impl WarpPredictBlockParams {
    /// `[firstX, firstY, lastX, lastY]`.
    const fn bounds(&self) -> [i32; 4] {
        [self.first_x, self.first_y, self.last_x, self.last_y]
    }
}

/// Validated affine-warp setup reusable across the 8x8 sections of one block.
#[derive(Clone, Copy, Debug)]
pub struct PreparedWarpPrediction {
    params: WarpPredictBlockParams,
    shear: Shear,
}

impl PreparedWarpPrediction {
    /// Validates the invariant warp parameters and derives the § 7.13.3.21 shear.
    ///
    /// `block_x` and `block_y` in `params` are the initial section coordinates;
    /// [`Self::predict_block_into`] accepts each reused section's coordinates.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid bounds, scaling, subsampling, or shear.
    pub fn new(params: &WarpPredictBlockParams) -> Result<Self> {
        validate_params(params)?;
        Ok(Self {
            params: *params,
            shear: setup_shear(params.warp_params)?,
        })
    }

    /// Returns `None` when the § 7.13.3.21 setup-shear process rejects the
    /// model (the [`warp_shear_is_valid`] test, which selects § 7.13.3.15
    /// `skipPred`), else the setup of [`Self::new`], deriving the shear once.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid bounds, scaling, or subsampling of a
    /// model whose shear is valid.
    pub fn with_valid_shear(params: &WarpPredictBlockParams) -> Result<Option<Self>> {
        let Ok(shear) = setup_shear(params.warp_params) else {
            return Ok(None);
        };
        validate_params(params)?;
        Ok(Some(Self {
            params: *params,
            shear,
        }))
    }

    /// Writes one 8x8 section using the prepared affine-warp setup.
    ///
    /// # Errors
    /// Returns [`ReconError`] when projection arithmetic or a derived filter
    /// phase is outside the checked AV2 envelope.
    pub fn predict_block_into<T: ReconSample>(
        &self,
        reference: &ReferencePlaneView<'_, T>,
        block_x: i32,
        block_y: i32,
        is_compound: bool,
        output: &mut [i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE],
    ) -> Result<()> {
        let params = WarpPredictBlockParams {
            block_x,
            block_y,
            ..self.params
        };
        warp_predict_block_prepared_into(reference, &params, &self.shear, is_compound, output)
    }

    /// Predicts every 8x8 section of a non-compound `width` x `height` block at
    /// `(x, y)` straight into `output` rows `stride` apart, applying the final
    /// § 7.13.3.19 `Clip1` as each sample is stored.
    ///
    /// # Errors
    /// Returns the errors of [`Self::predict_block_into`],
    /// [`ReconError::SampleTypeUnsupportedBitDepth`] when `T` cannot hold the
    /// bit depth, [`ReconError::ArithmeticOverflow`] when the block's end
    /// does not fit `i32`, [`ReconError::StrideTooSmall`] when `stride` is below
    /// `width`, or [`ReconError::BufferLengthMismatch`] when `output` cannot
    /// hold the block.
    pub fn predict_clipped_into<T: ReconSample>(
        &self,
        reference: &ReferencePlaneView<'_, T>,
        [x, y, width, height]: [usize; 4],
        output: &mut [T],
        stride: usize,
    ) -> Result<()> {
        crate::intra_dc_math::validate_sample_type::<T>(self.params.bit_depth)?;
        let fits = |start: usize, len: usize| {
            start
                .checked_add(len)
                .is_some_and(|end| i32::try_from(end).is_ok())
        };
        if !fits(x, width) || !fits(y, height) {
            return Err(ReconError::ArithmeticOverflow {
                context: "warp block position",
            });
        }
        if stride < width {
            return Err(ReconError::StrideTooSmall {
                stride_samples: stride,
                storage_width: width,
            });
        }
        let span = height
            .checked_sub(1)
            .and_then(|rows| rows.checked_mul(stride))
            .and_then(|start| start.checked_add(width))
            .filter(|&span| span <= output.len())
            .ok_or(ReconError::BufferLengthMismatch {
                expected: height.saturating_mul(stride),
                actual: output.len(),
            })?;
        let output = &mut output[..span];
        let max_sample = i32::from(self.params.bit_depth.max_sample());
        if self.translation_two_tap_into(
            reference,
            [x, y, width, height],
            output,
            stride,
            max_sample,
        )? {
            return Ok(());
        }
        for local_y in (0..height).step_by(WARPED_BLOCK_SIZE) {
            for local_x in (0..width).step_by(WARPED_BLOCK_SIZE) {
                let params = WarpPredictBlockParams {
                    block_x: (x + local_x) as i32,
                    block_y: (y + local_y) as i32,
                    ..self.params
                };
                let section_w = (width - local_x).min(WARPED_BLOCK_SIZE);
                let section_h = (height - local_y).min(WARPED_BLOCK_SIZE);
                let store = |row: usize, rounded: Simd<i32, WARPED_BLOCK_SIZE>| {
                    if row >= section_h {
                        return Ok(());
                    }
                    let start = (local_y + row) * stride + local_x;
                    store_clipped_row(&mut output[start..start + section_w], rounded, max_sample)
                };
                warp_predict_section(
                    reference,
                    &params,
                    &self.shear,
                    INTER_ROUND1_NON_COMPOUND,
                    store,
                )?;
            }
        }
        Ok(())
    }

    /// Predicts a whole translation-only block whose two phases both select a
    /// two-tap filter, returning `false` without writing for any other block
    /// or one that reads a clipped sample.
    ///
    /// A translation gives every 8x8 section the same phases and starts each
    /// section's source window eight samples after its neighbour's, so the
    /// sections tile one block-wide two-tap filter: each source row is
    /// filtered once for the block instead of once per section.
    fn translation_two_tap_into<T: ReconSample>(
        &self,
        reference: &ReferencePlaneView<'_, T>,
        [x, y, width, height]: [usize; 4],
        output: &mut [T],
        stride: usize,
        max_sample: i32,
    ) -> Result<bool> {
        if self.params.warp_params[2..] != IDENTITY_WARP_PARAMS[2..]
            || !width.is_multiple_of(WARPED_BLOCK_SIZE)
        {
            return Ok(false);
        }
        let projected = project_section_center(&WarpPredictBlockParams {
            block_x: x as i32,
            block_y: y as i32,
            ..self.params
        })?;
        let (Some(high), Some(vertical_high)) =
            (two_tap_phase(projected.sx4), two_tap_phase(projected.sy4))
        else {
            return Ok(false);
        };
        let first_tap = TWO_TAP_FIRST as i64 - 7;
        let first_col = i64::from(projected.x4_int) + first_tap;
        let first_row = i64::from(projected.y4_int) + first_tap;
        let params = &self.params;
        let last_col = i64::from(params.last_x).min(reference.width() as i64 - 1);
        let last_row = i64::from(params.last_y).min(reference.height() as i64 - 1);
        if first_col < i64::from(params.first_x)
            || first_row < i64::from(params.first_y)
            || first_col + width as i64 > last_col
            || first_row + height as i64 > last_row
        {
            return Ok(false);
        }
        let (first_col, first_row) = (first_col as usize, first_row as usize);
        let mut col = 0;
        while col + 16 <= width {
            let strip = [first_col + col, first_row, height, high];
            let target = &mut output[col..];
            two_tap_strip::<T, 16>(reference, strip, vertical_high, target, stride, max_sample)?;
            col += 16;
        }
        if col < width {
            let strip = [first_col + col, first_row, height, high];
            let target = &mut output[col..];
            two_tap_strip::<T, 8>(reference, strip, vertical_high, target, stride, max_sample)?;
        }
        Ok(true)
    }
}

/// Filters one `LANES`-column strip of a translation block down all `height`
/// rows from `[first_col, first_row, height, high]`, keeping the previous
/// horizontal row in registers for the vertical taps.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn two_tap_strip<T: ReconSample, const LANES: usize>(
    reference: &ReferencePlaneView<'_, T>,
    [first_col, first_row, height, high]: [usize; 4],
    vertical_high: usize,
    output: &mut [T],
    stride: usize,
    max_sample: i32,
) -> Result<()> {
    let mut top = two_tap_horizontal::<T, LANES>(reference.row(first_row), first_col, high);
    for row in 0..height {
        let bottom = two_tap_horizontal(reference.row(first_row + row + 1), first_col, high);
        let rounded = two_tap_vertical(top, bottom, vertical_high, INTER_ROUND1_NON_COMPOUND);
        let at = row * stride;
        store_clipped_row(&mut output[at..at + LANES], rounded.cast(), max_sample)?;
        top = bottom;
    }
    Ok(())
}

/// Stores the leading `target.len()` lanes of one non-compound warp output row
/// clipped to `0..=max_sample`, a bound within the storage type.
///
/// An `i16` intermediate times filter taps of absolute sum at most 222, after
/// `Round2(_, InterRound1)` with the non-compound 11, stays within 3552, so
/// narrowing the row to `i16` before the clamp is exact.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn store_clipped_row<T: ReconSample, const LANES: usize>(
    target: &mut [T],
    rounded: Simd<i32, LANES>,
    max_sample: i32,
) -> Result<()> {
    let clipped = rounded
        .cast::<i16>()
        .simd_clamp(Simd::splat(0), Simd::splat(max_sample as i16));
    if let Some(target) = T::u16_slice_mut(target) {
        store_lanes(target, clipped.cast());
    } else if let Some(target) = T::u8_slice_mut(target) {
        store_lanes(target, clipped.cast());
    } else {
        for (target, &sample) in target.iter_mut().zip(&clipped.to_array()) {
            *target = T::try_from_u16(sample as u16)?;
        }
    }
    Ok(())
}

#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn store_lanes<S: SimdElement, const LANES: usize>(target: &mut [S], lanes: Simd<S, LANES>) {
    if target.len() == LANES {
        lanes.copy_to_slice(target);
    } else {
        for (target, lane) in target.iter_mut().zip(lanes.to_array()) {
            *target = lane;
        }
    }
}

/// AV2 § 3 `EXT_WARP_TAPS`.
const EXT_WARP_TAPS: usize = 6;
/// AV2 § 3 `EXT_WARP_ROUND_BITS` = `WARPEDMODEL_PREC_BITS - EXT_WARP_PHASES_LOG2`.
const EXT_WARP_ROUND_BITS: u32 = 10;
/// Horizontal-pass rows the scaled extended warp can need.
///
/// § 7.11.3 admits at most a 2x reference-to-current scale, which spans
/// `3 * 2 + 1 + EXT_WARP_TAPS = 13` rows; the bound leaves room well past that
/// and turns anything beyond it into a typed error instead of an allocation.
const MAX_EXT_WARP_INTERMEDIATE_ROWS: usize = 64;

/// Reports whether the § 7.13.3.21 setup-shear process accepts a warp model,
/// deciding the § 7.13.3.15 `skipPred` fallback to the extended block warp.
#[must_use]
pub fn warp_shear_is_valid(warp_params: [i32; 6]) -> bool {
    setup_shear(warp_params).is_ok()
}

/// AV2 § 7.13.3.20 extended block warp: predicts one 4x4 unit
/// at `(j4, i4)` (in 4-sample units relative to the block's plane top-left) by
/// projecting the unit centre through the warp model and running the fixed-
/// phase `Ext_Warped_Filters` two-pass interpolation over the clipped
/// reference window. `is_compound` selects the § 7.13.3.16 `InterRound1`
/// shift; the returned values are the unclipped `Round2(s, InterRound1)`
/// predictors — single-reference writers finish with the § 4.8
/// [`crate::math::clip1_predicted_samples`] clamp, compound callers blend
/// the `Preds[refList]` intermediates first.
///
/// # Errors
/// Returns [`ReconError`] for invalid reference bounds, a filter phase outside
/// the generated table, or arithmetic overflow.
pub fn ext_warp_predict_unit<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    i4: usize,
    j4: usize,
    is_compound: bool,
) -> Result<[i32; 16]> {
    let round1 = if is_compound {
        INTER_ROUND1_COMPOUND
    } else {
        INTER_ROUND1_NON_COMPOUND
    };
    validate_params(params)?;
    let sub_x = u32::from(params.subsampling_x);
    let sub_y = u32::from(params.subsampling_y);
    let src_x = checked_ext_warp_source(params.block_x, j4, sub_x, "extended-warp source x")?;
    let src_y = checked_ext_warp_source(params.block_y, i4, sub_y, "extended-warp source y")?;
    let dst_x = project_coordinate(
        params.warp_params[2],
        params.warp_params[3],
        params.warp_params[0],
        src_x,
        src_y,
        "extended-warp projected x",
    )?;
    let dst_y = project_coordinate(
        params.warp_params[4],
        params.warp_params[5],
        params.warp_params[1],
        src_x,
        src_y,
        "extended-warp projected y",
    )?;
    let mut x4 = dst_x >> sub_x;
    let mut y4 = dst_y >> sub_y;
    if params.reference_scale_x != 1 << REF_SCALE_SHIFT
        || params.reference_scale_y != 1 << REF_SCALE_SHIFT
    {
        let scaled_coordinate =
            |coordinate: i64, scale: i32, context: &'static str| -> Result<i64> {
                let coordinate = coordinate
                    .checked_sub(2 << WARPEDMODEL_PREC_BITS)
                    .and_then(|value| value.checked_mul(i64::from(scale)))
                    .ok_or(ReconError::ArithmeticOverflow { context })?;
                Ok(round2_signed(coordinate, REF_SCALE_SHIFT))
            };
        x4 = scaled_coordinate(
            x4,
            params.reference_scale_x,
            "scaled extended-warp projected x",
        )?;
        y4 = scaled_coordinate(
            y4,
            params.reference_scale_y,
            "scaled extended-warp projected y",
        )?;
        let step_x = i64::from(round2_signed_i32(
            params.reference_scale_x,
            REF_SCALE_SHIFT - SCALE_SUBPEL_BITS,
        )) << (WARPEDMODEL_PREC_BITS - SCALE_SUBPEL_BITS);
        let step_y = i64::from(round2_signed_i32(
            params.reference_scale_y,
            REF_SCALE_SHIFT - SCALE_SUBPEL_BITS,
        )) << (WARPEDMODEL_PREC_BITS - SCALE_SUBPEL_BITS);
        return ext_warp_predict_scaled(reference, params, x4, y4, step_x, step_y, round1);
    }
    let ix4 =
        i32::try_from(x4 >> WARPEDMODEL_PREC_BITS).map_err(|_| ReconError::ArithmeticOverflow {
            context: "extended-warp projected x",
        })?;
    let sx4 = (x4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)) as i32;
    let iy4 =
        i32::try_from(y4 >> WARPEDMODEL_PREC_BITS).map_err(|_| ReconError::ArithmeticOverflow {
            context: "extended-warp projected y",
        })?;
    let sy4 = (y4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)) as i32;
    let phase = |s: i32| -> Result<&'static [i32; EXT_WARP_TAPS]> {
        let offs = round2_i32(s, EXT_WARP_ROUND_BITS);
        usize::try_from(offs)
            .ok()
            .and_then(|offs| EXT_WARPED_FILTERS.get(offs))
            .ok_or(ReconError::WarpFilterOffsetOutOfRange { offset: offs })
    };
    let taps_x = phase(sx4)?;
    let taps_y = phase(sy4)?;
    let mut intermediate = [Simd::<i32, 4>::splat(0); 9];
    let (x, y) = (i64::from(ix4), i64::from(iy4));
    let interior = x - 4 >= i64::from(params.first_x.max(0))
        && x + 4 <= i64::from(params.last_x)
        && x + 5 <= reference.width() as i64
        && y - 4 >= i64::from(params.first_y.max(0))
        && y + 4 <= i64::from(params.last_y);
    if interior {
        let col = (ix4 - 4) as usize;
        for (k, intermediate) in intermediate.iter_mut().enumerate() {
            let row = reference.row((iy4 + k as i32 - 4) as usize);
            let lo = warp_source_lanes::<T, 8>(row, col);
            let hi = warp_source_lanes::<T, 8>(row, col + 1);
            let windows = [
                simd_swizzle!(lo, [0, 1, 2, 3]),
                simd_swizzle!(lo, [1, 2, 3, 4]),
                simd_swizzle!(lo, [2, 3, 4, 5]),
                simd_swizzle!(lo, [3, 4, 5, 6]),
                simd_swizzle!(lo, [4, 5, 6, 7]),
                simd_swizzle!(lo, hi, [5, 6, 7, 15]),
            ];
            let mut sum = Simd::splat(0);
            for (&tap, window) in taps_x.iter().zip(windows) {
                sum += Simd::splat(tap) * window.cast::<i32>();
            }
            *intermediate = (sum + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i32;
        }
    } else {
        let mut cols = [0usize; 9];
        for (offset, col) in cols.iter_mut().enumerate() {
            let x = (ix4 - 4 + offset as i32).clamp(params.first_x, params.last_x);
            *col = (x as usize).min(reference.width() - 1);
        }
        for (k, intermediate) in intermediate.iter_mut().enumerate() {
            let row = (iy4 + k as i32 - 4).clamp(params.first_y, params.last_y) as usize;
            let (row, mut window) = (reference.row(row), [0i32; 9]);
            for (sample, &col) in window.iter_mut().zip(&cols) {
                *sample = i32::from(row[col].to_u16());
            }
            let mut sum = Simd::splat(0);
            for (m, &tap) in taps_x.iter().enumerate() {
                sum += Simd::splat(tap) * Simd::from_slice(&window[m..]);
            }
            *intermediate = (sum + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i32;
        }
    }
    let mut output = [0i32; 16];
    for (k, output) in output.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let mut sum = Simd::splat(0);
        for (m, &tap) in taps_y.iter().enumerate() {
            sum += Simd::splat(tap) * intermediate[k + m];
        }
        *output = ((sum + Simd::splat(1 << (round1 - 1))) >> round1 as i32).to_array();
    }
    Ok(output)
}

fn ext_warp_predict_scaled<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    x4: i64,
    y4: i64,
    step_x: i64,
    step_y: i64,
    round1: u32,
) -> Result<[i32; 16]> {
    let phase = |s: i64| -> Result<&'static [i32; EXT_WARP_TAPS]> {
        let offs = round2(s, EXT_WARP_ROUND_BITS);
        let offset = i32::try_from(offs).map_err(|_| ReconError::ArithmeticOverflow {
            context: "scaled extended-warp filter offset",
        })?;
        usize::try_from(offset)
            .ok()
            .and_then(|offs| EXT_WARPED_FILTERS.get(offs))
            .ok_or(ReconError::WarpFilterOffsetOutOfRange { offset })
    };
    let fetch = |row: i64, col: i64| -> i32 {
        let rr = row.clamp(i64::from(params.first_y), i64::from(params.last_y));
        let cc = col.clamp(i64::from(params.first_x), i64::from(params.last_x));
        reference.sample(rr as usize, cc as usize)
    };
    let iy4 = y4 >> WARPEDMODEL_PREC_BITS;
    let intermediate_height =
        ((y4 + step_y * 3) >> WARPEDMODEL_PREC_BITS) - iy4 + EXT_WARP_TAPS as i64;
    let intermediate_height = usize::try_from(intermediate_height)
        .ok()
        .filter(|&rows| rows <= MAX_EXT_WARP_INTERMEDIATE_ROWS)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "scaled extended-warp intermediate height",
        })?;
    let mut storage = [0i32; MAX_EXT_WARP_INTERMEDIATE_ROWS * 4];
    let intermediate = &mut storage[..intermediate_height * 4];
    for k in 0..intermediate_height {
        for l in 0..4 {
            let sample_x = x4 + step_x * l as i64;
            let int_x = sample_x >> WARPEDMODEL_PREC_BITS;
            let taps_x = phase(sample_x & ((1 << WARPEDMODEL_PREC_BITS) - 1))?;
            let int_y = iy4 + k as i64 - 2;
            let mut sum = 0i32;
            for (m, &tap) in taps_x.iter().enumerate() {
                sum += tap * fetch(int_y, int_x - 2 + m as i64);
            }
            intermediate[k * 4 + l] = round2_i32(sum, INTER_ROUND0);
        }
    }
    let mut out = [0i32; 16];
    for l in 0..4 {
        for k in 0..4 {
            let sample_y = y4 + step_y * k as i64;
            let row = usize::try_from((sample_y >> WARPEDMODEL_PREC_BITS) - iy4).map_err(|_| {
                ReconError::ArithmeticOverflow {
                    context: "scaled extended-warp intermediate row",
                }
            })?;
            let taps_y = phase(sample_y & ((1 << WARPEDMODEL_PREC_BITS) - 1))?;
            let mut sum = 0i32;
            for (m, &tap) in taps_y.iter().enumerate() {
                let value =
                    intermediate
                        .get((row + m) * 4 + l)
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "scaled extended-warp intermediate access",
                        })?;
                sum += tap * value;
            }
            out[k * 4 + l] = round2_i32(sum, round1);
        }
    }
    Ok(out)
}

/// Runs AV2 § 7.13.3.19 block-warp convolution for one 8x8 section, returning
/// row-major unclipped predictors.
///
/// The caller supplies an already-derived affine warp model and the current-plane
/// top-left coordinate of the 8x8 section. This function computes the section
/// center projection, validates the § 7.13.3.21 shear, applies the generated § 9.5
/// `Warped_Filters` table in the horizontal and vertical passes, and clips
/// reference reads to `[firstX, lastX] x [firstY, lastY]`. `is_compound`
/// selects the § 7.13.3.16 `InterRound1` shift; the 64 returned values are the
/// unclipped `Round2(s, InterRound1)` predictors — single-reference writers
/// finish with the § 4.8 [`crate::math::clip1_predicted_samples`] clamp,
/// compound callers blend the `Preds[refList]` intermediates first.
///
/// # Errors
///
/// Returns [`ReconError::WarpSubsamplingUnsupported`] for non-AV2 subsampling
/// factors, [`ReconError::WarpReferenceBoundsInvalid`] for a negative or empty
/// reference rectangle, [`ReconError::WarpInvalidShear`] when setup-shear rejects
/// the model, [`ReconError::WarpFilterOffsetOutOfRange`] for a derived filter row
/// outside the generated table, and [`ReconError::ArithmeticOverflow`] if public
/// caller inputs exceed the checked arithmetic envelope.
pub fn warp_predict_block<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    is_compound: bool,
) -> Result<Vec<i32>> {
    let mut output = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
    warp_predict_block_into(reference, params, is_compound, &mut output)?;
    Ok(output.into())
}

/// Writes one AV2 § 7.13.3.19 block-warp prediction into a caller-owned 8x8
/// array.
///
/// The prediction semantics match [`warp_predict_block`], including the
/// compound `InterRound1` selection and unclipped `i32` output. The output may
/// be partially modified when a derived vertical filter phase is invalid.
///
/// # Errors
/// Returns the same errors as [`warp_predict_block`].
pub fn warp_predict_block_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    is_compound: bool,
    output: &mut [i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE],
) -> Result<()> {
    PreparedWarpPrediction::new(params)?.predict_block_into(
        reference,
        params.block_x,
        params.block_y,
        is_compound,
        output,
    )
}

fn warp_predict_block_prepared_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    shear: &Shear,
    is_compound: bool,
    output: &mut [i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE],
) -> Result<()> {
    let round1 = if is_compound {
        INTER_ROUND1_COMPOUND
    } else {
        INTER_ROUND1_NON_COMPOUND
    };
    warp_predict_section(reference, params, shear, round1, |row, rounded| {
        output[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE]
            .copy_from_slice(&rounded.to_array()); // splot-copy-ok: publish row-wide SIMD warp output
        Ok(())
    })
}

/// Runs both § 7.13.3.19 passes for one 8x8 section and hands each rounded
/// output row to `store`.
fn warp_predict_section<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    shear: &Shear,
    round1: u32,
    store: impl FnMut(usize, Simd<i32, WARPED_BLOCK_SIZE>) -> Result<()>,
) -> Result<()> {
    let projected = project_section_center(params)?;
    let mut intermediate = [0i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
    if let Some(source_origin) = interior_warp_source_origin(reference, params, &projected) {
        if shear.alpha == 0
            && shear.beta == 0
            && let Some(high) = two_tap_phase(projected.sx4)
        {
            two_tap_intermediate(reference, source_origin, high, &mut intermediate);
        } else if shear.alpha != 0 && shear.beta == 0 {
            row_invariant_intermediate(
                reference,
                shear,
                &projected,
                source_origin,
                &mut intermediate,
            );
        } else {
            build_interior_intermediate(
                reference,
                shear,
                &projected,
                source_origin,
                &mut intermediate,
            );
        }
    } else {
        build_intermediate(
            reference,
            params.bounds(),
            shear,
            &projected,
            &mut intermediate,
        );
    }
    if shear.gamma == 0
        && shear.delta == 0
        && let Some(high) = two_tap_phase(projected.sy4)
    {
        return two_tap_output(high, &intermediate, round1, store);
    }
    if shear.gamma != 0 && shear.delta == 0 {
        return row_invariant_output(shear, &projected, &intermediate, round1, store);
    }
    build_output(shear, &projected, &intermediate, round1, store)
}

/// Runs the § 7.13.3.19 horizontal pass of a section with `beta == 0`, whose
/// rows all use the same eight column filters: they are transposed once for
/// the section instead of once per row.
fn row_invariant_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    let taps = transpose_warp_taps(core::array::from_fn(|col| {
        warp_taps(projected.sx4 + shear.alpha * (col as i32 - 4))
    }));
    for row in 0..WARP_INTERMEDIATE_ROWS {
        let windows = warp_windows(reference.row(first_row + row), first_col);
        let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
        for (window, taps) in windows.into_iter().zip(taps) {
            sum += window.cast::<i32>() * taps.cast::<i32>();
        }
        let rounded = (sum + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i32;
        rounded.cast::<i16>().copy_to_slice(
            &mut intermediate[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE],
        );
    }
}

/// Runs the § 7.13.3.19 vertical pass of a section with `delta == 0`, whose
/// output rows all use the same eight column filters.
fn row_invariant_output(
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &[i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    round1: u32,
    mut store: impl FnMut(usize, Simd<i32, WARPED_BLOCK_SIZE>) -> Result<()>,
) -> Result<()> {
    let weights = paired_column_taps(core::array::from_fn(|col| {
        warp_taps(projected.sy4 + shear.gamma * (col as i32 - 4))
    }));
    for row in 0..WARPED_BLOCK_SIZE {
        let sum = paired_rows_sum(intermediate, row, &weights);
        store(row, (sum + Simd::splat(1 << (round1 - 1))) >> round1 as i32)?;
    }
    Ok(())
}

/// First of the two adjacent nonzero taps of a two-tap `Warped_Filters` row.
const TWO_TAP_FIRST: usize = 3;

/// Reports whether a zero-shear `phase` selects `Warped_Filters` row 192 or
/// 256, whose only nonzero taps are 127 and 1 at [`TWO_TAP_FIRST`] and the
/// next tap, and returns `1` when the 127 is the second of them.
///
/// Zero shear keeps the projected phase in `0..1 << WARPEDMODEL_PREC_BITS`,
/// where those are the rows with `Round2(phase, WARPEDDIFF_PREC_BITS)` 0 or 64.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn two_tap_phase(phase: i32) -> Option<usize> {
    let rounded = phase + (1 << (WARPEDDIFF_PREC_BITS - 1));
    (rounded & ((WARPEDPIXEL_PREC_SHIFTS - 1) << WARPEDDIFF_PREC_BITS) == 0)
        .then_some((rounded >> WARPEDMODEL_PREC_BITS) as usize)
}

/// Runs the § 7.13.3.19 horizontal pass of a two-tap phase over the `LANES`
/// samples whose first tap reads `source[start]`.
///
/// With taps `127` and `1`, `Round2(127 * a + b, InterRound0)` equals
/// `16 * a + Round2(b - a, InterRound0)`, which keeps every 10-bit lane in
/// `i16`; `high` selects which of the two samples carries the 127.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn two_tap_horizontal<T: ReconSample, const LANES: usize>(
    source: &[T],
    start: usize,
    high: usize,
) -> Simd<i16, LANES> {
    let main = warp_source_lanes(source, start + high);
    let side = warp_source_lanes(source, start + 1 - high);
    (main << (7 - INTER_ROUND0) as i16)
        + ((side - main + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i16)
}

/// Runs the § 7.13.3.19 horizontal pass of a two-tap phase over the interior
/// window at `(first_col, first_row)`.
fn two_tap_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    (first_col, first_row): (usize, usize),
    high: usize,
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    for row in 0..WARP_INTERMEDIATE_ROWS {
        let source = reference.row(first_row + row);
        two_tap_horizontal::<T, WARPED_BLOCK_SIZE>(source, first_col + TWO_TAP_FIRST, high)
            .copy_to_slice(
                &mut intermediate[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE],
            );
    }
}

/// Runs the § 7.13.3.19 vertical pass of a two-tap phase, which reads only
/// intermediate rows [`TWO_TAP_FIRST`] through `TWO_TAP_FIRST + 8`.
fn two_tap_output(
    high: usize,
    intermediate: &[i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    round1: u32,
    mut store: impl FnMut(usize, Simd<i32, WARPED_BLOCK_SIZE>) -> Result<()>,
) -> Result<()> {
    for row in 0..WARPED_BLOCK_SIZE {
        let top = (row + TWO_TAP_FIRST) * WARPED_BLOCK_SIZE;
        let top_samples = Simd::<i16, WARPED_BLOCK_SIZE>::from_slice(&intermediate[top..]);
        let bottom_samples =
            Simd::<i16, WARPED_BLOCK_SIZE>::from_slice(&intermediate[top + WARPED_BLOCK_SIZE..]);
        store(
            row,
            two_tap_vertical(top_samples, bottom_samples, high, round1).cast(),
        )?;
    }
    Ok(())
}

/// Runs the § 7.13.3.19 vertical pass of a two-tap phase on `i16` lanes:
/// `Round2(127 * main + side, round1)`, where `high` selects `bottom` as the
/// sample under the 127.
///
/// `127 * main + side` is `128 * main + (side - main)`, and for `round1 >= 7`
/// that rounds to `(main + ((side - main + 2^(round1 - 1)) >> 7)) >> (round1 - 7)`.
/// Intermediates lie in `-6010..=22378`, so every step stays within `i16`.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn two_tap_vertical<const LANES: usize>(
    top: Simd<i16, LANES>,
    bottom: Simd<i16, LANES>,
    high: usize,
    round1: u32,
) -> Simd<i16, LANES> {
    let (main, side) = if high == 0 {
        (top, bottom)
    } else {
        (bottom, top)
    };
    let side = (side - main + Simd::splat(1 << (round1 - 1))) >> 7;
    (main + side) >> (round1 - 7) as i16
}

/// Admits the unclamped interior source origin for one 8x8 warp section.
///
/// The taps reach `last_col`, but [`warp_windows`] reads one whole vector past
/// `first_col + WARPED_BLOCK_SIZE`, so admission reserves the column after
/// `last_col` as well. Sections at the final column fall to the clamped
/// [`build_intermediate`], which yields the same samples.
fn interior_warp_source_origin<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    projected: &ProjectedCenter,
) -> Option<(usize, usize)> {
    let first_col = projected.x4_int.checked_sub(7)?;
    let last_col = projected.x4_int.checked_add(7)?;
    let first_row = projected.y4_int.checked_sub(7)?;
    let last_row = projected.y4_int.checked_add(7)?;
    let source_origin = (
        usize::try_from(first_col).ok()?,
        usize::try_from(first_row).ok()?,
    );
    (first_col >= params.first_x
        && last_col <= params.last_x
        && first_row >= params.first_y
        && last_row <= params.last_y
        && usize::try_from(last_col).is_ok_and(|col| col + 1 < reference.width())
        && usize::try_from(last_row).is_ok_and(|row| row < reference.height()))
    .then_some(source_origin)
}

fn validate_params(params: &WarpPredictBlockParams) -> Result<()> {
    if params.subsampling_x > 1 || params.subsampling_y > 1 {
        return Err(ReconError::WarpSubsamplingUnsupported {
            subsampling_x: params.subsampling_x,
            subsampling_y: params.subsampling_y,
        });
    }
    if params.reference_scale_x <= 0 || params.reference_scale_y <= 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "block-warp reference scale",
        });
    }
    if params.first_x < 0
        || params.first_y < 0
        || params.first_x > params.last_x
        || params.first_y > params.last_y
    {
        return Err(ReconError::WarpReferenceBoundsInvalid {
            first_x: params.first_x,
            first_y: params.first_y,
            last_x: params.last_x,
            last_y: params.last_y,
        });
    }
    Ok(())
}

fn checked_ext_warp_source(
    block: i32,
    unit: usize,
    subsampling: u32,
    context: &'static str,
) -> Result<i32> {
    let unit = i32::try_from(unit).map_err(|_| ReconError::ArithmeticOverflow { context })?;
    block
        .checked_add(
            unit.checked_mul(4)
                .and_then(|value| value.checked_add(2))
                .ok_or(ReconError::ArithmeticOverflow { context })?,
        )
        .and_then(|value| value.checked_mul(1i32 << subsampling))
        .ok_or(ReconError::ArithmeticOverflow { context })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Shear {
    alpha: i32,
    beta: i32,
    gamma: i32,
    delta: i32,
}

fn setup_shear(warp_params: [i32; 6]) -> Result<Shear> {
    let alpha0 = warp_params[2]
        .checked_sub(1 << WARPEDMODEL_PREC_BITS)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "block-warp alpha setup",
        })?
        .clamp(WARP_PARAM_CLIP_LOW, WARP_PARAM_CLIP_HIGH);
    let beta0 = warp_params[3].clamp(WARP_PARAM_CLIP_LOW, WARP_PARAM_CLIP_HIGH);
    let (div_shift, div_factor) = resolve_signed_divisor(warp_params[2])?;
    let v_product = warp_product(
        warp_params[4],
        1 << WARPEDMODEL_PREC_BITS,
        div_factor,
        "block-warp gamma setup",
    )?;
    let gamma0 = round2_signed(v_product, div_shift).clamp(
        i64::from(WARP_PARAM_CLIP_LOW),
        i64::from(WARP_PARAM_CLIP_HIGH),
    ) as i32;

    let w_product = warp_product(
        warp_params[3],
        warp_params[4],
        div_factor,
        "block-warp delta setup",
    )?;
    let rounded_w = round2_signed(w_product, div_shift);
    let delta_input = i64::from(warp_params[5])
        .checked_sub(rounded_w)
        .and_then(|value| value.checked_sub(1 << WARPEDMODEL_PREC_BITS))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "block-warp delta setup",
        })?;
    let delta0 = delta_input.clamp(
        i64::from(WARP_PARAM_CLIP_LOW),
        i64::from(WARP_PARAM_CLIP_HIGH),
    ) as i32;

    let shear = Shear {
        alpha: reduce_shear_i32(alpha0),
        beta: reduce_shear_i32(beta0),
        gamma: reduce_shear_i32(gamma0),
        delta: reduce_shear_i32(delta0),
    };
    if 4 * shear.alpha.abs() + 7 * shear.beta.abs() >= WARP_SHEAR_LIMIT
        || 4 * shear.gamma.abs() + 4 * shear.delta.abs() >= WARP_SHEAR_LIMIT
    {
        return Err(ReconError::WarpInvalidShear {
            alpha: shear.alpha,
            beta: shear.beta,
            gamma: shear.gamma,
            delta: shear.delta,
        });
    }
    Ok(shear)
}

fn reduce_shear_i32(value: i32) -> i32 {
    round2_signed_i32(value, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS
}

fn resolve_signed_divisor(d: i32) -> Result<(u32, i32)> {
    if d == 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "block-warp divisor resolution",
        });
    }
    let (shift, factor) = resolve_divisor_32(d.unsigned_abs())?;
    let factor = if d < 0 {
        -i32::from(factor)
    } else {
        i32::from(factor)
    };
    Ok((u32::from(shift), factor))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectedCenter {
    x4_int: i32,
    y4_int: i32,
    sx4: i32,
    sy4: i32,
}

#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn project_section_center(params: &WarpPredictBlockParams) -> Result<ProjectedCenter> {
    let src_x = checked_shift_left(
        params
            .block_x
            .checked_add(4)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "block-warp source x",
            })?,
        params.subsampling_x,
        "block-warp source x",
    )?;
    let src_y = checked_shift_left(
        params
            .block_y
            .checked_add(4)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "block-warp source y",
            })?,
        params.subsampling_y,
        "block-warp source y",
    )?;
    let dst_x = project_coordinate(
        params.warp_params[2],
        params.warp_params[3],
        params.warp_params[0],
        src_x,
        src_y,
        "block-warp projected x",
    )?;
    let dst_y = project_coordinate(
        params.warp_params[4],
        params.warp_params[5],
        params.warp_params[1],
        src_x,
        src_y,
        "block-warp projected y",
    )?;
    let x4 = dst_x >> params.subsampling_x;
    let y4 = dst_y >> params.subsampling_y;
    let mask = (1 << WARPEDMODEL_PREC_BITS) - 1;
    Ok(ProjectedCenter {
        x4_int: i32::try_from(x4 >> WARPEDMODEL_PREC_BITS).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "block-warp projected x",
            }
        })?,
        y4_int: i32::try_from(y4 >> WARPEDMODEL_PREC_BITS).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "block-warp projected y",
            }
        })?,
        sx4: i32::try_from(x4 & mask).map_err(|_| ReconError::ArithmeticOverflow {
            context: "block-warp horizontal phase",
        })?,
        sy4: i32::try_from(y4 & mask).map_err(|_| ReconError::ArithmeticOverflow {
            context: "block-warp vertical phase",
        })?,
    })
}

fn checked_shift_left(value: i32, shift: u8, context: &'static str) -> Result<i32> {
    value
        .checked_mul(1i32 << shift)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn build_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    [first_x, first_y, last_x, last_y]: [i32; 4],
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    const SOURCE_WINDOW: usize = WARPED_BLOCK_SIZE + WARP_FILTER_TAPS - 1;
    let first_col = projected.x4_int.saturating_sub(7);
    let last_col = projected.x4_int.saturating_add(7);
    let view_last = i32::try_from(reference.width().saturating_sub(1)).unwrap_or(i32::MAX);
    let bounded_first = first_x.min(view_last);
    let bounded_last = last_x.min(view_last);
    let copy_first = first_col.max(bounded_first);
    let copy_last = last_col.min(bounded_last);
    let copy_len = if copy_first <= copy_last {
        (copy_last - copy_first) as usize + 1
    } else {
        0
    };
    let prefix_len = if copy_len == 0 {
        SOURCE_WINDOW
    } else {
        (copy_first - first_col) as usize
    };
    let mut window = [0u16; SOURCE_WINDOW + 1];
    for i1 in -7i32..8 {
        let ref_row = (projected.y4_int + i1).clamp(first_y, last_y) as usize;
        let source = reference.row(ref_row);
        if copy_len == 0 {
            let source_col = first_col.clamp(bounded_first, bounded_last) as usize;
            window.fill(source[source_col].to_u16());
        } else {
            let source_start = copy_first as usize;
            let source_end = source_start + copy_len;
            let copied_end = prefix_len + copy_len;
            window[..prefix_len].fill(source[source_start].to_u16());
            if let Some(source) = T::u16_slice(source) {
                window[prefix_len..copied_end].copy_from_slice(&source[source_start..source_end]); // splot-copy-ok: materialize one clipped warp tap window from its contiguous span
            } else {
                for (destination, source) in window[prefix_len..copied_end]
                    .iter_mut()
                    .zip(&source[source_start..source_end])
                {
                    *destination = source.to_u16();
                }
            }
            window[copied_end..].fill(source[source_end - 1].to_u16());
        }
        let (lo, hi) = window.split_at(WARPED_BLOCK_SIZE);
        let windows = sliding_windows(
            Simd::<u16, WARPED_BLOCK_SIZE>::from_slice(lo).cast(),
            Simd::<u16, WARPED_BLOCK_SIZE>::from_slice(hi).cast(),
        );
        let row = (i1 + 7) as usize;
        horizontal_row(windows, projected.sx4 + shear.beta * i1, shear.alpha).copy_to_slice(
            &mut intermediate[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE],
        );
    }
}

fn build_interior_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    for row in 0..WARP_INTERMEDIATE_ROWS {
        let i1 = row as i32 - 7;
        let windows = warp_windows(reference.row(first_row + row), first_col);
        horizontal_row(windows, projected.sx4 + shear.beta * i1, shear.alpha).copy_to_slice(
            &mut intermediate[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE],
        );
    }
}

/// Runs the § 7.13.3.19 horizontal pass for one intermediate row from its tap
/// windows, whose column `col` uses the filter of phase `sx + alpha * (col - 4)`.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn horizontal_row(
    windows: [Simd<i16, WARPED_BLOCK_SIZE>; 8],
    sx: i32,
    alpha: i32,
) -> Simd<i16, WARPED_BLOCK_SIZE> {
    let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
    if alpha == 0 {
        let taps = warp_taps(sx);
        for (tap, window) in windows.into_iter().enumerate() {
            sum = warp_tap_mac(sum, window, taps[tap]);
        }
    } else {
        sum = column_dot_products(core::array::from_fn(|col| {
            windows[col].cast::<i32>() * warp_taps(sx + alpha * (col as i32 - 4)).cast::<i32>()
        }));
    }
    ((sum + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i32).cast()
}

/// Sums the eight lanes of each column's tap products into lane `col`.
///
/// Column `col` of a sheared row is the dot product of its own filter with
/// its own window, so the products reduce with three levels of pairwise adds
/// (`addp`) instead of transposing the filters first.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn column_dot_products(
    products: [Simd<i32, WARP_FILTER_TAPS>; WARPED_BLOCK_SIZE],
) -> Simd<i32, WARPED_BLOCK_SIZE> {
    let pairwise = |a: Simd<i32, 4>, b: Simd<i32, 4>| {
        simd_swizzle!(a, b, [0, 2, 4, 6]) + simd_swizzle!(a, b, [1, 3, 5, 7])
    };
    let quads: [Simd<i32, 4>; WARPED_BLOCK_SIZE] = core::array::from_fn(|col| {
        let product = products[col];
        pairwise(
            simd_swizzle!(product, [0, 1, 2, 3]),
            simd_swizzle!(product, [4, 5, 6, 7]),
        )
    });
    let pairs: [Simd<i32, 4>; 4] =
        core::array::from_fn(|k| pairwise(quads[2 * k], quads[2 * k + 1]));
    let low = pairwise(pairs[0], pairs[1]);
    let high = pairwise(pairs[2], pairs[3]);
    simd_swizzle!(low, high, [0, 1, 2, 3, 4, 5, 6, 7])
}

/// Transposes the eight per-column § 7.13.3.20 warp filters into per-tap lanes.
///
/// Reading `columns[col][tap]` one element at a time costs 64 scalar byte loads
/// and 64 vector lane inserts per output row, several times the multiply-
/// accumulate work they feed. Each column's taps are contiguous, so they load
/// as one vector; the three swizzle stages below are the standard 8x8 transpose
/// and lower to `trn1`/`trn2` pairs.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn transpose_warp_taps(
    rows: [Simd<i16, WARP_FILTER_TAPS>; WARPED_BLOCK_SIZE],
) -> [Simd<i16, WARPED_BLOCK_SIZE>; WARP_FILTER_TAPS] {
    const TRN1: [usize; 8] = [0, 8, 2, 10, 4, 12, 6, 14];
    const TRN2: [usize; 8] = [1, 9, 3, 11, 5, 13, 7, 15];
    const TRN1_PAIRS: [usize; 8] = [0, 1, 8, 9, 4, 5, 12, 13];
    const TRN2_PAIRS: [usize; 8] = [2, 3, 10, 11, 6, 7, 14, 15];
    const TRN1_QUADS: [usize; 8] = [0, 1, 2, 3, 8, 9, 10, 11];
    const TRN2_QUADS: [usize; 8] = [4, 5, 6, 7, 12, 13, 14, 15];
    let ones = [
        simd_swizzle!(rows[0], rows[1], TRN1),
        simd_swizzle!(rows[0], rows[1], TRN2),
        simd_swizzle!(rows[2], rows[3], TRN1),
        simd_swizzle!(rows[2], rows[3], TRN2),
        simd_swizzle!(rows[4], rows[5], TRN1),
        simd_swizzle!(rows[4], rows[5], TRN2),
        simd_swizzle!(rows[6], rows[7], TRN1),
        simd_swizzle!(rows[6], rows[7], TRN2),
    ];
    let pairs = [
        simd_swizzle!(ones[0], ones[2], TRN1_PAIRS),
        simd_swizzle!(ones[1], ones[3], TRN1_PAIRS),
        simd_swizzle!(ones[0], ones[2], TRN2_PAIRS),
        simd_swizzle!(ones[1], ones[3], TRN2_PAIRS),
        simd_swizzle!(ones[4], ones[6], TRN1_PAIRS),
        simd_swizzle!(ones[5], ones[7], TRN1_PAIRS),
        simd_swizzle!(ones[4], ones[6], TRN2_PAIRS),
        simd_swizzle!(ones[5], ones[7], TRN2_PAIRS),
    ];
    [
        simd_swizzle!(pairs[0], pairs[4], TRN1_QUADS),
        simd_swizzle!(pairs[1], pairs[5], TRN1_QUADS),
        simd_swizzle!(pairs[2], pairs[6], TRN1_QUADS),
        simd_swizzle!(pairs[3], pairs[7], TRN1_QUADS),
        simd_swizzle!(pairs[0], pairs[4], TRN2_QUADS),
        simd_swizzle!(pairs[1], pairs[5], TRN2_QUADS),
        simd_swizzle!(pairs[2], pairs[6], TRN2_QUADS),
        simd_swizzle!(pairs[3], pairs[7], TRN2_QUADS),
    ]
}

fn build_output(
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &[i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    round1: u32,
    mut store: impl FnMut(usize, Simd<i32, WARPED_BLOCK_SIZE>) -> Result<()>,
) -> Result<()> {
    for row in 0..WARPED_BLOCK_SIZE {
        let i1 = row as i32 - 4;
        let weights = if shear.gamma == 0 {
            let taps = warp_taps(projected.sy4 + shear.delta * i1);
            core::array::from_fn(|pair| {
                let (a, b) = (taps[2 * pair], taps[2 * pair + 1]);
                Simd::from_array(core::array::from_fn(
                    |lane| if lane % 2 == 0 { a } else { b },
                ))
            })
        } else {
            paired_column_taps(core::array::from_fn(|col| {
                let i2 = col as i32 - 4;
                warp_taps(projected.sy4 + shear.gamma * i2 + shear.delta * i1)
            }))
        };
        let sum = paired_rows_sum(intermediate, row, &weights);
        store(row, (sum + Simd::splat(1 << (round1 - 1))) >> round1 as i32)?;
    }
    Ok(())
}

/// The vertical sum of output row `row` with [`paired_column_taps`] weights:
/// each pair of intermediate rows interleaves into one multiply-add.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn paired_rows_sum(
    intermediate: &[i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    row: usize,
    weights: &[Simd<i16, 16>; WARP_FILTER_TAPS / 2],
) -> Simd<i32, WARPED_BLOCK_SIZE> {
    let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
    for (pair, &weights) in weights.iter().enumerate() {
        let rows: Simd<i16, 16> =
            Simd::from_slice(&intermediate[(row + 2 * pair) * WARPED_BLOCK_SIZE..]);
        let rows = simd_swizzle!(rows, [0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15]);
        let product = rows.cast::<i32>() * weights.cast::<i32>();
        sum += simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
            + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15]);
    }
    sum
}

/// Regroups the eight per-column § 7.13.3.20 filters into the weights of tap
/// pairs `2j, 2j + 1`: lanes `2c, 2c + 1` of pair `j` hold column `c`'s two
/// taps, so one multiply-add of two interleaved intermediate rows sums both
/// taps of every column. The regrouping is a 4x4 transpose of 32-bit tap
/// pairs within each 128-bit half.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn paired_column_taps(
    columns: [Simd<i16, WARP_FILTER_TAPS>; WARPED_BLOCK_SIZE],
) -> [Simd<i16, 16>; WARP_FILTER_TAPS / 2] {
    let words = columns.map(|taps| Simd::<i32, 4>::from_ne_bytes(taps.to_ne_bytes()));
    let halves: [Simd<i32, 8>; 4] = core::array::from_fn(|col| {
        simd_swizzle!(words[col], words[col + 4], [0, 1, 2, 3, 4, 5, 6, 7])
    });
    let low = [
        simd_swizzle!(halves[0], halves[1], [0, 8, 1, 9, 4, 12, 5, 13]),
        simd_swizzle!(halves[2], halves[3], [0, 8, 1, 9, 4, 12, 5, 13]),
    ];
    let high = [
        simd_swizzle!(halves[0], halves[1], [2, 10, 3, 11, 6, 14, 7, 15]),
        simd_swizzle!(halves[2], halves[3], [2, 10, 3, 11, 6, 14, 7, 15]),
    ];
    let pairs = [
        simd_swizzle!(low[0], low[1], [0, 1, 8, 9, 4, 5, 12, 13]),
        simd_swizzle!(low[0], low[1], [2, 3, 10, 11, 6, 7, 14, 15]),
        simd_swizzle!(high[0], high[1], [0, 1, 8, 9, 4, 5, 12, 13]),
        simd_swizzle!(high[0], high[1], [2, 3, 10, 11, 6, 7, 14, 15]),
    ];
    pairs.map(|pair| Simd::from_ne_bytes(pair.to_ne_bytes()))
}

/// Reads `LANES` consecutive reference samples as `i16` lanes.
///
/// § 6 Table 6.3 admits only `BitDepth` 8 and 10, so every reference sample is
/// at most 1023 and the reinterpretation preserves the value. Keeping the lanes
/// 16-bit is what lets [`warp_tap_mac`] fold its widening into the multiply,
/// the same narrowing the § 7.13.3.18 sub-pel taps already use.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_source_lanes<T: ReconSample, const LANES: usize>(
    source: &[T],
    start: usize,
) -> Simd<i16, LANES> {
    reference_lanes::<LANES, T>(source, start).cast()
}

/// Builds the eight overlapping tap windows from two loads instead of eight.
///
/// Window `t` is `source[first_col + t ..][..8]`. Reading the whole 16-sample
/// span once and sliding it by lane leaves each window's values untouched, so
/// the § 7.13.3.19 sum is unchanged; only the load shape differs.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_windows<T: ReconSample>(
    source: &[T],
    first_col: usize,
) -> [Simd<i16, WARPED_BLOCK_SIZE>; 8] {
    sliding_windows(
        warp_source_lanes(source, first_col),
        warp_source_lanes(source, first_col + WARPED_BLOCK_SIZE),
    )
}

/// The eight windows `[lo, hi][t..t + 8]` of a 16-sample span.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn sliding_windows(
    lo: Simd<i16, WARPED_BLOCK_SIZE>,
    hi: Simd<i16, WARPED_BLOCK_SIZE>,
) -> [Simd<i16, WARPED_BLOCK_SIZE>; 8] {
    [
        lo,
        simd_swizzle!(lo, hi, [1, 2, 3, 4, 5, 6, 7, 8]),
        simd_swizzle!(lo, hi, [2, 3, 4, 5, 6, 7, 8, 9]),
        simd_swizzle!(lo, hi, [3, 4, 5, 6, 7, 8, 9, 10]),
        simd_swizzle!(lo, hi, [4, 5, 6, 7, 8, 9, 10, 11]),
        simd_swizzle!(lo, hi, [5, 6, 7, 8, 9, 10, 11, 12]),
        simd_swizzle!(lo, hi, [6, 7, 8, 9, 10, 11, 12, 13]),
        simd_swizzle!(lo, hi, [7, 8, 9, 10, 11, 12, 13, 14]),
    ]
}

/// Accumulates one AV2 § 7.13.3.19 warp tap across the eight prediction columns.
///
/// Both factors are 16-bit: samples fit `i16` by [`warp_source_lanes`], and
/// every `Warped_Filters` tap is an `i8`. Sign-extending both sides lets the
/// target fold the widening into one multiply-accumulate instead of widening
/// each operand into 32 bits first.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_tap_mac(
    accumulator: Simd<i32, WARPED_BLOCK_SIZE>,
    samples: Simd<i16, WARPED_BLOCK_SIZE>,
    tap: i16,
) -> Simd<i32, WARPED_BLOCK_SIZE> {
    accumulator + samples.cast::<i32>() * Simd::<i16, WARPED_BLOCK_SIZE>::splat(tap).cast()
}

#[cfg(test)]
fn warped_filter_row(phase: i32) -> &'static [i8; WARP_FILTER_TAPS] {
    let offset =
        ((phase + (1 << (WARPEDDIFF_PREC_BITS - 1))) >> WARPEDDIFF_PREC_BITS) + WARP_FILTER_CENTER;
    debug_assert!((0..WARPED_FILTERS.len() as i32).contains(&offset));
    &WARPED_FILTERS[offset as usize]
}

/// `Warped_Filters` widened to `i16` and zero-padded to a power-of-two length.
static WARPED_TAPS: [[i16; WARP_FILTER_TAPS]; 512] = {
    let mut taps = [[0; WARP_FILTER_TAPS]; 512];
    let mut row = 0;
    while row < WARPED_FILTERS.len() {
        let mut tap = 0;
        while tap < WARP_FILTER_TAPS {
            taps[row][tap] = WARPED_FILTERS[row][tap] as i16;
            tap += 1;
        }
        row += 1;
    }
    taps
};

/// A `Warped_Filters` row as `i16` lanes, indexed with one mask instead of a
/// bounds check: setup-shear keeps every section phase inside the table, where
/// the mask changes nothing.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_taps(phase: i32) -> Simd<i16, WARP_FILTER_TAPS> {
    const BIAS: i32 =
        (1 << (WARPEDDIFF_PREC_BITS - 1)) + (WARP_FILTER_CENTER << WARPEDDIFF_PREC_BITS);
    debug_assert!(
        (0..WARPED_FILTERS.len() as i32).contains(&((phase + BIAS) >> WARPEDDIFF_PREC_BITS))
    );
    let index = ((phase + BIAS) as u32 >> WARPEDDIFF_PREC_BITS) as usize & (WARPED_TAPS.len() - 1);
    Simd::from_array(WARPED_TAPS[index])
}

/// `coefficient_x * src_x + coefficient_y * src_y + translation`, or an
/// overflow error when it leaves `i64`. Each `i32` product fits `i64`, and
/// so does a product plus the translation, so only the last add can overflow.
fn project_coordinate(
    coefficient_x: i32,
    coefficient_y: i32,
    translation: i32,
    src_x: i32,
    src_y: i32,
    context: &'static str,
) -> Result<i64> {
    (i64::from(coefficient_x) * i64::from(src_x) + i64::from(translation))
        .checked_add(i64::from(coefficient_y) * i64::from(src_y))
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn warp_product(left: i32, middle: i32, right: i32, context: &'static str) -> Result<i64> {
    let product = i128::from(left) * i128::from(middle) * i128::from(right);
    i64::try_from(product).map_err(|_| ReconError::ArithmeticOverflow { context })
}

#[cfg(test)]
#[path = "warp_prediction_tests.rs"]
mod tests;
