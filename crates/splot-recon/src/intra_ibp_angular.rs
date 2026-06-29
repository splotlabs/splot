// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 §7.13.2.9 Intra Bilateral Prediction (IBP) angular blend.
//!
//! Feature tracking: `RECON-INTRA-IBP-ANGULAR-PREDICTION`.
//!
//! When `useIBP` fires for a one-sided directional luma leaf (§7.13.2.7:
//! `applyIbp && angleDelta even && plane 0 && (pAngle < 90 || pAngle > 180) &&
//! MrlIndex == 0`), the §7.13.2.8 single directional prediction is blended with a
//! SECOND directional prediction taken at `secondAngle` (the opposite zone), using
//! the per-position weights from the §7.13.2.9 IBP weights process. This module
//! computes those weights and applies the blend in place over a caller-provided
//! primary prediction and a caller-provided `secondPred` (the §7.13.2.8 prediction
//! at `secondAngle`).

use crate::intra::IntraRectBlockSize;
use crate::intra_dc_math::{DIV_LUT_BITS, resolve_divisor, round2};
use crate::{ReconError, ReconSample, Result};

/// AV2 §3 `IBP_WEIGHT_SIZE_LOG2`: the IBP weight grid is `IBP_WEIGHT_SIZE` square.
const IBP_WEIGHT_SIZE_LOG2: u32 = 4;
/// AV2 §3 `IBP_WEIGHT_SIZE = 1 << IBP_WEIGHT_SIZE_LOG2`.
const IBP_WEIGHT_SIZE: usize = 1 << IBP_WEIGHT_SIZE_LOG2;
/// AV2 §3 `IBP_WEIGHT_SHIFT = DIV_LUT_BITS`.
const IBP_WEIGHT_SHIFT: u8 = DIV_LUT_BITS;
/// AV2 §3 `IBP_WEIGHT_MAX = 1 << IBP_WEIGHT_SHIFT`.
const IBP_WEIGHT_MAX: u16 = 1 << IBP_WEIGHT_SHIFT;
/// The §7.13.2.9 lower clamp on `pAngle` before the weight derivative lookup
/// (`pAngle = Max(39, pAngle)`).
const IBP_WEIGHT_MIN_ANGLE: u16 = 39;

/// AV2 §7.13.2.9 `Dr_Intra_Derivative[90]`, the same projection-step table the
/// directional predictor uses (transcribed in `intra_directional_angle.rs`); the
/// IBP weights process indexes it at `90 - pAngle` for the (clamped) zone-1 angle
/// `secondAngle`'s mirror. Kept local so the IBP weight math is self-contained.
#[rustfmt::skip]
const DR_INTRA_DERIVATIVE: [u16; 90] = [
    0,    4096, 2048,
    1365, 1024, 819,
    682,  585,  512,
    455,  409,  409,  409, 372,
    341,  292,  273,
    256,  227,  215,
    204,  186,  178,
    170,  157,  151,
    146,  136,  132,
    128,  117,  110,
    107,  99,   97,   97,
    93,   87,   83,
    81,   77,   74,
    73,   69,   66,
    64,   62,   59,
    56,   55,   53,
    50,   49,   47,
    44,   42,   42,   41,
    38,   37,   35,
    32,   31,   30,
    28,   27,   26,
    24,   23,   22,
    20,   19,   18,
    16,   15,   14,
    12,   11,   10,   10,  10,
    9,    8,    7,
    6,    5,    4,
    3,    2,    1,
];

/// AV2 §7.13.2.9 `angle_to_mode_index[90]`: maps a (clamped) zone-1 weight angle to
/// the `is_ibp_enabled` directional-mode slot. Index `15` is the "no IBP" slot.
/// Transcribed verbatim from AVM `av2_common_int.h`.
#[rustfmt::skip]
const ANGLE_TO_MODE_INDEX: [u8; 90] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 0,  0,  15, 0,  0,  14, 0,  0,  13, 0,  0,  12, 0,  0,  11, 0,  0,
    10, 0,  0,  0,  9,  0,  0,  8,  0,  0,  7,  0,  0,  6,  0,  0,  5,  0,
    0,  4,  0,  0,  3,  0,  0,  0,  0,  2,  0,  0,  1,  0,  0,  0,  0,  0,
];

