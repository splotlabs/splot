// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 §7.13.2.9 Intra Bilateral Prediction (IBP) angular blend.
//!
//! Feature tracking: `RECON-INTRA-IBP-ANGULAR-PREDICTION`.
//!
//! Blends a primary one-sided directional prediction with the opposite-zone
//! `secondPred` when the §7.13.2.7 `useIBP` gate and §7.13.2.9 mode gate fire.

use crate::intra::IntraRectBlockSize;
use crate::intra_dc_math::{DIV_LUT_BITS, resolve_divisor, round2};
use crate::intra_directional_angle::{
    DR_INTRA_DERIVATIVE, ZONE_1_MAX, ZONE_3_INDEX_BASE, ZONE_3_MIN,
};
use crate::{ReconError, ReconSample, Result};

const IBP_WEIGHT_SIZE_LOG2: u32 = 4;
const IBP_WEIGHT_SIZE: usize = 1 << IBP_WEIGHT_SIZE_LOG2;
const IBP_WEIGHT_SHIFT: u8 = DIV_LUT_BITS;
const IBP_WEIGHT_MAX: u16 = 1 << IBP_WEIGHT_SHIFT;
const IBP_WEIGHT_MIN_ANGLE: u16 = 39;

// AV2 §7.13.2.9 `angle_to_mode_index[90]`; slot 15 is disabled.
#[rustfmt::skip]
const ANGLE_TO_MODE_INDEX: [u8; 90] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 0,  0,  15, 0,  0,  14, 0,  0,  13, 0,  0,  12, 0,  0,  11, 0,  0,
    10, 0,  0,  0,  9,  0,  0,  8,  0,  0,  7,  0,  0,  6,  0,  0,  5,  0,
    0,  4,  0,  0,  3,  0,  0,  0,  0,  2,  0,  0,  1,  0,  0,  0,  0,  0,
];

// AV2 §7.13.2.9 `is_ibp_enabled[16]`.
const IS_IBP_ENABLED: [bool; 16] = [
    false, true, false, false, true, false, true, false, true, false, false, true, false, true,
    false, true,
];

/// Returns whether AV2 §7.13.2.9 applies the IBP blend for `p_angle`.
#[must_use]
pub fn ibp_blend_fires(p_angle: u16) -> bool {
    enabled_weight_angle(p_angle).is_some()
}

fn enabled_weight_angle(p_angle: u16) -> Option<u16> {
    let weight_angle = if p_angle > 0 && p_angle < ZONE_1_MAX {
        p_angle
    } else if p_angle > ZONE_3_MIN && p_angle < ZONE_3_INDEX_BASE {
        ZONE_3_INDEX_BASE - p_angle
    } else {
        return None;
    };
    let mode_index = *ANGLE_TO_MODE_INDEX.get(usize::from(weight_angle))?;
    IS_IBP_ENABLED
        .get(usize::from(mode_index))
        .copied()
        .unwrap_or(false)
        .then_some(weight_angle)
}

fn ibp_weights(p_angle: u16) -> Result<[[u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE]> {
    let p_angle = p_angle.max(IBP_WEIGHT_MIN_ANGLE);
    let index = usize::from(ZONE_1_MAX.checked_sub(p_angle).ok_or(
        ReconError::ArithmeticOverflow {
            context: "IBP angular weight angle index",
        },
    )?);
    let dy = i64::from(
        *DR_INTRA_DERIVATIVE
            .get(index)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "IBP angular weight derivative lookup",
            })?,
    );
    let mut weights = [[0u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE];
    for (r, row) in weights.iter_mut().enumerate() {
        let mut y = dy;
        for slot in row.iter_mut() {
            let dist = ((i64::try_from(r).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight row index",
            })? + 1)
                << 6)
                + y;
            let dist_u64 = u64::try_from(dist).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight distance range",
            })?;
            let (shift_raw, div) = resolve_divisor(dist_u64)?;
            let shift =
                shift_raw
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

/// Applies AV2 §7.13.2.9 IBP angular blending in place.
///
/// `primary` and `second` are tightly packed `width * height` row-major buffers.
/// Non-one-sided or disabled `p_angle`s are validated no-ops.
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
    let Some(weight_angle) = enabled_weight_angle(p_angle) else {
        return Ok(());
    };
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
    let zone1 = p_angle < ZONE_1_MAX;
    let shift = IBP_WEIGHT_SIZE_LOG2 + 1;
    let c_shift = (width as u32) >> shift;
    let r_shift = (height as u32) >> shift;
    for row in 0..height {
        let row_idx = (row as u32 >> r_shift) as usize;
        for column in 0..width {
            let col_idx = (column as u32 >> c_shift) as usize;
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
