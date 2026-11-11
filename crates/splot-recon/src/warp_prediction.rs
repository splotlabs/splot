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

#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, Level};
use fearless_simd::{i16x8, i16x16, i32x4, i32x8, i64x2, prelude::*, u8x16, u16x8, u16x16, u64x2};
use splot_core::tables::warp_filter::{EXT_WARPED_FILTERS, WARPED_FILTERS};

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2;
use crate::math::{round2_i32, round2_signed, round2_signed_i32};
use crate::subpel_mc::ReferencePlaneView;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod eight_bit;
mod ext_unit;

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

type Intermediate = [[i16; WARPED_BLOCK_SIZE]; WARP_INTERMEDIATE_ROWS];

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
        let max_sample = self.params.bit_depth.max_sample() as i16;
        let block = [x, y, width, height];
        baseline_simd!(simd => {
            self.clipped_sections(simd, reference, block, output, stride, max_sample)
        })
    }

    #[inline]
    fn clipped_sections<S: Simd, T: ReconSample>(
        &self,
        simd: S,
        reference: &ReferencePlaneView<'_, T>,
        [x, y, width, height]: [usize; 4],
        output: &mut [T],
        stride: usize,
        max_sample: i16,
    ) -> Result<()> {
        if self.translation_two_tap_into(
            simd,
            reference,
            [x, y, width, height],
            output,
            stride,
            max_sample,
        )? {
            return Ok(());
        }
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        let view = reference.as_u8();
        for local_y in (0..height).step_by(WARPED_BLOCK_SIZE) {
            let section_h = (height - local_y).min(WARPED_BLOCK_SIZE);
            let row = WarpPredictBlockParams {
                block_x: x as i32,
                block_y: (y + local_y) as i32,
                ..self.params
            };
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            let written = 0u64;
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            let written = match (&view, output.get_mut(local_y * stride..)) {
                (Some(view), Some(target)) if section_h == WARPED_BLOCK_SIZE => {
                    T::u8_slice_mut(target).map_or(0, |target| {
                        let sections = width / WARPED_BLOCK_SIZE;
                        eight_bit::put_row(view, &row, &self.shear, sections, target, stride)
                    })
                }
                _ => 0,
            };
            for (section, local_x) in (0..width).step_by(WARPED_BLOCK_SIZE).enumerate() {
                if section < 64 && written >> section & 1 != 0 {
                    continue;
                }
                let params = WarpPredictBlockParams {
                    block_x: (x + local_x) as i32,
                    ..row
                };
                let section_w = (width - local_x).min(WARPED_BLOCK_SIZE);
                let store = |row: usize, rounded: i32x8<S>| {
                    if row >= section_h {
                        return Ok(());
                    }
                    let start = (local_y + row) * stride + local_x;
                    let (low, high) = rounded.split();
                    let target = &mut output[start..start + section_w];
                    store_clipped_row(simd, target, low.saturating_narrow(high), max_sample)
                };
                warp_predict_section(
                    simd,
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
    #[inline]
    fn translation_two_tap_into<S: Simd, T: ReconSample>(
        &self,
        simd: S,
        reference: &ReferencePlaneView<'_, T>,
        [x, y, width, height]: [usize; 4],
        output: &mut [T],
        stride: usize,
        max_sample: i16,
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
        let lanes = |row, col| warp_source_lanes(simd, reference.row(row), col);
        let wide = |row, col| lanes(row, col).combine(lanes(row, col + 8));
        let mut col = 0;
        while col + 16 <= width {
            let strip = [first_col + col, first_row, height, high];
            let target = &mut output[col..];
            two_tap_strip(simd, wide, strip, vertical_high, target, stride, max_sample)?;
            col += 16;
        }
        if col < width {
            let strip = [first_col + col, first_row, height, high];
            let target = &mut output[col..];
            two_tap_strip(
                simd,
                lanes,
                strip,
                vertical_high,
                target,
                stride,
                max_sample,
            )?;
        }
        Ok(true)
    }
}

/// Filters one `N`-column strip of a translation block down all `height`
/// rows from `[first_col, first_row, height, high]`, keeping the previous
/// horizontal row in registers for the vertical taps.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn two_tap_strip<S, T, V, const N: usize>(
    simd: S,
    load: impl Fn(usize, usize) -> V,
    [first_col, first_row, height, high]: [usize; 4],
    vertical_high: usize,
    output: &mut [T],
    stride: usize,
    max_sample: i16,
) -> Result<()>
where
    S: Simd,
    T: ReconSample,
    V: SimdInt<S, Element = i16, Array = [i16; N]>,
{
    let horizontal = |row| two_tap_horizontal(|col| load(row, col), first_col, high);
    let mut top = horizontal(first_row);
    for row in 0..height {
        let bottom = horizontal(first_row + row + 1);
        let rounded = two_tap_vertical(top, bottom, vertical_high, INTER_ROUND1_NON_COMPOUND);
        let at = row * stride;
        store_clipped_row(simd, &mut output[at..at + N], rounded, max_sample)?;
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
fn store_clipped_row<S: Simd, T: ReconSample, V: SimdInt<S, Element = i16>>(
    simd: S,
    target: &mut [T],
    rounded: V,
    max_sample: i16,
) -> Result<()> {
    let clipped = rounded.clamp(0, max_sample);
    let rows = clipped.as_slice().as_chunks::<WARPED_BLOCK_SIZE>().0;
    for (target, lanes) in target.chunks_mut(WARPED_BLOCK_SIZE).zip(rows) {
        let lanes = i16x8::load_array_ref(simd, lanes).bitcast::<u16x8<S>>();
        if let Some(target) = T::u16_slice_mut(target) {
            match target.first_chunk_mut() {
                Some(target) => lanes.store_array(target),
                None => store_lanes(target, lanes.to_array()),
            }
        } else if let Some(target) = T::u8_slice_mut(target) {
            let bytes = lanes.narrow(lanes).bitcast::<u64x2<S>>()[0].to_le_bytes();
            match target.first_chunk_mut() {
                Some(target) => *target = bytes,
                None => store_lanes(target, bytes),
            }
        } else {
            for (target, sample) in target.iter_mut().zip(lanes.to_array()) {
                *target = T::try_from_u16(sample)?;
            }
        }
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn store_lanes<E: Copy, const N: usize>(target: &mut [E], lanes: [E; N]) {
    for (target, lane) in target.iter_mut().zip(lanes) {
        *target = lane;
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
    let phase = |s: i32| -> Result<usize> {
        let offs = round2_i32(s, EXT_WARP_ROUND_BITS);
        usize::try_from(offs)
            .ok()
            .filter(|&offs| offs < EXT_WARPED_FILTERS.len())
            .ok_or(ReconError::WarpFilterOffsetOutOfRange { offset: offs })
    };
    let phase_x = phase(sx4)?;
    let phase_y = phase(sy4)?;
    Ok(baseline_simd!(simd => {
        let intermediate = ext_warp_intermediate(simd, reference, params, [ix4, iy4], phase_x);
        ext_unit::vertical(simd, &intermediate, phase_y, round1)
    }))
}

#[inline]
fn ext_warp_intermediate<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    [ix4, iy4]: [i32; 2],
    phase_x: usize,
) -> [i16; 36] {
    let mut intermediate = [0i16; 36];
    let (x, y) = (i64::from(ix4), i64::from(iy4));
    let interior = x - 4 >= i64::from(params.first_x.max(0))
        && x + 4 <= i64::from(params.last_x)
        && x + 5 <= reference.width() as i64
        && y - 4 >= i64::from(params.first_y.max(0))
        && y + 4 <= i64::from(params.last_y);
    let rounded = |sum: i32x4<S>| {
        let sum = (sum + (1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0;
        let lanes = sum.saturating_narrow(sum).to_array();
        [lanes[0], lanes[1], lanes[2], lanes[3]]
    };
    if interior {
        let col = (ix4 - 4) as usize;
        for (k, intermediate) in intermediate.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let row = reference.row((iy4 + k as i32 - 4) as usize);
            let first = warp_source_lanes(simd, row, col);
            let second = warp_source_lanes(simd, row, col + 1);
            *intermediate = rounded(ext_unit::horizontal(simd, first, second, phase_x));
        }
    } else {
        let taps_x = &EXT_WARPED_FILTERS[phase_x];
        let mut cols = [0usize; 9];
        for (offset, col) in cols.iter_mut().enumerate() {
            let x = (ix4 - 4 + offset as i32).clamp(params.first_x, params.last_x);
            *col = (x as usize).min(reference.width() - 1);
        }
        for (k, intermediate) in intermediate.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let row = (iy4 + k as i32 - 4).clamp(params.first_y, params.last_y) as usize;
            let (row, mut window) = (reference.row(row), [0i32; 9]);
            for (sample, &col) in window.iter_mut().zip(&cols) {
                *sample = i32::from(row[col].to_u16());
            }
            let mut sum = i32x4::splat(simd, 0);
            for (m, &tap) in taps_x.iter().enumerate() {
                if let Some(lanes) = window[m..].first_chunk::<4>() {
                    sum += i32x4::load_array_ref(simd, lanes) * tap;
                }
            }
            *intermediate = rounded(sum);
        }
    }
    intermediate
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
    let rows = output.as_chunks_mut::<WARPED_BLOCK_SIZE>().0;
    baseline_simd!(simd => {
        warp_predict_section(simd, reference, params, shear, round1, |row, rounded| {
            rounded.store_array(&mut rows[row]);
            Ok(())
        })
    })
}

/// Runs both § 7.13.3.19 passes for one 8x8 section and hands each rounded
/// output row to `store`.
#[inline]
fn warp_predict_section<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    shear: &Shear,
    round1: u32,
    store: impl FnMut(usize, i32x8<S>) -> Result<()>,
) -> Result<()> {
    let projected = project_section_center(params)?;
    let mut intermediate = [[0i16; WARPED_BLOCK_SIZE]; WARP_INTERMEDIATE_ROWS];
    let origin = interior_warp_source_origin(reference, params, &projected);
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if let Some(source_origin) = origin
        && !(shear.alpha == 0 && shear.beta == 0 && two_tap_phase(projected.sx4).is_some())
        && let Some(reference) = reference.as_u8()
        && let Some(rows) = eight_bit::section(&reference, shear, &projected, source_origin, round1)
    {
        let mut store = store;
        for (row, values) in rows.into_iter().enumerate() {
            store(row, i32x8::load_array(simd, values))?;
        }
        return Ok(());
    }
    if let Some(source_origin) = origin {
        if shear.alpha == 0
            && shear.beta == 0
            && let Some(high) = two_tap_phase(projected.sx4)
        {
            two_tap_intermediate(simd, reference, source_origin, high, &mut intermediate);
        } else if shear.alpha != 0 && shear.beta == 0 {
            row_invariant_intermediate(
                simd,
                reference,
                shear,
                &projected,
                source_origin,
                &mut intermediate,
            );
        } else {
            build_interior_intermediate(
                simd,
                reference,
                shear,
                &projected,
                source_origin,
                &mut intermediate,
            );
        }
    } else {
        build_intermediate(
            simd,
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
        return two_tap_output(simd, high, &intermediate, round1, store);
    }
    if shear.gamma != 0 && shear.delta == 0 {
        return row_invariant_output(simd, shear, &projected, &intermediate, round1, store);
    }
    build_output(simd, shear, &projected, &intermediate, round1, store)
}

/// Runs the § 7.13.3.19 horizontal pass of a section with `beta == 0`, whose
/// rows all use the same eight column filters: they are transposed once for
/// the section instead of once per row.
#[inline]
fn row_invariant_intermediate<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    intermediate: &mut Intermediate,
) {
    let weights = column_weights(
        simd,
        core::array::from_fn(|col| warp_taps(simd, projected.sx4 + shear.alpha * (col as i32 - 4))),
    );
    for (row, intermediate) in intermediate.iter_mut().enumerate() {
        let [lo, hi] = warp_span(simd, reference.row(first_row + row), first_col);
        let sum = tap_sum(sliding_windows(lo, hi), &weights);
        *intermediate = round_intermediate(sum).to_array();
    }
}

/// Runs the § 7.13.3.19 vertical pass of a section with `delta == 0`, whose
/// output rows all use the same eight column filters.
#[inline]
fn row_invariant_output<S: Simd>(
    simd: S,
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &Intermediate,
    round1: u32,
    mut store: impl FnMut(usize, i32x8<S>) -> Result<()>,
) -> Result<()> {
    let weights = column_weights(
        simd,
        core::array::from_fn(|col| warp_taps(simd, projected.sy4 + shear.gamma * (col as i32 - 4))),
    );
    for row in 0..WARPED_BLOCK_SIZE {
        let sum = rows_sum(simd, intermediate, row, &weights);
        store(row, (sum + (1 << (round1 - 1))) >> round1)?;
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
fn two_tap_horizontal<S: Simd, V: SimdInt<S, Element = i16>>(
    load: impl Fn(usize) -> V,
    start: usize,
    high: usize,
) -> V {
    let (main, side) = (load(start + high), load(start + 1 - high));
    (main << (7 - INTER_ROUND0)) + ((side - main + (1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0)
}

/// Runs the § 7.13.3.19 horizontal pass of a two-tap phase over the interior
/// window at `(first_col, first_row)`.
#[inline]
fn two_tap_intermediate<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    (first_col, first_row): (usize, usize),
    high: usize,
    intermediate: &mut Intermediate,
) {
    for (row, intermediate) in intermediate.iter_mut().enumerate() {
        let source = reference.row(first_row + row);
        let start = first_col + TWO_TAP_FIRST;
        let load = |col| warp_source_lanes(simd, source, col);
        *intermediate = two_tap_horizontal(load, start, high).into();
    }
}

/// Runs the § 7.13.3.19 vertical pass of a two-tap phase, which reads only
/// intermediate rows [`TWO_TAP_FIRST`] through `TWO_TAP_FIRST + 8`.
#[inline]
fn two_tap_output<S: Simd>(
    simd: S,
    high: usize,
    intermediate: &Intermediate,
    round1: u32,
    mut store: impl FnMut(usize, i32x8<S>) -> Result<()>,
) -> Result<()> {
    for row in 0..WARPED_BLOCK_SIZE {
        let top = i16x8::load_array_ref(simd, &intermediate[row + TWO_TAP_FIRST]);
        let bottom = i16x8::load_array_ref(simd, &intermediate[row + TWO_TAP_FIRST + 1]);
        let rounded = two_tap_vertical(top, bottom, high, round1);
        store(row, rounded.combine(rounded).widen().0)?;
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
fn two_tap_vertical<S: Simd, V: SimdInt<S, Element = i16>>(
    top: V,
    bottom: V,
    high: usize,
    round1: u32,
) -> V {
    let (main, side) = if high == 0 {
        (top, bottom)
    } else {
        (bottom, top)
    };
    let side = (side - main + (1 << (round1 - 1))) >> 7;
    (main + side) >> (round1 - 7)
}

/// Admits the unclamped interior source origin for one 8x8 warp section.
///
/// The taps reach `last_col`, but [`warp_span`] reads one whole vector past
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

#[inline]
fn build_intermediate<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    [first_x, first_y, last_x, last_y]: [i32; 4],
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &mut Intermediate,
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
        let (halves, _) = window.as_chunks::<WARPED_BLOCK_SIZE>();
        let half = |half: usize| u16x8::load_array_ref(simd, &halves[half]).bitcast();
        let sx = projected.sx4 + shear.beta * i1;
        let span = [half(0), half(1)];
        intermediate[(i1 + 7) as usize] = horizontal_row(simd, span, sx, shear.alpha).into();
    }
}

#[inline]
fn build_interior_intermediate<S: Simd, T: ReconSample>(
    simd: S,
    reference: &ReferencePlaneView<'_, T>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    intermediate: &mut Intermediate,
) {
    for (row, intermediate) in intermediate.iter_mut().enumerate() {
        let sx = projected.sx4 + shear.beta * (row as i32 - 7);
        let span = warp_span(simd, reference.row(first_row + row), first_col);
        *intermediate = horizontal_row(simd, span, sx, shear.alpha).into();
    }
}

/// Runs the § 7.13.3.19 horizontal pass for one intermediate row from its tap
/// windows, whose column `col` uses the filter of phase `sx + alpha * (col - 4)`.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn horizontal_row<S: Simd>(simd: S, [lo, hi]: [i16x8<S>; 2], sx: i32, alpha: i32) -> i16x8<S> {
    let sum = if alpha == 0 {
        tap_sum(sliding_windows(lo, hi), &uniform_weights(simd, sx))
    } else {
        let taps = |col: usize| warp_taps(simd, sx + alpha * (col as i32 - 4));
        let near = lo.combine(lo.slide::<4>(hi));
        let far = hi.combine(hi.slide::<4>(hi));
        let quad = |col, windows: i16x16<S>| {
            windows.pairwise_dot_product(taps(col).combine(taps(col + 4)))
        };
        column_dot_products(
            simd,
            [
                quad(0, near),
                quad(1, near.slide_within_blocks::<1>(far)),
                quad(2, near.slide_within_blocks::<2>(far)),
                quad(3, near.slide_within_blocks::<3>(far)),
            ],
        )
    };
    round_intermediate(sum)
}

#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn round_intermediate<S: Simd>(sum: i32x8<S>) -> i16x8<S> {
    let (low, high) = ((sum + (1i32 << (INTER_ROUND0 - 1))) >> INTER_ROUND0).split();
    low.saturating_narrow(high)
}

/// Sums the pair sums of each column's tap products, `quads[k]` holding columns
/// `k` and `k + 4` in its two 128-bit blocks, into lane `col`.
///
/// Column `col` of a sheared row is the dot product of its own filter with
/// its own window, so the products reduce with two more levels of pairwise
/// adds instead of transposing the filters first.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn column_dot_products<S: Simd>(simd: S, quads: [i32x8<S>; WARPED_BLOCK_SIZE / 2]) -> i32x8<S> {
    let pairs = [
        hadd_blocks(simd, quads[0], quads[1]),
        hadd_blocks(simd, quads[2], quads[3]),
    ];
    hadd_blocks(simd, pairs[0], pairs[1])
}

/// The pair sums `[a0 + a1, a2 + a3, b0 + b1, b2 + b3]` of each 128-bit block.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn hadd_blocks<S: Simd>(simd: S, a: i32x8<S>, b: i32x8<S>) -> i32x8<S> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return i32x8::load_array(simd, avx2_hadd_blocks(avx2, a.to_array(), b.to_array()));
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = simd;
    let pairwise = |a: i32x4<S>, b: i32x4<S>| a.unzip_low(b) + a.unzip_high(b);
    let ((a0, a1), (b0, b1)) = (a.split(), b.split());
    pairwise(a0, b0).combine(pairwise(a1, b1))
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[inline(always)]
    fn avx2_hadd_blocks(avx2: Avx2, a: [i32; 8], b: [i32; 8]) -> [i32; 8] {
        use core::arch::x86_64::{__m256i, _mm256_hadd_epi32};
        let a: __m256i = i32x8::load_array(avx2, a).into();
        let b: __m256i = i32x8::load_array(avx2, b).into();
        i32x8::<Avx2>::simd_from(avx2, _mm256_hadd_epi32(a, b)).to_array()
    }
);

/// Transposes the eight per-column § 7.13.3.20 warp filters into per-tap lanes.
///
/// Reading `columns[col][tap]` one element at a time costs 64 scalar byte loads
/// and 64 vector lane inserts per output row, several times the multiply-
/// accumulate work they feed. Each column's taps are contiguous, so they load
/// as one vector; the three interleave stages below are the standard 8x8
/// transpose of 16-, 32- and 64-bit lanes.
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn transpose_warp_taps<S: Simd>(
    rows: [i16x8<S>; WARPED_BLOCK_SIZE],
) -> [i16x8<S>; WARP_FILTER_TAPS] {
    let words = |a: i16x8<S>, b: i16x8<S>| {
        let (low, high) = a.bitcast::<i32x4<S>>().interleave(b.bitcast::<i32x4<S>>());
        (low.bitcast::<i16x8<S>>(), high.bitcast::<i16x8<S>>())
    };
    let quads = |a: i16x8<S>, b: i16x8<S>| {
        let (low, high) = a.bitcast::<i64x2<S>>().interleave(b.bitcast::<i64x2<S>>());
        [low.bitcast::<i16x8<S>>(), high.bitcast::<i16x8<S>>()]
    };
    let ones: [(i16x8<S>, i16x8<S>); 4] =
        core::array::from_fn(|k| rows[2 * k].interleave(rows[2 * k + 1]));
    let pairs = [
        words(ones[0].0, ones[1].0),
        words(ones[0].1, ones[1].1),
        words(ones[2].0, ones[3].0),
        words(ones[2].1, ones[3].1),
    ];
    let [t0, t1] = quads(pairs[0].0, pairs[2].0);
    let [t2, t3] = quads(pairs[0].1, pairs[2].1);
    let [t4, t5] = quads(pairs[1].0, pairs[3].0);
    let [t6, t7] = quads(pairs[1].1, pairs[3].1);
    [t0, t1, t2, t3, t4, t5, t6, t7]
}

#[inline]
fn build_output<S: Simd>(
    simd: S,
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &Intermediate,
    round1: u32,
    mut store: impl FnMut(usize, i32x8<S>) -> Result<()>,
) -> Result<()> {
    for row in 0..WARPED_BLOCK_SIZE {
        let i1 = row as i32 - 4;
        let weights = if shear.gamma == 0 {
            uniform_weights(simd, projected.sy4 + shear.delta * i1)
        } else {
            column_weights(
                simd,
                core::array::from_fn(|col| {
                    let i2 = col as i32 - 4;
                    warp_taps(simd, projected.sy4 + shear.gamma * i2 + shear.delta * i1)
                }),
            )
        };
        let sum = rows_sum(simd, intermediate, row, &weights);
        store(row, (sum + (1i32 << (round1 - 1))) >> round1)?;
    }
    Ok(())
}

/// The weights of one pass over eight columns: on x86 each column's tap
/// pairs `2j, 2j + 1` side by side for one multiply-add of two interleaved
/// rows, elsewhere each tap across the columns for one widening
/// multiply-accumulate.
#[cfg(target_arch = "x86_64")]
type TapWeights<S> = [i16x16<S>; WARP_FILTER_TAPS / 2];
#[cfg(not(target_arch = "x86_64"))]
type TapWeights<S> = [i16x8<S>; WARP_FILTER_TAPS];

/// The [`TapWeights`] of eight per-column filters.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn column_weights<S: Simd>(simd: S, columns: [i16x8<S>; WARPED_BLOCK_SIZE]) -> TapWeights<S> {
    #[cfg(target_arch = "x86_64")]
    return paired_column_taps(simd, columns);
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = simd;
        transpose_warp_taps(columns)
    }
}

/// The [`TapWeights`] of the filter of `phase` in every column.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn uniform_weights<S: Simd>(simd: S, phase: i32) -> TapWeights<S> {
    #[cfg(target_arch = "x86_64")]
    return tap_pairs(simd, warp_taps(simd, phase));
    #[cfg(not(target_arch = "x86_64"))]
    WARPED_TAPS[filter_index(phase)].map(|tap| i16x8::splat(simd, tap))
}

/// The column sums of `rows[t][c]` weighed by tap `t` of column `c`.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn tap_sum<S: Simd>(rows: [i16x8<S>; WARP_FILTER_TAPS], weights: &TapWeights<S>) -> i32x8<S> {
    #[cfg(target_arch = "x86_64")]
    return paired_sum(rows, weights);
    #[cfg(not(target_arch = "x86_64"))]
    {
        let (mut low, mut high) = rows[0].widening_mul(weights[0]);
        for (row, &weight) in rows.iter().zip(weights).skip(1) {
            let (l, h) = row.widening_mul(weight);
            low += l;
            high += h;
        }
        low.combine(high)
    }
}