/// AV2 §7.13.2.9 `is_ibp_enabled[16]`: whether the directional mode in each
/// `angle_to_mode_index` slot participates in the IBP blend. When the leaf's
/// `mode_index` is disabled, AVM skips the blend (`pred` is the primary only).
/// Transcribed verbatim from AVM `av2_common_int.h`.
const IS_IBP_ENABLED: [bool; 16] = [
    false, true, false, false, true, false, true, false, true, false, false, true, false, true,
    false, true,
];

/// Whether the §7.13.2.9 IBP blend actually fires for a one-sided `p_angle`, i.e.
/// whether the leaf's `mode_index` (from `angle_to_mode_index`) is in the
/// `is_ibp_enabled` set. AVM gates the blend on this: a `useIBP` leaf whose mode
/// is disabled keeps the bare primary prediction. Returns `false` for any
/// `p_angle` outside the one-sided ranges (the caller already excludes those).
#[must_use]
pub fn ibp_blend_fires(p_angle: u16) -> bool {
    weight_angle_and_mode_index(p_angle).is_some_and(|(_, mode_index)| {
        IS_IBP_ENABLED
            .get(usize::from(mode_index))
            .copied()
            .unwrap_or(false)
    })
}

/// AV2 §7.13.2.9: the (zone-1) weight angle and `mode_index` for a one-sided
/// `p_angle`. Zone-1 (`0 < p < 90`) uses `p` directly; zone-3 (`180 < p < 270`)
/// uses `270 - p` (mirroring it into zone-1). Returns `None` for a non-one-sided
/// `p_angle`.
fn weight_angle_and_mode_index(p_angle: u16) -> Option<(u16, u8)> {
    let weight_angle = if p_angle > 0 && p_angle < 90 {
        p_angle
    } else if p_angle > 180 && p_angle < 270 {
        270 - p_angle
    } else {
        return None;
    };
    let mode_index = *ANGLE_TO_MODE_INDEX.get(usize::from(weight_angle))?;
    Some((weight_angle, mode_index))
}

/// AV2 §7.13.2.9 IBP weights process. Input `p_angle` is the §7.13.2.9 weight
/// angle (zone-1 `pAngle`, or `270 - pAngle` for zone-3). The process clamps it to
/// `Max(39, pAngle)`, looks up `dy = Dr_Intra_Derivative[90 - pAngle]`, and fills
/// an `IBP_WEIGHT_SIZE` x `IBP_WEIGHT_SIZE` grid:
///
/// ```text
/// dy = Dr_Intra_Derivative[90 - pAngle]
/// for (r = 0; r < IBP_WEIGHT_SIZE; r++) {
///     y = dy
///     for (c = 0; c < IBP_WEIGHT_SIZE; c++) {
///         dist = ((r + 1) << 6) + y
///         (shift, div) = resolve_divisor(dist)
///         shift -= DIV_LUT_BITS
///         weights[r][c] = Round2(y * div, shift)
///         y += dy
///     }
/// }
/// ```
///
/// Transcribed verbatim from the committed spec mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-9` (cross-checked against
/// AVM `av2_dr_prediction_z1_info`).
fn ibp_weights(p_angle: u16) -> Result<[[u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE]> {
    let p_angle = p_angle.max(IBP_WEIGHT_MIN_ANGLE);
    let index = usize::from(90u16.checked_sub(p_angle).ok_or(ReconError::ArithmeticOverflow {
        context: "IBP angular weight angle index",
    })?);
    let dy = i64::from(*DR_INTRA_DERIVATIVE.get(index).ok_or(ReconError::ArithmeticOverflow {
        context: "IBP angular weight derivative lookup",
    })?);
    let mut weights = [[0u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE];
    for (r, row) in weights.iter_mut().enumerate() {
        let mut y = dy;
        for slot in row.iter_mut() {
            // dist = ((r + 1) << 6) + y; both terms are non-negative.
            let dist = ((i64::try_from(r).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight row index",
            })? + 1)
                << 6)
                + y;
            let dist_u64 = u64::try_from(dist).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight distance range",
            })?;
            let (shift_raw, div) = resolve_divisor(dist_u64)?;
            // §7.13.2.9: shift -= DIV_LUT_BITS, then weight0 = Round2(y * div, shift).
            let shift = shift_raw
                .checked_sub(DIV_LUT_BITS)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "IBP angular weight shift",
                })?;
            let product = y
                .checked_mul(i64::from(div))
                .and_then(|p| u64::try_from(p).ok())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "IBP angular weight product",
                })?;
            *slot = round2(product, shift);
            y += dy;
        }
    }
    Ok(weights)
}

