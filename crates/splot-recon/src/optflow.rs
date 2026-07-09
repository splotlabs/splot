// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::math::{clip3, round2_signed};
use crate::{BitDepth, ReconError, Result, resolve_divisor};

const GRADIENT_UNIT: usize = 16;
const MAX_LS_BITS: u32 = 26;
const MV_REFINE_PREC_BITS: i32 = 4;
const MV_DELTA_LIMIT: i64 = 1 << MV_REFINE_PREC_BITS;

/// Derives the AV2 § 7.13.3.9 optical-flow motion-vector deltas for each
/// `unit_size` square in a pair of clipped, row-major luma predictors.
///
/// Each returned entry contains `[[row0, col0], [row1, col1]]` in 1/16-pel
/// units. Entries are ordered left-to-right, top-to-bottom. `distances` are the
/// unscaled relative order-hint distances; this function performs the § 7.13.3.9
/// ratio reduction before solving the least-squares system.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either predictor does not
/// contain exactly `width * height` values, [`ReconError::ZeroDimension`] for
/// an empty predictor, or [`ReconError::InvalidOptflowUnitSize`] unless the
/// unit size is 4 or 8 and divides both predictor dimensions.
pub fn derive_optflow_mv_deltas(
    pred0: &[u16],
    pred1: &[u16],
    width: usize,
    height: usize,
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<Vec<[[i32; 2]; 2]>> {
    if width == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor width",
        });
    }
    if height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor height",
        });
    }
    let expected = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "optical-flow predictor area",
        })?;
    for predictor in [pred0, pred1] {
        if predictor.len() != expected {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: predictor.len(),
            });
        }
    }
    if !matches!(unit_size, 4 | 8)
        || !width.is_multiple_of(unit_size)
        || !height.is_multiple_of(unit_size)
    {
        return Err(ReconError::InvalidOptflowUnitSize {
            unit_size,
            width,
            height,
        });
    }

    let unit_count = (width / unit_size) * (height / unit_size);
    if distances.contains(&0) {
        return Ok(vec![[[0; 2]; 2]; unit_count]);
    }
    let distances = reduce_distances(distances);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let mut weighted = vec![0i32; expected];
    let mut difference = vec![0i32; expected];
    for index in 0..expected {
        let left = i64::from(pred0[index]);
        let right = i64::from(pred1[index]);
        weighted[index] = round2_signed(
            i64::from(distances[0]) * left - i64::from(distances[1]) * right,
            downshift,
        ) as i32;
        difference[index] = round2_signed(left - right, downshift) as i32;
    }

    let (gradient_x, gradient_y) = gradients(&weighted, width, height);
    let mut deltas = Vec::with_capacity(unit_count);
    for unit_y in (0..height).step_by(unit_size) {
        for unit_x in (0..width).step_by(unit_size) {
            deltas.push(solve_unit(
                &gradient_x,
                &gradient_y,
                &difference,
                width,
                unit_x,
                unit_y,
                unit_size,
                distances,
            )?);
        }
    }
    Ok(deltas)
}

fn reduce_distances(distances: [i32; 2]) -> [i32; 2] {
    let magnitudes = match distances[0]
        .unsigned_abs()
        .cmp(&distances[1].unsigned_abs())
    {
        core::cmp::Ordering::Equal => [1, 1],
        core::cmp::Ordering::Greater => [2, 1],
        core::cmp::Ordering::Less => [1, 2],
    };
    [
        if distances[0] < 0 {
            -magnitudes[0]
        } else {
            magnitudes[0]
        },
        if distances[1] < 0 {
            -magnitudes[1]
        } else {
            magnitudes[1]
        },
    ]
}

fn gradients(values: &[i32], width: usize, height: usize) -> (Vec<i32>, Vec<i32>) {
    let mut horizontal = vec![0i32; values.len()];
    let mut vertical = vec![0i32; values.len()];
    for row in 0..height {
        for col in 0..width {
            let col_start = (col / GRADIENT_UNIT) * GRADIENT_UNIT;
            let col_end = (col_start + GRADIENT_UNIT).min(width) - 1;
            let row_start = (row / GRADIENT_UNIT) * GRADIENT_UNIT;
            let row_end = (row_start + GRADIENT_UNIT).min(height) - 1;
            let col_prev = col.saturating_sub(1).max(col_start);
            let col_prev2 = col.saturating_sub(2).max(col_start);
            let col_next = (col + 1).min(col_end);
            let col_next2 = (col + 2).min(col_end);
            let mut value = 42 * (values[row * width + col_next] - values[row * width + col_prev])
                - 5 * (values[row * width + col_next2] - values[row * width + col_prev2]);
            if col + 1 > col_end || col < col_start + 1 {
                value *= 2;
            }
            horizontal[row * width + col] = round2_signed(i64::from(value), 7) as i32;

            let row_prev = row.saturating_sub(1).max(row_start);
            let row_prev2 = row.saturating_sub(2).max(row_start);
            let row_next = (row + 1).min(row_end);
            let row_next2 = (row + 2).min(row_end);
            let mut value = 42 * (values[row_next * width + col] - values[row_prev * width + col])
                - 5 * (values[row_next2 * width + col] - values[row_prev2 * width + col]);
            if row + 1 > row_end || row < row_start + 1 {
                value *= 2;
            }
            vertical[row * width + col] = round2_signed(i64::from(value), 7) as i32;
        }
    }
    (horizontal, vertical)
}

