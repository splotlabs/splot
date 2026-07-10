// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.19 block-warp prediction kernel.
//!
//! This module implements the narrow source-backed single-reference block-warp
//! predictor used after the caller has already decoded `IsWarp` / `WarpMv` and
//! derived the six affine warp parameters. It owns only the § 7.13.3.19 8x8
//! convolution and the § 7.13.3.21 setup-shear check; model derivation, motion
//! mode selection, `SubMvs` storage, compound blending, extended warp, scaled
//! reference fallback, and decode entropy remain caller responsibilities.
//!
//! Feature tracking: `RECON-SUBPEL-MC`.

use splot_core::tables::warp_filter::{EXT_WARPED_FILTERS, WARPED_FILTERS};

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::intra_dc_math::resolve_divisor;
use crate::math::{clip3, round2, round2_signed};
use crate::subpel_mc::ReferencePlaneView;

/// AV2 § 7.13.3.19 block-warp predictor side length in samples.
pub const WARPED_BLOCK_SIZE: usize = 8;

const WARPEDMODEL_PREC_BITS: u32 = 16;
const WARPEDDIFF_PREC_BITS: u32 = 10;
const WARP_PARAM_REDUCE_BITS: u32 = 6;
const WARPEDPIXEL_PREC_SHIFTS: i64 = 1 << 6;
const WARP_FILTER_CENTER: i64 = 3 * WARPEDPIXEL_PREC_SHIFTS;
const INTER_ROUND0: u32 = 3;
const INTER_ROUND1_NON_COMPOUND: u32 = 11;
const INTER_ROUND1_COMPOUND: u32 = 7;
const WARP_INTERMEDIATE_ROWS: usize = 15;
const WARP_FILTER_TAPS: usize = 8;
const WARP_PARAM_CLIP_LOW: i64 = -32_768;
const WARP_PARAM_CLIP_HIGH: i64 = 32_767 - (1 << (WARP_PARAM_REDUCE_BITS - 1));
const WARP_SHEAR_LIMIT: i128 = 3i128 << WARPEDMODEL_PREC_BITS;

/// AV2 `Default_Warp_Params[6]` identity affine model (§ 7.12.2.11 / § 7.13.3.19).
pub const IDENTITY_WARP_PARAMS: [i64; 6] = [
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
    pub warp_params: [i64; 6],
    /// Top-left x coordinate of this 8x8 prediction section in the current plane.
    pub block_x: i64,
    /// Top-left y coordinate of this 8x8 prediction section in the current plane.
    pub block_y: i64,
    /// Plane horizontal chroma subsampling (`subX`): `0` for luma, otherwise the
    /// sequence `SubsamplingX` value.
    pub subsampling_x: u8,
    /// Plane vertical chroma subsampling (`subY`): `0` for luma, otherwise the
    /// sequence `SubsamplingY` value.
    pub subsampling_y: u8,
    /// Inclusive left reference-sampling bound (`firstX`).
    pub first_x: i64,
    /// Inclusive top reference-sampling bound (`firstY`).
    pub first_y: i64,
    /// Inclusive right reference-sampling bound (`lastX`).
    pub last_x: i64,
    /// Inclusive bottom reference-sampling bound (`lastY`).
    pub last_y: i64,
    /// Active bit depth used by the final § 4.8 `Clip1` clamp.
    pub bit_depth: BitDepth,
}

/// Runs the AV2 § 7.13.3.19 block-warp convolution for one single-reference 8x8
/// AV2 § 3 `EXT_WARP_TAPS`.
const EXT_WARP_TAPS: usize = 6;
/// AV2 § 3 `EXT_WARP_ROUND_BITS` = `WARPEDMODEL_PREC_BITS - EXT_WARP_PHASES_LOG2`.
const EXT_WARP_ROUND_BITS: u32 = 10;

/// Reports whether the § 7.13.3.21 setup-shear process accepts a warp model,
/// deciding the § 7.13.3.15 `skipPred` fallback to the extended block warp.
#[must_use]
pub fn warp_shear_is_valid(warp_params: [i64; 6]) -> bool {
    setup_shear(warp_params).is_ok()
}