/// AV2 §7.13.2.9 IBP blend, applied in place over `primary` (the §7.13.2.8
/// prediction at `pAngle`) using `second` (the §7.13.2.8 prediction at
/// `secondAngle`). Both buffers are tightly packed `width * height` row-major
/// arrays (stride == width). The per-sample weight `s` is indexed by the
/// down-shifted row/column:
///
/// ```text
/// cShift = w >> (IBP_WEIGHT_SIZE_LOG2 + 1)
/// rShift = h >> (IBP_WEIGHT_SIZE_LOG2 + 1)
/// for (r = 0; r < h; r++) for (c = 0; c < w; c++) {
///     s = pAngle < 90 ? weights[r >> rShift][c >> cShift]
///                     : weights[c >> cShift][r >> rShift]
///     primary[r][c] = Round2(primary[r][c] * s +
///                            second[r][c] * (IBP_WEIGHT_MAX - s), IBP_WEIGHT_SHIFT)
/// }
/// ```
///
/// Transcribed verbatim from the committed spec mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8` (cross-checked against
/// AVM `av2_highbd_ibp_dr_prediction_z1_c` / `_z3_c`). The blend only fires when
/// [`ibp_blend_fires`] holds for `p_angle`; otherwise it is a validated no-op.
///
/// # Errors
/// Returns [`ReconError::ArithmeticOverflow`] on index/arithmetic overflow or
/// when `primary`/`second` are too small for `size`, and any error surfaced by
/// the §7.13.2.9 weight derivation.
pub fn apply_ibp_dr_blend_rect<T: ReconSample>(
    size: IntraRectBlockSize,
    p_angle: u16,
    primary: &mut [T],
    second: &[T],
) -> Result<()> {
    let Some((weight_angle, mode_index)) = weight_angle_and_mode_index(p_angle) else {
        return Ok(());
    };
    if !IS_IBP_ENABLED
        .get(usize::from(mode_index))
        .copied()
        .unwrap_or(false)
    {
        // §7.13.2.9: the leaf's mode is not in the enabled set; AVM keeps the bare
        // primary prediction. No blend, no mutation.
        return Ok(());
    }
    let width = size.width();
    let height = size.height();
    let needed = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP angular blend buffer size",
        })?;
    if primary.len() < needed || second.len() < needed {
        return Err(ReconError::ArithmeticOverflow {
            context: "IBP angular blend buffer too small",
        });
    }
    let weights = ibp_weights(weight_angle)?;
    let zone1 = p_angle < 90;
    // cShift = w >> (IBP_WEIGHT_SIZE_LOG2 + 1); rShift = h >> (…+1).
    let shift = IBP_WEIGHT_SIZE_LOG2 + 1;
    let c_shift = (width as u32) >> shift;
    let r_shift = (height as u32) >> shift;
    for row in 0..height {
        let row_idx = (row as u32 >> r_shift) as usize;
        for column in 0..width {
            let col_idx = (column as u32 >> c_shift) as usize;
            // Zone-1 indexes [row][col]; zone-3 transposes to [col][row].
            let s = if zone1 {
                weight_at(&weights, row_idx, col_idx)?
            } else {
                weight_at(&weights, col_idx, row_idx)?
            };
            let index = row * width + column;
            let primary_value = u64::from(primary[index].to_u16());
            let second_value = u64::from(second[index].to_u16());
            let inverse = u64::from(IBP_WEIGHT_MAX - s);
            let blended = round2(
                primary_value * u64::from(s) + second_value * inverse,
                IBP_WEIGHT_SHIFT,
            );
            primary[index] = T::try_from_u16(blended)?;
        }
    }
    Ok(())
}

fn weight_at(
    weights: &[[u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE],
    outer: usize,
    inner: usize,
) -> Result<u16> {
    weights
        .get(outer)
        .and_then(|row| row.get(inner))
        .copied()
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP angular weight index",
        })
}

#[cfg(test)]
#[path = "intra_ibp_angular/tests.rs"]
mod tests;