#[allow(clippy::too_many_arguments)]
fn solve_unit(
    gradient_x: &[i32],
    gradient_y: &[i32],
    difference: &[i32],
    stride: usize,
    unit_x: usize,
    unit_y: usize,
    unit_size: usize,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    let mut su2 = (unit_size * unit_size) as i64;
    let mut sv2 = su2;
    let mut suv = 0i64;
    let mut suw = 0i64;
    let mut svw = 0i64;
    for row in unit_y..unit_y + unit_size {
        for col in unit_x..unit_x + unit_size {
            let index = row * stride + col;
            let u = i64::from(gradient_x[index]);
            let v = i64::from(gradient_y[index]);
            let w = i64::from(difference[index]);
            su2 += u * u;
            suv += u * v;
            sv2 += v * v;
            suw += u * w;
            svw += v * w;
        }
    }

    let max_product_bits = (1 + msb(su2)) + (1 + msb(sv2));
    let max_product_bits = max_product_bits
        .max((1 + msb(sv2)) + (1 + msb(suw.abs())))
        .max((1 + msb(suv.abs())) + (1 + msb(svw.abs())))
        .max((1 + msb(su2)) + (1 + msb(svw.abs())))
        .max((1 + msb(suv.abs())) + (1 + msb(suw.abs())));
    let reduction = max_product_bits.saturating_sub(MAX_LS_BITS - 3) >> 1;
    su2 = round2_signed(su2, reduction);
    sv2 = round2_signed(sv2, reduction);
    suv = round2_signed(suv, reduction);
    suw = round2_signed(suw, reduction);
    svw = round2_signed(svw, reduction);

    let determinant = su2 * sv2 - suv * suv;
    if determinant <= 0 {
        return Ok([[0; 2]; 2]);
    }
    let solution = divide_and_round(
        [sv2 * suw - suv * svw, su2 * svw - suv * suw],
        determinant,
        MV_REFINE_PREC_BITS - 1,
    )?;
    let col = -solution[0];
    let row = -solution[1];
    Ok([
        [
            clip3(
                -MV_DELTA_LIMIT,
                MV_DELTA_LIMIT,
                row * i64::from(distances[0]),
            ) as i32,
            clip3(
                -MV_DELTA_LIMIT,
                MV_DELTA_LIMIT,
                col * i64::from(distances[0]),
            ) as i32,
        ],
        [
            clip3(
                -MV_DELTA_LIMIT,
                MV_DELTA_LIMIT,
                row * i64::from(distances[1]),
            ) as i32,
            clip3(
                -MV_DELTA_LIMIT,
                MV_DELTA_LIMIT,
                col * i64::from(distances[1]),
            ) as i32,
        ],
    ])
}

fn divide_and_round(values: [i64; 2], denominator: i64, shift: i32) -> Result<[i64; 2]> {
    let (denominator_shift, inverse) = if denominator == 1 {
        (0i32, 1i64)
    } else {
        let (denominator_shift, inverse) = resolve_divisor(denominator as u64)?;
        (i32::from(denominator_shift), i64::from(inverse))
    };
    let inverse_msb = msb(inverse);
    let mut result = [0i64; 2];
    for (output, value) in result.iter_mut().zip(values) {
        if value == 0 {
            continue;
        }
        let sign = value.signum();
        let mut magnitude = value.abs();
        let reduction = (msb(magnitude) + inverse_msb + 4).saturating_sub(MAX_LS_BITS);
        magnitude = round2_signed(magnitude, reduction);
        let increase = shift + reduction as i32 - denominator_shift;
        magnitude = if increase <= -31 {
            let reduced = round2_signed(magnitude, (-increase - 30) as u32);
            round2_signed(reduced * inverse, 30)
        } else if increase >= 0 {
            magnitude * inverse * (1i64 << increase)
        } else {
            round2_signed(magnitude * inverse, (-increase) as u32)
        };
        *output = sign * magnitude;
    }
    Ok(result)
}

fn msb(value: i64) -> u32 {
    i64::BITS - 1 - value.max(1).leading_zeros()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn equal_predictors_produce_zero_deltas() {
        let predictor = vec![128; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&predictor, &predictor, 8, 8, 8, BitDepth::Eight, [3, -3],)
                .unwrap(),
            vec![[[0; 2]; 2]],
        );
    }

    #[test]
    fn four_by_four_units_cover_an_eight_by_eight_predictor() {
        let predictor = vec![80; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&predictor, &predictor, 8, 8, 4, BitDepth::Eight, [1, -2],)
                .unwrap()
                .len(),
            4,
        );
    }

    #[test]
    fn linear_horizontal_difference_solves_to_opposite_fractional_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1],).unwrap(),
            vec![[[0, -14], [0, 14]]],
        );
    }

    #[test]
    fn zero_distance_disables_refinement() {
        let left = vec![0; 8 * 8];
        let right = vec![255; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [0, -1],).unwrap(),
            vec![[[0; 2]; 2]],
        );
    }
}