/// AV2 § 7.13.3.20 extended block warp (unscaled arm): predicts one 4x4 unit
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
#[allow(clippy::too_many_arguments)]
pub fn ext_warp_predict_unit<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    i4: usize,
    j4: usize,
    is_compound: bool,
) -> Result<Vec<i32>> {
    let round1 = if is_compound {
        INTER_ROUND1_COMPOUND
    } else {
        INTER_ROUND1_NON_COMPOUND
    };
    validate_params(params)?;
    let sub_x = u32::from(params.subsampling_x);
    let sub_y = u32::from(params.subsampling_y);
    let src_x = (params.block_x + (j4 as i64) * 4 + 2) << sub_x;
    let src_y = (params.block_y + (i4 as i64) * 4 + 2) << sub_y;
    let dst_x =
        params.warp_params[2] * src_x + params.warp_params[3] * src_y + params.warp_params[0];
    let dst_y =
        params.warp_params[4] * src_x + params.warp_params[5] * src_y + params.warp_params[1];
    let x4 = dst_x >> sub_x;
    let y4 = dst_y >> sub_y;
    let ix4 = x4 >> WARPEDMODEL_PREC_BITS;
    let sx4 = x4 & ((1 << WARPEDMODEL_PREC_BITS) - 1);
    let iy4 = y4 >> WARPEDMODEL_PREC_BITS;
    let sy4 = y4 & ((1 << WARPEDMODEL_PREC_BITS) - 1);
    let phase = |s: i64| -> Result<&'static [i32; EXT_WARP_TAPS]> {
        let offs = round2(s, EXT_WARP_ROUND_BITS);
        usize::try_from(offs)
            .ok()
            .and_then(|offs| EXT_WARPED_FILTERS.get(offs))
            .ok_or(ReconError::WarpFilterOffsetOutOfRange { offset: offs })
    };
    let taps_x = phase(sx4)?;
    let fetch = |row: i64, col: i64| -> i64 {
        let rr = clip3(params.first_y, params.last_y, row);
        let cc = clip3(params.first_x, params.last_x, col);
        reference.sample(rr as usize, cc as usize)
    };
    let mut intermediate = [0i64; 9 * 4];
    for k in -4i64..5 {
        for l in -2i64..2 {
            let mut sum = 0i64;
            for (m, &tap) in taps_x.iter().enumerate() {
                sum += i64::from(tap) * fetch(iy4 + k, ix4 + l - 2 + m as i64);
            }
            intermediate[((k + 4) * 4 + (l + 2)) as usize] = round2(sum, INTER_ROUND0);
        }
    }
    let taps_y = phase(sy4)?;
    let mut out = vec![0i32; 16];
    for k in -2i64..2 {
        for l in -2i64..2 {
            let mut sum = 0i64;
            for (m, &tap) in taps_y.iter().enumerate() {
                sum += i64::from(tap) * intermediate[((k + m as i64 + 2) * 4 + (l + 2)) as usize];
            }
            out[((k + 2) * 4 + (l + 2)) as usize] = round2(sum, round1) as i32;
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
    let round1 = if is_compound {
        INTER_ROUND1_COMPOUND
    } else {
        INTER_ROUND1_NON_COMPOUND
    };
    validate_params(params)?;
    let shear = setup_shear(params.warp_params)?;
    let projected = project_section_center(params)?;
    let mut intermediate = [0i32; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
    build_intermediate(reference, params, &shear, &projected, &mut intermediate)?;
    build_output(&shear, &projected, &intermediate, round1)
}

fn validate_params(params: &WarpPredictBlockParams) -> Result<()> {
    if params.subsampling_x > 1 || params.subsampling_y > 1 {
        return Err(ReconError::WarpSubsamplingUnsupported {
            subsampling_x: params.subsampling_x,
            subsampling_y: params.subsampling_y,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Shear {
    alpha: i64,
    beta: i64,
    gamma: i64,
    delta: i64,
}

fn setup_shear(warp_params: [i64; 6]) -> Result<Shear> {
    let alpha0 = clip3(
        WARP_PARAM_CLIP_LOW,
        WARP_PARAM_CLIP_HIGH,
        warp_params[2]
            .checked_sub(1 << WARPEDMODEL_PREC_BITS)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "block-warp alpha setup",
            })?,
    );
    let beta0 = clip3(WARP_PARAM_CLIP_LOW, WARP_PARAM_CLIP_HIGH, warp_params[3]);
    let (div_shift, div_factor) = resolve_signed_divisor(warp_params[2])?;
    let v_product = checked_product_to_i64(
        &[
            i128::from(warp_params[4]),
            1i128 << WARPEDMODEL_PREC_BITS,
            i128::from(div_factor),
        ],
        "block-warp gamma setup",
    )?;
    let gamma0 = clip3(
        WARP_PARAM_CLIP_LOW,
        WARP_PARAM_CLIP_HIGH,
        round2_signed(v_product, div_shift),
    );

    let w_product = checked_product_to_i64(
        &[
            i128::from(warp_params[3]),
            i128::from(warp_params[4]),
            i128::from(div_factor),
        ],
        "block-warp delta setup",
    )?;
    let rounded_w = round2_signed(w_product, div_shift);
    let delta_input = warp_params[5]
        .checked_sub(rounded_w)
        .and_then(|value| value.checked_sub(1 << WARPEDMODEL_PREC_BITS))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "block-warp delta setup",
        })?;
    let delta0 = clip3(WARP_PARAM_CLIP_LOW, WARP_PARAM_CLIP_HIGH, delta_input);

    let shear = Shear {
        alpha: round2_signed(alpha0, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS,
        beta: round2_signed(beta0, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS,
        gamma: round2_signed(gamma0, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS,
        delta: round2_signed(delta0, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS,
    };
    if 4 * i128::from(shear.alpha).abs() + 7 * i128::from(shear.beta).abs() >= WARP_SHEAR_LIMIT
        || 4 * i128::from(shear.gamma).abs() + 4 * i128::from(shear.delta).abs() >= WARP_SHEAR_LIMIT
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

fn resolve_signed_divisor(d: i64) -> Result<(u32, i64)> {
    if d == 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "block-warp divisor resolution",
        });
    }
    let (shift, factor) = resolve_divisor(d.unsigned_abs())?;
    let factor = if d < 0 {
        -i64::from(factor)
    } else {
        i64::from(factor)
    };
    Ok((u32::from(shift), factor))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectedCenter {
    x4_int: i64,
    y4_int: i64,
    sx4: i64,
    sy4: i64,
}

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
    let dst_x = checked_sum_to_i64(
        &[
            checked_mul_i128(params.warp_params[2], src_x, "block-warp projected x")?,
            checked_mul_i128(params.warp_params[3], src_y, "block-warp projected x")?,
            i128::from(params.warp_params[0]),
        ],
        "block-warp projected x",
    )?;
    let dst_y = checked_sum_to_i64(
        &[
            checked_mul_i128(params.warp_params[4], src_x, "block-warp projected y")?,
            checked_mul_i128(params.warp_params[5], src_y, "block-warp projected y")?,
            i128::from(params.warp_params[1]),
        ],
        "block-warp projected y",
    )?;
    let x4 = dst_x >> params.subsampling_x;
    let y4 = dst_y >> params.subsampling_y;
    let mask = (1 << WARPEDMODEL_PREC_BITS) - 1;
    Ok(ProjectedCenter {
        x4_int: x4 >> WARPEDMODEL_PREC_BITS,
        y4_int: y4 >> WARPEDMODEL_PREC_BITS,
        sx4: x4 & mask,
        sy4: y4 & mask,
    })
}

fn checked_shift_left(value: i64, shift: u8, context: &'static str) -> Result<i64> {
    value
        .checked_mul(1i64 << shift)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn build_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &WarpPredictBlockParams,
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &mut [i32; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
) -> Result<()> {
    for i1 in -7..8 {
        for i2 in -4..4 {
            let sx = projected
                .sx4
                .checked_add(shear.alpha * i64::from(i2))
                .and_then(|value| value.checked_add(shear.beta * i64::from(i1)))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "block-warp horizontal phase",
                })?;
            let taps = warped_filter_row(sx)?;
            let ref_row = clip3(
                params.first_y,
                params.last_y,
                projected.y4_int + i64::from(i1),
            );
            let mut sum = 0i64;
            for (i3, &tap) in taps.iter().enumerate() {
                let ref_col = clip3(
                    params.first_x,
                    params.last_x,
                    projected.x4_int + i64::from(i2) - 3 + i3 as i64,
                );
                sum += i64::from(tap) * reference.sample(ref_row as usize, ref_col as usize);
            }
            let row = (i1 + 7) as usize;
            let col = (i2 + 4) as usize;
            intermediate[row * WARPED_BLOCK_SIZE + col] = round2(sum, INTER_ROUND0) as i32;
        }
    }
    Ok(())
}

fn build_output(
    shear: &Shear,
    projected: &ProjectedCenter,
    intermediate: &[i32; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE],
    round1: u32,
) -> Result<Vec<i32>> {
    let mut output = vec![0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
    for i1 in -4..4 {
        for i2 in -4..4 {
            let sy = projected
                .sy4
                .checked_add(shear.gamma * i64::from(i2))
                .and_then(|value| value.checked_add(shear.delta * i64::from(i1)))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "block-warp vertical phase",
                })?;
            let taps = warped_filter_row(sy)?;
            let mut sum = 0i64;
            for (i3, &tap) in taps.iter().enumerate() {
                let row = (i1 + i3 as i32 + 4) as usize;
                let col = (i2 + 4) as usize;
                sum += i64::from(tap) * i64::from(intermediate[row * WARPED_BLOCK_SIZE + col]);
            }
            let row = (i1 + 4) as usize;
            let col = (i2 + 4) as usize;
            output[row * WARPED_BLOCK_SIZE + col] = round2(sum, round1) as i32;
        }
    }
    Ok(output)
}

fn warped_filter_row(phase: i64) -> Result<&'static [i32; WARP_FILTER_TAPS]> {
    let offset = round2(phase, WARPEDDIFF_PREC_BITS)
        .checked_add(WARP_FILTER_CENTER)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "block-warp filter offset",
        })?;
    let offset_usize =
        usize::try_from(offset).map_err(|_| ReconError::WarpFilterOffsetOutOfRange { offset })?;
    WARPED_FILTERS
        .get(offset_usize)
        .ok_or(ReconError::WarpFilterOffsetOutOfRange { offset })
}

