// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 §7.13.2.9 Intra Bilateral Prediction (IBP) angular blend.
//!
//! Feature tracking: `RECON-INTRA-IBP-ANGULAR-PREDICTION`.
//!
//! Blends a primary one-sided directional prediction with the opposite-zone
//! `secondPred` when the §7.13.2.7 `useIBP` gate and §7.13.2.9 mode gate fire.

use crate::intra::IntraRectBlockSize;
use crate::intra_dc_math::{DIV_LUT_BITS, IntraLaneSample, resolve_divisor_32, round2_u32};
use crate::intra_directional_angle::{
    DR_INTRA_DERIVATIVE, ZONE_1_MAX, ZONE_3_INDEX_BASE, ZONE_3_MIN,
};
use crate::intra_ibp_dc::blend_ibp_lanes;
use crate::{ReconError, ReconSample, Result};
use fearless_simd::{Simd, SimdBase, u16x16};
use std::sync::OnceLock;

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

/// AV2 § 7.13.2.9 blend weights for one angle.
type IbpWeights = [[u16; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE];

/// Derived § 7.13.2.9 weight tables, one slot per clamped weight angle: the
/// zone 1 `[row][column]` table and its zone 3 transpose.
static IBP_WEIGHT_TABLES: [OnceLock<[IbpWeights; 2]>; ZONE_1_MAX as usize + 1] =
    [const { OnceLock::new() }; ZONE_1_MAX as usize + 1];

/// Returns the § 7.13.2.9 weight table for `p_angle` and its transpose,
/// deriving them once.
///
/// The table depends only on the clamped weight angle, so the per-block
/// derivation is memoized rather than repeated for every predicted block. The
/// slot lookup rejects exactly the angles [`ibp_weights`] rejects.
fn ibp_weight_table(p_angle: u16) -> Result<&'static [IbpWeights; 2]> {
    let slot = IBP_WEIGHT_TABLES
        .get(usize::from(p_angle.max(IBP_WEIGHT_MIN_ANGLE)))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP angular weight angle index",
        })?;
    if let Some(weights) = slot.get() {
        return Ok(weights);
    }
    let weights = ibp_weights(p_angle)?;
    let transposed = core::array::from_fn(|column| weights.map(|row| row[column]));
    Ok(slot.get_or_init(|| [weights, transposed]))
}

fn ibp_weights(p_angle: u16) -> Result<IbpWeights> {
    let p_angle = p_angle.max(IBP_WEIGHT_MIN_ANGLE);
    let index = usize::from(ZONE_1_MAX.checked_sub(p_angle).ok_or(
        ReconError::ArithmeticOverflow {
            context: "IBP angular weight angle index",
        },
    )?);
    let dy = i32::from(
        *DR_INTRA_DERIVATIVE
            .get(index)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "IBP angular weight derivative lookup",
            })?,
    );
    let mut weights: IbpWeights = [[0; IBP_WEIGHT_SIZE]; IBP_WEIGHT_SIZE];
    for (r, row) in weights.iter_mut().enumerate() {
        let mut y = dy;
        for slot in row.iter_mut() {
            let dist = ((i32::try_from(r).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight row index",
            })? + 1)
                << 6)
                + y;
            let dist_u32 = u32::try_from(dist).map_err(|_| ReconError::ArithmeticOverflow {
                context: "IBP angular weight distance range",
            })?;
            let (shift_raw, div) = resolve_divisor_32(dist_u32)?;
            let shift =
                shift_raw
                    .checked_sub(DIV_LUT_BITS)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "IBP angular weight shift",
                    })?;
            let product = y
                .checked_mul(i32::from(div))
                .and_then(|p| u32::try_from(p).ok())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "IBP angular weight product",
                })?;
            *slot = round2_u32(product, shift) as u16;
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
    let zone1 = p_angle < ZONE_1_MAX;
    let weights = &ibp_weight_table(weight_angle)?[usize::from(!zone1)];
    let shift = IBP_WEIGHT_SIZE_LOG2 + 1;
    let c_shift = (width as u32) >> shift;
    let r_shift = (height as u32) >> shift;
    if width <= 64 && height <= 64 && width.is_multiple_of(4) {
        let (primary, second) = (&mut primary[..needed], &second[..needed]);
        if let (Some(primary), Some(second)) = (T::u16_slice_mut(primary), T::u16_slice(second)) {
            blend_rows_lanes(weights, (c_shift, r_shift), width, true, primary, second);
            return Ok(());
        }
        if let (Some(primary), Some(second)) = (T::u8_slice_mut(primary), T::u8_slice(second)) {
            blend_rows_lanes(weights, (c_shift, r_shift), width, false, primary, second);
            return Ok(());
        }
    }
    for (row, (primary_row, second_row)) in primary
        .chunks_exact_mut(width)
        .zip(second.chunks_exact(width))
        .take(height)
        .enumerate()
    {
        let row_weights = weight_row(weights, (row as u32 >> r_shift) as usize)?;
        for (column, (target, &source)) in primary_row.iter_mut().zip(second_row).enumerate() {
            let s = weight_lane(row_weights, (column as u32 >> c_shift) as usize)?;
            let primary_value = u32::from(target.to_u16());
            let second_value = u32::from(source.to_u16());
            let inverse = u32::from(IBP_WEIGHT_MAX - s);
            let blended = round2_u32(
                primary_value * u32::from(s) + second_value * inverse,
                IBP_WEIGHT_SHIFT,
            ) as u16;
            *target = T::try_from_u16(blended)?;
        }
    }
    Ok(())
}