/// The vertical sum of output row `row`: on x86 each pair of intermediate
/// rows interleaves into one multiply-add.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn rows_sum<S: Simd>(
    simd: S,
    intermediate: &Intermediate,
    row: usize,
    weights: &TapWeights<S>,
) -> i32x8<S> {
    #[cfg(target_arch = "x86_64")]
    {
        let pair = |pair: usize| {
            let rows = intermediate[row + 2 * pair..]
                .as_flattened()
                .first_chunk::<16>();
            rows.map_or(i16x16::splat(simd, 0), |rows| {
                i16x16::load_array_ref(simd, rows)
            })
            .split()
        };
        let [(r0, r1), (r2, r3), (r4, r5), (r6, r7)] = core::array::from_fn(pair);
        tap_sum([r0, r1, r2, r3, r4, r5, r6, r7], weights)
    }
    #[cfg(not(target_arch = "x86_64"))]
    tap_sum(
        core::array::from_fn(|tap| {
            intermediate
                .get(row + tap)
                .map_or(i16x8::splat(simd, 0), |lanes| {
                    i16x8::load_array_ref(simd, lanes)
                })
        }),
        weights,
    )
}

/// The column sums of `rows[2p][c] * weights[p][2c] + rows[2p + 1][c] *
/// weights[p][2c + 1]` over the four tap pairs `p`, one multiply-add each.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn paired_sum<S: Simd>(
    rows: [i16x8<S>; WARP_FILTER_TAPS],
    weights: &[i16x16<S>; WARP_FILTER_TAPS / 2],
) -> i32x8<S> {
    let product = |pair: usize| {
        interleave_columns(rows[2 * pair], rows[2 * pair + 1]).pairwise_dot_product(weights[pair])
    };
    product(0) + product(1) + (product(2) + product(3))
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn interleave_columns<S: Simd>(a: i16x8<S>, b: i16x8<S>) -> i16x16<S> {
    let (low, high) = a.interleave(b);
    low.combine(high)
}

/// One `Warped_Filters` row's tap pairs `2j, 2j + 1`, each repeated across
/// the eight columns.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn tap_pairs<S: Simd>(simd: S, taps: i16x8<S>) -> [i16x16<S>; WARP_FILTER_TAPS / 2] {
    let pairs = taps.bitcast::<i32x4<S>>().to_array();
    pairs.map(|pair| i32x8::splat(simd, pair).bitcast())
}

