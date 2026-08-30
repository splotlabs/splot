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
    Simd,
    num::{SimdInt, SimdUint},
    simd_swizzle,
};

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2;
use crate::math::{round2_i32, round2_signed, round2_signed_i32};
use crate::subpel_mc::ReferencePlaneView;

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
}

/// Runs the AV2 § 7.13.3.19 block-warp convolution for one single-reference 8x8
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
    let fetch = |row: i32, col: i32| -> i32 {
        let rr = row.clamp(params.first_y, params.last_y);
        let cc = col.clamp(params.first_x, params.last_x);
        reference.sample(rr as usize, cc as usize)
    };
    let mut intermediate = [0i32; 9 * 4];
    for k in -4i32..5 {
        for l in -2i32..2 {
            let mut sum = 0i32;
            for (m, &tap) in taps_x.iter().enumerate() {
                sum += tap * fetch(iy4 + k, ix4 + l - 2 + m as i32);
            }
            intermediate[((k + 4) * 4 + (l + 2)) as usize] = round2_i32(sum, INTER_ROUND0);
        }
    }
    let taps_y = phase(sy4)?;
    let mut output = [0i32; 16];
    for k in -2i32..2 {
        for l in -2i32..2 {
            let mut sum = 0i32;
            for (m, &tap) in taps_y.iter().enumerate() {
                sum += tap * intermediate[((k + m as i32 + 2) * 4 + (l + 2)) as usize];
            }
            output[((k + 2) * 4 + (l + 2)) as usize] = round2_i32(sum, round1);
        }
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

/// prediction section and returns row-major samples after the final `Clip1`.
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
    let projected = project_section_center(params)?;
    let mut intermediate = [0i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
    if let Some(source_origin) = interior_warp_source_origin(reference, params, &projected) {
        build_interior_intermediate(
            reference,
            shear,
            &projected,
            source_origin,
            &mut intermediate,
        );
    } else {
        build_intermediate(reference, params, shear, &projected, &mut intermediate);
    }
    build_output(shear, &projected, &intermediate, round1, output);
    Ok(())
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
    params: &WarpPredictBlockParams,
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    const SOURCE_WINDOW: usize = WARPED_BLOCK_SIZE + WARP_FILTER_TAPS - 1;
    let first_col = projected.x4_int.saturating_sub(7);
    let last_col = projected.x4_int.saturating_add(7);
    let view_last = i32::try_from(reference.width().saturating_sub(1)).unwrap_or(i32::MAX);
    let bounded_first = params.first_x.min(view_last);
    let bounded_last = params.last_x.min(view_last);
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
    let mut window = [0u16; SOURCE_WINDOW];
    for i1 in -7i32..8 {
        let ref_row = (projected.y4_int + i1).clamp(params.first_y, params.last_y) as usize;
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
        for i2 in -4i32..4 {
            let sx = projected.sx4 + shear.alpha * i2 + shear.beta * i1;
            let taps = warped_filter_row(sx);
            let row = (i1 + 7) as usize;
            let col = (i2 + 4) as usize;
            let sum = taps
                .iter()
                .zip(&window[col..col + WARP_FILTER_TAPS])
                .map(|(&tap, &sample)| i32::from(tap) * i32::from(sample))
                .sum();
            intermediate[row * WARPED_BLOCK_SIZE + col] =
                narrow_warp_intermediate(round2_i32(sum, INTER_ROUND0));
        }
    }
}

fn build_interior_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    intermediate: &mut [i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) {
    if shear.alpha == 0 {
        let mut supported = true;
        for row in 0..WARP_INTERMEDIATE_ROWS {
            let i1 = row as i32 - 7;
            let source = reference.row(first_row + row);
            let Some(source) = T::u16_slice(source) else {
                supported = false;
                break;
            };
            let taps = warped_filter_row(projected.sx4 + shear.beta * i1);
            let windows = warp_windows(source, first_col);
            let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
            for (&window, &weight) in windows.iter().zip(taps.iter()) {
                sum = warp_tap_mac(sum, window, weight);
            }
            let rounded = (sum + Simd::splat(1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0 as i32;
            intermediate[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE]
                .copy_from_slice(&rounded.cast::<i16>().to_array()); // splot-copy-ok: store uniform-phase row-wide SIMD warp intermediate
        }
        if supported {
            return;
        }
    }
    for row in 0..WARP_INTERMEDIATE_ROWS {
        let i1 = row as i32 - 7;
        let source = reference.row(first_row + row);
        for col in 0..WARPED_BLOCK_SIZE {
            let i2 = col as i32 - 4;
            let sx = projected.sx4 + shear.alpha * i2 + shear.beta * i1;
            let taps = warped_filter_row(sx);
            let samples = &source[first_col + col..first_col + col + WARP_FILTER_TAPS];
            let sum = taps
                .iter()
                .zip(samples)
                .map(|(&tap, &sample)| i32::from(tap) * i32::from(sample.to_u16()))
                .sum();
            intermediate[row * WARPED_BLOCK_SIZE + col] =
                narrow_warp_intermediate(round2_i32(sum, INTER_ROUND0));
        }
    }
}

#[allow(clippy::needless_range_loop, reason = "transpose warp taps for SIMD")]
fn build_output(
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &[i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    round1: u32,
    output: &mut [i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE],
) {
    if shear.gamma == 0 {
        for row in 0..WARPED_BLOCK_SIZE {
            let i1 = row as i32 - 4;
            let taps = warped_filter_row(projected.sy4 + shear.delta * i1);
            let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
            for (tap, &weight) in taps.iter().enumerate() {
                let samples = Simd::from_slice(&intermediate[(row + tap) * WARPED_BLOCK_SIZE..])
                    .cast::<i32>();
                sum += Simd::splat(i32::from(weight)) * samples;
            }
            let rounded = (sum + Simd::splat(1 << (round1 - 1))) >> round1 as i32;
            output[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE]
                .copy_from_slice(&rounded.to_array()); // splot-copy-ok: publish uniform-phase row-wide SIMD warp output
        }
        return;
    }
    for row in 0..WARPED_BLOCK_SIZE {
        let i1 = row as i32 - 4;
        let columns: [&'static [i8; WARP_FILTER_TAPS]; WARPED_BLOCK_SIZE] =
            core::array::from_fn(|col| {
                let i2 = col as i32 - 4;
                warped_filter_row(projected.sy4 + shear.gamma * i2 + shear.delta * i1)
            });
        let mut sum = Simd::<i32, WARPED_BLOCK_SIZE>::splat(0);
        for tap in 0..WARP_FILTER_TAPS {
            let taps = Simd::<i16, WARPED_BLOCK_SIZE>::from_array(core::array::from_fn(|col| {
                i16::from(columns[col][tap])
            }));
            let samples =
                Simd::from_slice(&intermediate[(row + tap) * WARPED_BLOCK_SIZE..]).cast::<i32>();
            sum += taps.cast::<i32>() * samples;
        }
        let rounded = (sum + Simd::splat(1 << (round1 - 1))) >> round1 as i32;
        output[row * WARPED_BLOCK_SIZE..(row + 1) * WARPED_BLOCK_SIZE]
            .copy_from_slice(&rounded.to_array()); // splot-copy-ok: publish row-wide SIMD warp output
    }
}

/// Reads eight consecutive reference samples as `i16` lanes.
///
/// § 6 Table 6.3 admits only `BitDepth` 8 and 10, so every reference sample is
/// at most 1023 and the reinterpretation preserves the value. Keeping the lanes
/// 16-bit is what lets [`warp_tap_mac`] fold its widening into the multiply,
/// the same narrowing the § 7.13.3.18 sub-pel taps already use.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_source_lanes(source: &[u16], start: usize) -> Simd<i16, WARPED_BLOCK_SIZE> {
    Simd::<u16, WARPED_BLOCK_SIZE>::from_slice(&source[start..]).cast()
}

/// Builds the eight overlapping tap windows from two loads instead of eight.
///
/// Window `t` is `source[first_col + t ..][..8]`. Reading the whole 16-sample
/// span once and sliding it by lane leaves each window's values untouched, so
/// the § 7.13.3.19 sum is unchanged; only the load shape differs.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn warp_windows(source: &[u16], first_col: usize) -> [Simd<i16, WARPED_BLOCK_SIZE>; 8] {
    let lo = warp_source_lanes(source, first_col);
    let hi = warp_source_lanes(source, first_col + WARPED_BLOCK_SIZE);
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
    tap: i8,
) -> Simd<i32, WARPED_BLOCK_SIZE> {
    accumulator
        + samples.cast::<i32>() * Simd::<i16, WARPED_BLOCK_SIZE>::splat(i16::from(tap)).cast()
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "10-bit warp filter bounds are -6010..=22378 after InterRound0"
)]
fn narrow_warp_intermediate(value: i32) -> i16 {
    debug_assert!((-6010..=22378).contains(&value));
    value as i16
}

fn warped_filter_row(phase: i32) -> &'static [i8; WARP_FILTER_TAPS] {
    let offset = round2_i32(phase, WARPEDDIFF_PREC_BITS) + WARP_FILTER_CENTER;
    debug_assert!((0..WARPED_FILTERS.len() as i32).contains(&offset));
    &WARPED_FILTERS[offset as usize]
}

fn project_coordinate(
    coefficient_x: i32,
    coefficient_y: i32,
    translation: i32,
    src_x: i32,
    src_y: i32,
    context: &'static str,
) -> Result<i64> {
    let projected = i128::from(coefficient_x) * i128::from(src_x)
        + i128::from(coefficient_y) * i128::from(src_y)
        + i128::from(translation);
    i64::try_from(projected).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn warp_product(left: i32, middle: i32, right: i32, context: &'static str) -> Result<i64> {
    let product = i128::from(left) * i128::from(middle) * i128::from(right);
    i64::try_from(product).map_err(|_| ReconError::ArithmeticOverflow { context })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn build_ref(width: usize, height: usize) -> Vec<u16> {
        let mut samples = vec![0u16; width * height];
        for row in 0..height {
            for col in 0..width {
                samples[row * width + col] = ((row * 17 + col * 11 + (row ^ col) * 3) % 251) as u16;
            }
        }
        samples
    }

    fn default_params(
        block_x: i32,
        block_y: i32,
        ref_w: i32,
        ref_h: i32,
    ) -> WarpPredictBlockParams {
        WarpPredictBlockParams {
            warp_params: IDENTITY_WARP_PARAMS,
            block_x,
            block_y,
            subsampling_x: 0,
            subsampling_y: 0,
            reference_scale_x: 1 << REF_SCALE_SHIFT,
            reference_scale_y: 1 << REF_SCALE_SHIFT,
            first_x: 0,
            first_y: 0,
            last_x: ref_w - 1,
            last_y: ref_h - 1,
            bit_depth: BitDepth::Eight,
        }
    }

    fn reference_warp_8x8(
        samples: &[u16],
        ref_w: usize,
        ref_h: usize,
        params: &WarpPredictBlockParams,
    ) -> Vec<u16> {
        let clip = |lo: i32, hi: i32, value: i32| value.max(lo).min(hi);
        let round = |value: i64, shift: u32| {
            if shift == 0 {
                value
            } else {
                (value + (1 << (shift - 1))) >> shift
            }
        };
        let fetch = |row: i32, col: i32| {
            let row = clip(0, ref_h as i32 - 1, row) as usize;
            let col = clip(0, ref_w as i32 - 1, col) as usize;
            i64::from(samples[row * ref_w + col])
        };

        let shear = setup_shear(params.warp_params).unwrap();
        let projected = project_section_center(params).unwrap();
        let mut intermediate = [0i64; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
        for i1 in -7i32..8 {
            for i2 in -4i32..4 {
                let sx = projected.sx4 + shear.alpha * i2 + shear.beta * i1;
                let offset = (round(i64::from(sx), WARPEDDIFF_PREC_BITS)
                    + i64::from(WARP_FILTER_CENTER)) as usize;
                let taps = &WARPED_FILTERS[offset];
                let mut sum = 0i64;
                for (i3, &tap) in taps.iter().enumerate() {
                    let rr = clip(params.first_y, params.last_y, projected.y4_int + i1);
                    let cc = clip(
                        params.first_x,
                        params.last_x,
                        projected.x4_int + i2 - 3 + i3 as i32,
                    );
                    sum += i64::from(tap) * fetch(rr, cc);
                }
                intermediate[(i1 + 7) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                    round(sum, INTER_ROUND0);
            }
        }

        let max_sample = i64::from(params.bit_depth.max_sample());
        let mut out = vec![0u16; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        for i1 in -4i32..4 {
            for i2 in -4i32..4 {
                let sy = projected.sy4 + shear.gamma * i2 + shear.delta * i1;
                let offset = (round(i64::from(sy), WARPEDDIFF_PREC_BITS)
                    + i64::from(WARP_FILTER_CENTER)) as usize;
                let taps = &WARPED_FILTERS[offset];
                let mut sum = 0i64;
                for (i3, &tap) in taps.iter().enumerate() {
                    let row = (i1 + i3 as i32 + 4) as usize;
                    let col = (i2 + 4) as usize;
                    sum += i64::from(tap) * intermediate[row * WARPED_BLOCK_SIZE + col];
                }
                let pred = round2(sum, INTER_ROUND1_NON_COMPOUND);
                out[(i1 + 4) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                    pred.clamp(0, max_sample) as u16;
            }
        }
        out
    }

    #[test]
    fn warped_filters_table_shape_and_sums() {
        assert_eq!(WARPED_FILTERS.len(), 449);
        for (index, row) in WARPED_FILTERS.iter().enumerate() {
            assert_eq!(row.len(), WARP_FILTER_TAPS);
            assert_eq!(
                row.iter().map(|&tap| i32::from(tap)).sum::<i32>(),
                128,
                "row {index}"
            );
        }
    }

    #[test]
    fn identity_warp_flat_reference_returns_flat_block() {
        let ref_w = 16usize;
        let ref_h = 16usize;
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(4, 4, ref_w as i32, ref_h as i32);

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i32::from(params.bit_depth.max_sample()),
        );
        assert_eq!(out, vec![77u16; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
    }

    #[test]
    fn compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
        let (ref_w, ref_h) = (16usize, 16usize);
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(4, 4, ref_w as i32, ref_h as i32);

        let out = warp_predict_block(&view, &params, true).unwrap();
        assert_eq!(out, vec![16 * 77i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
    }

    #[test]
    fn compound_intermediate_affine_matches_round7_spec_trace() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
        params.warp_params = [
            1 << WARPEDMODEL_PREC_BITS,
            -(2 << WARPEDMODEL_PREC_BITS),
            (1 << WARPEDMODEL_PREC_BITS) + 256,
            -128,
            192,
            (1 << WARPEDMODEL_PREC_BITS) - 320,
        ];

        let shear = setup_shear(params.warp_params).unwrap();
        let projected = project_section_center(&params).unwrap();
        let mut intermediate = [0i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
        build_intermediate(&view, &params, &shear, &projected, &mut intermediate);
        let mut want = vec![0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        for i1 in -4i32..4 {
            for i2 in -4i32..4 {
                let sy = projected.sy4 + shear.gamma * i2 + shear.delta * i1;
                let taps = &WARPED_FILTERS[(round2(i64::from(sy), WARPEDDIFF_PREC_BITS)
                    + i64::from(WARP_FILTER_CENTER))
                    as usize];
                let mut sum = 0i64;
                for (i3, &tap) in taps.iter().enumerate() {
                    let row = (i1 + i3 as i32 + 4) as usize;
                    sum += i64::from(tap)
                        * i64::from(intermediate[row * WARPED_BLOCK_SIZE + (i2 + 4) as usize]);
                }
                want[(i1 + 4) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                    round2(sum, INTER_ROUND1_COMPOUND) as i32;
            }
        }

        let got = warp_predict_block(&view, &params, true).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn fixed_block_output_matches_vec_wrapper() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
        params.warp_params = [
            1 << WARPEDMODEL_PREC_BITS,
            -(2 << WARPEDMODEL_PREC_BITS),
            (1 << WARPEDMODEL_PREC_BITS) + 256,
            -128,
            192,
            (1 << WARPEDMODEL_PREC_BITS) - 320,
        ];

        for is_compound in [false, true] {
            let expected = warp_predict_block(&view, &params, is_compound).unwrap();
            let mut output = [i32::MIN; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
            warp_predict_block_into(&view, &params, is_compound, &mut output).unwrap();
            assert_eq!(output.as_slice(), expected);
        }
    }

    #[test]
    fn ext_compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(8, 12, ref_w as i32, ref_h as i32);
        let out = ext_warp_predict_unit(&view, &params, 0, 0, true).unwrap();
        assert_eq!(out, [16 * 77i32; 16]);
    }

    #[test]
    fn scaled_ext_warp_identity_samples_the_reference_at_scaled_steps() {
        let (ref_w, ref_h) = (16usize, 16usize);
        let samples = (0..ref_h)
            .flat_map(|row| (0..ref_w).map(move |col| (row * ref_w + col) as u16))
            .collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
        params.reference_scale_x = 2 << REF_SCALE_SHIFT;
        params.reference_scale_y = 2 << REF_SCALE_SHIFT;

        let out = ext_warp_predict_unit(&view, &params, 0, 0, false).unwrap();
        assert_eq!(
            out,
            [0, 2, 4, 6, 32, 34, 36, 38, 64, 66, 68, 70, 96, 98, 100, 102]
        );
    }

    #[test]
    fn integer_translation_warp_matches_spec_trace() {
        let ref_w = 24usize;
        let ref_h = 24usize;
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(6, 5, ref_w as i32, ref_h as i32);
        params.warp_params[0] = 2 << WARPEDMODEL_PREC_BITS;
        params.warp_params[1] = 3 << WARPEDMODEL_PREC_BITS;

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i32::from(params.bit_depth.max_sample()),
        );
        let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want);
    }

    #[test]
    fn affine_warp_nontrivial_case_matches_spec_trace() {
        let ref_w = 32usize;
        let ref_h = 32usize;
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
        params.warp_params = [
            1 << WARPEDMODEL_PREC_BITS,
            -(2 << WARPEDMODEL_PREC_BITS),
            (1 << WARPEDMODEL_PREC_BITS) + 256,
            -128,
            192,
            (1 << WARPEDMODEL_PREC_BITS) - 320,
        ];

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i32::from(params.bit_depth.max_sample()),
        );
        let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want);
        assert_eq!(&out[..8], &[25, 33, 41, 49, 58, 115, 121, 129]);
    }

    /// The interior 16-bit-lane pass must equal the clamped scalar pass
    /// everywhere the clamps are inactive, including at the largest legal
    /// 10-bit sample, where reinterpreting `u16` as `i16` would change sign if
    /// § 6 Table 6.3 admitted a wider sample. Every section here is centred far
    /// enough inside the reference that `interior_warp_source_origin` accepts it.
    #[test]
    fn interior_intermediate_matches_the_clamped_pass_over_the_ten_bit_range() {
        let (ref_w, ref_h) = (64usize, 64usize);
        let samples = (0..ref_w * ref_h)
            .map(|index| match index % 7 {
                0 => 1023,
                1 => 1022,
                2 => 0,
                _ => ((index * 149) % 1024) as u16,
            })
            .collect::<Vec<u16>>();
        assert!(samples.contains(&1023));
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

        let models = [
            IDENTITY_WARP_PARAMS,
            [
                0,
                0,
                1 << WARPEDMODEL_PREC_BITS,
                384,
                -256,
                1 << WARPEDMODEL_PREC_BITS,
            ],
            [
                1 << WARPEDMODEL_PREC_BITS,
                -(2 << WARPEDMODEL_PREC_BITS),
                (1 << WARPEDMODEL_PREC_BITS) + 256,
                -128,
                192,
                (1 << WARPEDMODEL_PREC_BITS) - 320,
            ],
        ];
        let mut covered = (false, false);
        for warp_params in models {
            for (block_x, block_y) in [(24, 24), (32, 24), (24, 32)] {
                let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
                params.warp_params = warp_params;
                params.bit_depth = BitDepth::Ten;
                let shear = setup_shear(warp_params).unwrap();
                let projected = project_section_center(&params).unwrap();
                let origin = interior_warp_source_origin(&view, &params, &projected).unwrap();
                if shear.alpha == 0 {
                    covered.0 = true;
                } else {
                    covered.1 = true;
                }

                let mut fast = [0i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
                build_interior_intermediate(&view, &shear, &projected, origin, &mut fast);
                let mut want = [0i16; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
                build_intermediate(&view, &params, &shear, &projected, &mut want);
                assert_eq!(
                    fast, want,
                    "model {warp_params:?} at ({block_x}, {block_y})"
                );
            }
        }
        assert_eq!(covered, (true, true), "both shear column cases exercised");
    }

    /// [`warp_windows`] reads a whole vector past `first_col + WARPED_BLOCK_SIZE`,
    /// one sample beyond the taps' own reach, so admission must reserve the
    /// column after `last_col`. Without the reservation the admitted section at
    /// the final column reads off the end of its row.
    #[test]
    fn interior_admission_reserves_the_column_after_the_last_tap() {
        let (stride, ref_h) = (64usize, 64usize);
        let samples = vec![512u16; stride * ref_h];
        let params = default_params(24, 24, stride as i32, ref_h as i32);
        let projected = project_section_center(&params).unwrap();
        let last_col = usize::try_from(projected.x4_int + 7).unwrap();

        let tight =
            ReferencePlaneView::from_strided(&samples, stride, last_col + 1, ref_h).unwrap();
        assert_eq!(
            interior_warp_source_origin(&tight, &params, &projected),
            None,
            "a section whose taps end at the final column must not be admitted"
        );

        let roomy =
            ReferencePlaneView::from_strided(&samples, stride, last_col + 2, ref_h).unwrap();
        let (first_col, first_row) =
            interior_warp_source_origin(&roomy, &params, &projected).unwrap();
        assert!(
            first_col + 2 * WARPED_BLOCK_SIZE <= roomy.row(first_row).len(),
            "the admitted origin must leave a full two-vector window in the row"
        );
    }

    #[test]
    fn border_extension_clips_reference_reads() {
        let ref_w = 12usize;
        let ref_h = 12usize;
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let samples_u8 = samples
            .iter()
            .map(|&sample| sample as u8)
            .collect::<Vec<_>>();
        let view_u8 = ReferencePlaneView::new(&samples_u8, ref_w, ref_h).unwrap();
        for (translate_x, translate_y) in [(-2, -1), (-20, -20), (6, 5), (20, 20)] {
            let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
            params.warp_params[0] = translate_x << WARPEDMODEL_PREC_BITS;
            params.warp_params[1] = translate_y << WARPEDMODEL_PREC_BITS;
            let projected = project_section_center(&params).unwrap();
            assert_eq!(
                interior_warp_source_origin(&view, &params, &projected),
                None
            );

            let out = crate::math::clip1_predicted_samples(
                warp_predict_block(&view, &params, false).unwrap(),
                i32::from(params.bit_depth.max_sample()),
            );
            let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
            assert_eq!(out, want, "translation ({translate_x}, {translate_y})");
            let out_u8 = crate::math::clip1_predicted_samples(
                warp_predict_block(&view_u8, &params, false).unwrap(),
                i32::from(params.bit_depth.max_sample()),
            );
            assert_eq!(
                out_u8, want,
                "u8 translation ({translate_x}, {translate_y})"
            );
        }
    }

    #[test]
    fn rejects_invalid_reference_bounds() {
        let samples = vec![0u16; 64];
        let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
        let mut params = default_params(0, 0, 8, 8);
        params.first_x = 4;
        params.last_x = 3;
        assert!(matches!(
            warp_predict_block(&view, &params, false),
            Err(ReconError::WarpReferenceBoundsInvalid { .. })
        ));
    }

    #[test]
    fn rejects_unsupported_subsampling() {
        let samples = vec![0u16; 64];
        let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
        let mut params = default_params(0, 0, 8, 8);
        params.subsampling_x = 2;
        assert!(matches!(
            warp_predict_block(&view, &params, false),
            Err(ReconError::WarpSubsamplingUnsupported { .. })
        ));
    }

    #[test]
    fn rejects_invalid_shear_without_predicting() {
        let samples = vec![0u16; 64];
        let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
        let mut params = default_params(0, 0, 8, 8);
        params.warp_params[3] = 1 << WARPEDMODEL_PREC_BITS;

        assert!(matches!(
            warp_predict_block(&view, &params, false),
            Err(ReconError::WarpInvalidShear { .. })
        ));
    }

    #[test]
    fn ext_warp_identity_reproduces_the_colocated_unit() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(8, 12, ref_w as i32, ref_h as i32);
        for (i4, j4) in [(0usize, 0usize), (1, 1)] {
            let out = crate::math::clip1_predicted_samples(
                ext_warp_predict_unit(&view, &params, i4, j4, false)
                    .unwrap()
                    .into(),
                i32::from(params.bit_depth.max_sample()),
            );
            for r in 0..4 {
                for c in 0..4 {
                    let src = samples[(12 + i4 * 4 + r) * ref_w + 8 + j4 * 4 + c];
                    assert_eq!(out[r * 4 + c], src, "i4={i4} j4={j4} r={r} c={c}");
                }
            }
        }
    }

    #[test]
    fn ext_warp_integer_translation_shifts_the_source_window() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(8, 12, ref_w as i32, ref_h as i32);
        params.warp_params[0] = 3 << WARPEDMODEL_PREC_BITS;
        params.warp_params[1] = 2 << WARPEDMODEL_PREC_BITS;
        let out = crate::math::clip1_predicted_samples(
            ext_warp_predict_unit(&view, &params, 0, 1, false)
                .unwrap()
                .into(),
            i32::from(params.bit_depth.max_sample()),
        );
        for r in 0..4 {
            for c in 0..4 {
                let src = samples[(12 + 2 + r) * ref_w + 8 + 4 + 3 + c];
                assert_eq!(out[r * 4 + c], src, "r={r} c={c}");
            }
        }
    }
}