/// The lane form of the blend over `width`-sample rows, 16 lanes at a time:
/// 16 columns of one row, or `16 / w` rows of a narrower block. The weights of
/// each distinct weight row expand once. Every `(row, column) >> shift` index
/// is below `IBP_WEIGHT_SIZE` because the shifts scale a side of at most 64.
#[inline(never)]
fn blend_rows_lanes<O: IntraLaneSample>(
    weights: &IbpWeights,
    shifts: (u32, u32),
    width: usize,
    wide: bool,
    primary: &mut [O],
    second: &[O],
) {
    if width <= 16 {
        baseline_simd!(simd => blend_rows::<_, _, 16>(simd, weights, shifts, width, wide, primary, second));
    } else {
        baseline_simd!(simd => blend_rows::<_, _, 64>(simd, weights, shifts, width, wide, primary, second));
    }
}

/// [`blend_rows_lanes`] for rows of at most `W` samples. Rows of at most 16
/// samples read their weights straight from the table (no column shift); wider
/// rows expand each distinct weight row into `W`-wide scratch once.
#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_rows<S: Simd, O: IntraLaneSample, const W: usize>(
    simd: S,
    weights: &IbpWeights,
    (c_shift, r_shift): (u32, u32),
    width: usize,
    wide: bool,
    primary: &mut [O],
    second: &[O],
) {
    let width_log2 = width.trailing_zeros();
    let row_weights = |row: usize| &weights[(row >> r_shift) & (IBP_WEIGHT_SIZE - 1)];
    let mut expanded = [0u16; W];
    let mut expanded_row = usize::MAX;
    let mut group = [0u16; 16];
    for (index, (target, source)) in primary
        .as_chunks_mut::<16>()
        .0
        .iter_mut()
        .zip(second.as_chunks::<16>().0)
        .enumerate()
    {
        let first = index * 16;
        let row = first >> width_log2;
        let weight = match width {
            4 | 8 => {
                fn expand<const N: usize>(group: &mut [u16; 16], row: impl Fn(usize) -> [u16; N]) {
                    for (r, lanes) in group.as_chunks_mut::<N>().0.iter_mut().enumerate() {
                        *lanes = row(r);
                    }
                }
                if width == 4 {
                    expand::<4>(&mut group, |r| {
                        row_weights(row + r)
                            .first_chunk()
                            .copied()
                            .unwrap_or_default()
                    });
                } else {
                    expand::<8>(&mut group, |r| {
                        row_weights(row + r)
                            .first_chunk()
                            .copied()
                            .unwrap_or_default()
                    });
                }
                u16x16::from_slice(simd, &group)
            }
            16 => u16x16::from_slice(simd, row_weights(row)),
            _ => {
                let weight_row = row >> r_shift;
                if weight_row != expanded_row {
                    let weights = row_weights(row);
                    for (column, slot) in expanded.iter_mut().take(width).enumerate() {
                        *slot = weights[(column >> c_shift) & (IBP_WEIGHT_SIZE - 1)];
                    }
                    expanded_row = weight_row;
                }
                let column = first & (width - 1);
                u16x16::from_slice(simd, &expanded[column..column + 16])
            }
        };
        let blended = blend_ibp_lanes(
            simd,
            wide,
            O::load16(simd, target),
            O::load16(simd, source),
            weight,
        );
        O::store16(simd, blended, target);
    }
}

fn weight_row(weights: &IbpWeights, outer: usize) -> Result<&[u16]> {
    weights
        .get(outer)
        .map(AsRef::as_ref)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP angular weight index",
        })
}

fn weight_lane(row: &[u16], inner: usize) -> Result<u16> {
    row.get(inner)
        .copied()
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP angular weight index",
        })
}

#[cfg(test)]
#[path = "intra_ibp_angular/tests.rs"]
mod tests;