/// Regroups the eight per-column § 7.13.3.20 filters into the weights of tap
/// pairs `2j, 2j + 1`: lanes `2c, 2c + 1` of pair `j` hold column `c`'s two
/// taps, so one multiply-add of two interleaved intermediate rows sums both
/// taps of every column. The regrouping is a 4x4 transpose of 32-bit tap
/// pairs within each 128-bit half.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn paired_column_taps<S: Simd>(
    simd: S,
    columns: [i16x8<S>; WARPED_BLOCK_SIZE],
) -> [i16x16<S>; WARP_FILTER_TAPS / 2] {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        let pairs = avx2_paired_column_taps(avx2, columns.map(SimdBase::to_array));
        return pairs.map(|pair| i16x16::load_array(simd, pair));
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = simd;
    let half = |first: usize| {
        let words = |col: usize| columns[first + col].bitcast::<i32x4<S>>();
        let pairs = |a: i32x4<S>, b: i32x4<S>| {
            let (low, high) = a.bitcast::<i64x2<S>>().interleave(b.bitcast::<i64x2<S>>());
            [low.bitcast::<i32x4<S>>(), high.bitcast::<i32x4<S>>()]
        };
        let (zero, one) = words(0).interleave(words(1));
        let (two, three) = words(2).interleave(words(3));
        let [p0, p1] = pairs(zero, two);
        let [p2, p3] = pairs(one, three);
        [p0, p1, p2, p3]
    };
    let (low, high) = (half(0), half(WARPED_BLOCK_SIZE / 2));
    core::array::from_fn(|pair| low[pair].combine(high[pair]).bitcast())
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[inline(always)]
    fn avx2_paired_column_taps(
        avx2: Avx2,
        columns: [[i16; WARPED_BLOCK_SIZE]; WARPED_BLOCK_SIZE],
    ) -> [[i16; 16]; WARP_FILTER_TAPS / 2] {
        use core::arch::x86_64::{
            __m256i, _mm256_unpackhi_epi32, _mm256_unpackhi_epi64, _mm256_unpacklo_epi32,
            _mm256_unpacklo_epi64,
        };
        let half = |col: usize| -> __m256i {
            let column = |col| i16x8::load_array(avx2, columns[col]);
            column(col).combine(column(col + 4)).into()
        };
        let (near, far) = ((half(0), half(1)), (half(2), half(3)));
        let low = (
            _mm256_unpacklo_epi32(near.0, near.1),
            _mm256_unpacklo_epi32(far.0, far.1),
        );
        let high = (
            _mm256_unpackhi_epi32(near.0, near.1),
            _mm256_unpackhi_epi32(far.0, far.1),
        );
        [
            _mm256_unpacklo_epi64(low.0, low.1),
            _mm256_unpackhi_epi64(low.0, low.1),
            _mm256_unpacklo_epi64(high.0, high.1),
            _mm256_unpackhi_epi64(high.0, high.1),
        ]
        .map(|pair| i16x16::<Avx2>::simd_from(avx2, pair).to_array())
    }
);