fn checked_mul_i128(left: i64, right: i64, context: &'static str) -> Result<i128> {
    i128::from(left)
        .checked_mul(i128::from(right))
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn checked_sum_to_i64(values: &[i128], context: &'static str) -> Result<i64> {
    let mut sum = 0i128;
    for &value in values {
        sum = sum
            .checked_add(value)
            .ok_or(ReconError::ArithmeticOverflow { context })?;
    }
    i64::try_from(sum).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn checked_product_to_i64(values: &[i128], context: &'static str) -> Result<i64> {
    let mut product = 1i128;
    for &value in values {
        product = product
            .checked_mul(value)
            .ok_or(ReconError::ArithmeticOverflow { context })?;
    }
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
        block_x: i64,
        block_y: i64,
        ref_w: i64,
        ref_h: i64,
    ) -> WarpPredictBlockParams {
        WarpPredictBlockParams {
            warp_params: IDENTITY_WARP_PARAMS,
            block_x,
            block_y,
            subsampling_x: 0,
            subsampling_y: 0,
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
        let clip = |lo: i64, hi: i64, value: i64| value.max(lo).min(hi);
        let round = |value: i64, shift: u32| {
            if shift == 0 {
                value
            } else {
                (value + (1 << (shift - 1))) >> shift
            }
        };
        let fetch = |row: i64, col: i64| {
            let row = clip(0, ref_h as i64 - 1, row) as usize;
            let col = clip(0, ref_w as i64 - 1, col) as usize;
            i64::from(samples[row * ref_w + col])
        };

        let shear = setup_shear(params.warp_params).unwrap();
        let projected = project_section_center(params).unwrap();
        let mut intermediate = [0i64; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
        for i1 in -7..8 {
            for i2 in -4..4 {
                let sx = projected.sx4 + shear.alpha * i64::from(i2) + shear.beta * i64::from(i1);
                let offset = (round(sx, WARPEDDIFF_PREC_BITS) + WARP_FILTER_CENTER) as usize;
                let taps = &WARPED_FILTERS[offset];
                let mut sum = 0i64;
                for (i3, &tap) in taps.iter().enumerate() {
                    let rr = clip(
                        params.first_y,
                        params.last_y,
                        projected.y4_int + i64::from(i1),
                    );
                    let cc = clip(
                        params.first_x,
                        params.last_x,
                        projected.x4_int + i64::from(i2) - 3 + i3 as i64,
                    );
                    sum += i64::from(tap) * fetch(rr, cc);
                }
                intermediate[(i1 + 7) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                    round(sum, INTER_ROUND0);
            }
        }

        let max_sample = i64::from(params.bit_depth.max_sample());
        let mut out = vec![0u16; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        for i1 in -4..4 {
            for i2 in -4..4 {
                let sy = projected.sy4 + shear.gamma * i64::from(i2) + shear.delta * i64::from(i1);
                let offset = (round(sy, WARPEDDIFF_PREC_BITS) + WARP_FILTER_CENTER) as usize;
                let taps = &WARPED_FILTERS[offset];
                let mut sum = 0i64;
                for (i3, &tap) in taps.iter().enumerate() {
                    let row = (i1 + i3 as i32 + 4) as usize;
                    let col = (i2 + 4) as usize;
                    sum += i64::from(tap) * intermediate[row * WARPED_BLOCK_SIZE + col];
                }
                let pred = round2(sum, INTER_ROUND1_NON_COMPOUND);
                out[(i1 + 4) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                    clip(0, max_sample, pred) as u16;
            }
        }
        out
    }

    #[test]
    fn warped_filters_table_shape_and_sums() {
        assert_eq!(WARPED_FILTERS.len(), 449);
        for (index, row) in WARPED_FILTERS.iter().enumerate() {
            assert_eq!(row.len(), WARP_FILTER_TAPS);
            assert_eq!(row.iter().sum::<i32>(), 128, "row {index}");
        }
    }

    #[test]
    fn identity_warp_flat_reference_returns_flat_block() {
        let ref_w = 16usize;
        let ref_h = 16usize;
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(4, 4, ref_w as i64, ref_h as i64);

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i64::from(params.bit_depth.max_sample()),
        );
        assert_eq!(out, vec![77u16; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
    }

    #[test]
    fn compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
        let (ref_w, ref_h) = (16usize, 16usize);
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(4, 4, ref_w as i64, ref_h as i64);

        let out = warp_predict_block(&view, &params, true).unwrap();
        assert_eq!(out, vec![16 * 77i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
    }

    #[test]
    fn compound_intermediate_affine_matches_round7_spec_trace() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(10, 9, ref_w as i64, ref_h as i64);
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
        let mut intermediate = [0i32; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
        build_intermediate(&view, &params, &shear, &projected, &mut intermediate).unwrap();
        let mut want = vec![0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        for i1 in -4..4 {
            for i2 in -4..4 {
                let sy = projected.sy4 + shear.gamma * i64::from(i2) + shear.delta * i64::from(i1);
                let taps = &WARPED_FILTERS
                    [(round2(sy, WARPEDDIFF_PREC_BITS) + WARP_FILTER_CENTER) as usize];
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
    fn ext_compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
        let (ref_w, ref_h) = (32usize, 32usize);
        let samples = vec![77u16; ref_w * ref_h];
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let params = default_params(8, 12, ref_w as i64, ref_h as i64);
        let out = ext_warp_predict_unit(&view, &params, 0, 0, true).unwrap();
        assert_eq!(out, vec![16 * 77i32; 16]);
    }

    #[test]
    fn integer_translation_warp_matches_spec_trace() {
        let ref_w = 24usize;
        let ref_h = 24usize;
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(6, 5, ref_w as i64, ref_h as i64);
        params.warp_params[0] = 2 << WARPEDMODEL_PREC_BITS;
        params.warp_params[1] = 3 << WARPEDMODEL_PREC_BITS;

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i64::from(params.bit_depth.max_sample()),
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
        let mut params = default_params(10, 9, ref_w as i64, ref_h as i64);
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
            i64::from(params.bit_depth.max_sample()),
        );
        let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want);
        assert_eq!(&out[..8], &[25, 33, 41, 49, 58, 115, 121, 129]);
    }

    #[test]
    fn border_extension_clips_reference_reads() {
        let ref_w = 12usize;
        let ref_h = 12usize;
        let samples = build_ref(ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        let mut params = default_params(0, 0, ref_w as i64, ref_h as i64);
        params.warp_params[0] = -(2 << WARPEDMODEL_PREC_BITS);
        params.warp_params[1] = -(1 << WARPEDMODEL_PREC_BITS);

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i64::from(params.bit_depth.max_sample()),
        );
        let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want);
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
        let params = default_params(8, 12, ref_w as i64, ref_h as i64);
        for (i4, j4) in [(0usize, 0usize), (1, 1)] {
            let out = crate::math::clip1_predicted_samples(
                ext_warp_predict_unit(&view, &params, i4, j4, false).unwrap(),
                i64::from(params.bit_depth.max_sample()),
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
        let mut params = default_params(8, 12, ref_w as i64, ref_h as i64);
        params.warp_params[0] = 3 << WARPEDMODEL_PREC_BITS;
        params.warp_params[1] = 2 << WARPEDMODEL_PREC_BITS;
        let out = crate::math::clip1_predicted_samples(
            ext_warp_predict_unit(&view, &params, 0, 1, false).unwrap(),
            i64::from(params.bit_depth.max_sample()),
        );
        for r in 0..4 {
            for c in 0..4 {
                let src = samples[(12 + 2 + r) * ref_w + 8 + 4 + 3 + c];
                assert_eq!(out[r * 4 + c], src, "r={r} c={c}");
            }
        }
    }
}