/// Reads eight consecutive reference samples as `i16` lanes.
///
/// § 6 Table 6.3 admits only `BitDepth` 8 and 10, so every reference sample is
/// at most 1023 and the reinterpretation preserves the value. Keeping the lanes
/// 16-bit is what lets the multiply-adds take them without widening, the same
/// narrowing the § 7.13.3.18 sub-pel taps already use.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_source_lanes<S: Simd, T: ReconSample>(simd: S, source: &[T], start: usize) -> i16x8<S> {
    let Some(source) = source.get(start..start.wrapping_add(8)) else {
        return i16x8::splat(simd, 0);
    };
    if let Some(lanes) = T::u16_slice(source).and_then(<[u16]>::first_chunk::<8>) {
        return u16x8::load_array_ref(simd, lanes).bitcast();
    }
    if let Some(lanes) = T::u8_slice(source).and_then(<[u8]>::first_chunk::<8>) {
        let bytes = u64x2::splat(simd, u64::from_le_bytes(*lanes)).bitcast::<u8x16<S>>();
        return bytes.widen().0.bitcast();
    }
    i16x8::splat(simd, 0)
}

/// Reads the 16-sample span `source[first_col..][..16]` whose sliding windows
/// are the eight tap windows of a row, with one load instead of eight.
///
/// Window `t` is `source[first_col + t ..][..8]`. Reading the whole 16-sample
/// span once and sliding it by lane leaves each window's values untouched, so
/// the § 7.13.3.19 sum is unchanged; only the load shape differs.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_span<S: Simd, T: ReconSample>(simd: S, source: &[T], first_col: usize) -> [i16x8<S>; 2] {
    let zero = i16x8::splat(simd, 0);
    let Some(source) = source.get(first_col..first_col.wrapping_add(16)) else {
        return [zero; 2];
    };
    if let Some(span) = T::u16_slice(source).and_then(<[u16]>::first_chunk::<16>) {
        let (lo, hi) = u16x16::load_array_ref(simd, span).split();
        return [lo.bitcast(), hi.bitcast()];
    }
    if let Some(span) = T::u8_slice(source).and_then(<[u8]>::first_chunk::<16>) {
        let (lo, hi) = u8x16::load_array_ref(simd, span).widen();
        return [lo.bitcast(), hi.bitcast()];
    }
    [zero; 2]
}

/// The eight windows `[lo, hi][t..t + 8]` of a 16-sample span.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn sliding_windows<S: Simd>(lo: i16x8<S>, hi: i16x8<S>) -> [i16x8<S>; 8] {
    [
        lo,
        lo.slide::<1>(hi),
        lo.slide::<2>(hi),
        lo.slide::<3>(hi),
        lo.slide::<4>(hi),
        lo.slide::<5>(hi),
        lo.slide::<6>(hi),
        lo.slide::<7>(hi),
    ]
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
fn warp_taps<S: Simd>(simd: S, phase: i32) -> i16x8<S> {
    i16x8::load_array_ref(simd, &WARPED_TAPS[filter_index(phase)])
}

/// The `Warped_Filters` row of `phase`, masked into the padded table.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn filter_index(phase: i32) -> usize {
    const BIAS: i32 =
        (1 << (WARPEDDIFF_PREC_BITS - 1)) + (WARP_FILTER_CENTER << WARPEDDIFF_PREC_BITS);
    debug_assert!(
        (0..WARPED_FILTERS.len() as i32).contains(&((phase + BIAS) >> WARPEDDIFF_PREC_BITS))
    );
    ((phase + BIAS) as u32 >> WARPEDDIFF_PREC_BITS) as usize & (WARPED_TAPS.len() - 1)
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
