// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Select, Simd,
    cmp::SimdOrd,
    num::{SimdInt, SimdUint},
};

use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2_signed_i32;
use crate::{BitDepth, ReconError, Result};

const GRADIENT_UNIT: usize = 16;
const MAX_LS_BITS: u32 = 26;
const MV_REFINE_PREC_BITS: i32 = 4;
const MV_DELTA_LIMIT: i32 = 1 << MV_REFINE_PREC_BITS;

/// Reusable working storage for optical-flow motion-vector derivation.
#[derive(Debug, Default)]
pub struct OptflowScratch {
    samples: Vec<i16>,
    deltas: Vec<[[i32; 2]; 2]>,
}

impl OptflowScratch {
    /// Creates reusable optical-flow storage with the requested capacities.
    #[must_use]
    pub fn with_capacity(sample_capacity: usize, delta_capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(sample_capacity),
            deltas: Vec::with_capacity(delta_capacity),
        }
    }
}

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
    let mut scratch = OptflowScratch::default();
    derive_optflow_mv_deltas_into(
        pred0,
        pred1,
        width,
        height,
        unit_size,
        bit_depth,
        distances,
        &mut scratch,
    )?;
    Ok(core::mem::take(&mut scratch.deltas))
}

/// Derives AV2 § 7.13.3.9 optical-flow deltas into reusable buffers.
///
/// `scratch` retains its allocations between calls. The returned slice contains
/// exactly one entry per `unit_size` square. Its internal output is cleared
/// before validation and remains empty on error.
///
/// # Errors
///
/// Returns the same errors as [`derive_optflow_mv_deltas`].
#[allow(clippy::too_many_arguments)]
pub fn derive_optflow_mv_deltas_into<'a>(
    pred0: &[u16],
    pred1: &[u16],
    width: usize,
    height: usize,
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &'a mut OptflowScratch,
) -> Result<&'a [[[i32; 2]; 2]]> {
    scratch.deltas.clear();
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
        scratch.deltas.resize(unit_count, [[0; 2]; 2]);
        return Ok(&scratch.deltas);
    }
    let distances = reduce_distances(distances);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let scratch_len = expected
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "optical-flow scratch sample count",
        })?;
    scratch.samples.resize(scratch_len, 0);
    let (weighted, scratch_samples) = scratch.samples.split_at_mut(expected);
    let (difference, scratch_samples) = scratch_samples.split_at_mut(expected);
    let (gradient_x, gradient_y) = scratch_samples.split_at_mut(expected);
    let max_sample = bit_depth.max_sample();
    let (pred0_chunks, pred0_remainder) = pred0.as_chunks::<8>();
    let (pred1_chunks, pred1_remainder) = pred1.as_chunks::<8>();
    let (weighted_chunks, weighted_remainder) = weighted.as_chunks_mut::<8>();
    let (difference_chunks, difference_remainder) = difference.as_chunks_mut::<8>();
    debug_assert!(pred0_remainder.is_empty());
    debug_assert!(pred1_remainder.is_empty());
    debug_assert!(weighted_remainder.is_empty());
    debug_assert!(difference_remainder.is_empty());
    let mut peak = Simd::<u16, 8>::splat(0);
    for (((left_samples, right_samples), weighted), difference) in pred0_chunks
        .iter()
        .zip(pred1_chunks)
        .zip(weighted_chunks)
        .zip(difference_chunks)
    {
        let left_samples = Simd::from_array(*left_samples);
        let right_samples = Simd::from_array(*right_samples);
        peak = peak.simd_max(left_samples).simd_max(right_samples);
        let left = left_samples.cast::<i32>();
        let right = right_samples.cast::<i32>();
        *weighted = round2_signed_simd(
            Simd::splat(distances[0]) * left - Simd::splat(distances[1]) * right,
            downshift,
        )
        .cast::<i16>()
        .to_array();
        *difference = round2_signed_simd(left - right, downshift)
            .cast::<i16>()
            .to_array();
    }
    if peak.reduce_max() > max_sample {
        for (sample_index, (&left, &right)) in pred0.iter().zip(pred1).enumerate() {
            for (predictor, value) in [(0, left), (1, right)] {
                if value > max_sample {
                    return Err(ReconError::OptflowPredictorSampleOutOfRange {
                        predictor,
                        sample_index,
                        value,
                        max: max_sample,
                    });
                }
            }
        }
    }

    gradients(weighted, width, height, gradient_x, gradient_y);
    scratch.deltas.reserve(unit_count);
    for unit_y in (0..height).step_by(unit_size) {
        for unit_x in (0..width).step_by(unit_size) {
            let delta = match solve_unit(
                gradient_x, gradient_y, difference, width, unit_x, unit_y, unit_size, distances,
            ) {
                Ok(delta) => delta,
                Err(error) => {
                    scratch.deltas.clear();
                    return Err(error);
                }
            };
            scratch.deltas.push(delta);
        }
    }
    Ok(&scratch.deltas)
}

/// Derives one 8x8 optical-flow delta from strided predictors.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either predictor does not
/// contain the requested 8x8 region, or the same arithmetic errors as
/// [`derive_optflow_mv_deltas`].
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn derive_optflow_mv_delta_8x8_strided_into(
    pred0: &[u16],
    start0: usize,
    pred1: &[u16],
    start1: usize,
    stride: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &mut OptflowScratch,
) -> Result<[[i32; 2]; 2]> {
    scratch.deltas.clear();
    if stride < 8 {
        return Err(ReconError::BufferLengthMismatch {
            expected: 8,
            actual: stride,
        });
    }
    let span_len = stride
        .checked_mul(7)
        .and_then(|offset| offset.checked_add(8))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided optical-flow predictor span",
        })?;
    let pred0 = pred0.get(start0..start0.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start0.saturating_add(span_len),
            actual: pred0.len(),
        },
    )?;
    let pred1 = pred1.get(start1..start1.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start1.saturating_add(span_len),
            actual: pred1.len(),
        },
    )?;
    if distances.contains(&0) {
        return Ok([[0; 2]; 2]);
    }

    let distances = reduce_distances(distances);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    scratch.samples.resize(8 * 8 * 4, 0);
    let (weighted, scratch_samples) = scratch.samples.split_at_mut(8 * 8);
    let (difference, scratch_samples) = scratch_samples.split_at_mut(8 * 8);
    let (gradient_x, gradient_y) = scratch_samples.split_at_mut(8 * 8);
    prepare_optflow_strided_rows(
        pred0,
        pred1,
        stride,
        bit_depth.max_sample(),
        distances,
        downshift,
        weighted,
        difference,
    )?;

    gradients(weighted, 8, 8, gradient_x, gradient_y);
    solve_unit(gradient_x, gradient_y, difference, 8, 0, 0, 8, distances)
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn prepare_optflow_strided_rows(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    max_sample: u16,
    distances: [i32; 2],
    downshift: u32,
    weighted: &mut [i16],
    difference: &mut [i16],
) -> Result<()> {
    let (weighted_rows, weighted_remainder) = weighted.as_chunks_mut::<8>();
    let (difference_rows, difference_remainder) = difference.as_chunks_mut::<8>();
    debug_assert!(weighted_remainder.is_empty());
    debug_assert!(difference_remainder.is_empty());
    let mut peak = Simd::<u16, 8>::splat(0);
    for (row, (weighted, difference)) in weighted_rows
        .iter_mut()
        .zip(difference_rows.iter_mut())
        .enumerate()
    {
        let source = row * stride;
        let left_samples = Simd::<u16, 8>::from_slice(&pred0[source..]);
        let right_samples = Simd::<u16, 8>::from_slice(&pred1[source..]);
        peak = peak.simd_max(left_samples).simd_max(right_samples);
        let left = left_samples.cast::<i32>();
        let right = right_samples.cast::<i32>();
        *weighted = round2_signed_simd(
            Simd::splat(distances[0]) * left - Simd::splat(distances[1]) * right,
            downshift,
        )
        .cast::<i16>()
        .to_array();
        *difference = round2_signed_simd(left - right, downshift)
            .cast::<i16>()
            .to_array();
    }
    if peak.reduce_max() > max_sample {
        for row in 0..weighted_rows.len() {
            let source = row * stride;
            let sample_index = row * 8;
            for lane in 0..8 {
                for (predictor, value) in [(0, pred0[source + lane]), (1, pred1[source + lane])] {
                    if value > max_sample {
                        return Err(ReconError::OptflowPredictorSampleOutOfRange {
                            predictor,
                            sample_index: sample_index + lane,
                            value,
                            max: max_sample,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn round2_signed_simd<const LANES: usize>(value: Simd<i32, LANES>, shift: u32) -> Simd<i32, LANES> {
    if shift == 0 {
        return value;
    }
    let rounded = (value.abs() + Simd::splat(1 << (shift - 1))) >> shift as i32;
    value.is_negative().select(-rounded, rounded)
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

fn gradients(
    values: &[i16],
    width: usize,
    height: usize,
    horizontal: &mut [i16],
    vertical: &mut [i16],
) {
    for (source, output) in values
        .chunks_exact(width)
        .zip(horizontal.chunks_exact_mut(width))
    {
        let (source_units, source_remainder) = source.as_chunks::<GRADIENT_UNIT>();
        let (output_units, output_remainder) = output.as_chunks_mut::<GRADIENT_UNIT>();
        for (source, output) in source_units.iter().zip(output_units) {
            horizontal_gradient_unit(source, output);
        }
        horizontal_gradient_partial(source_remainder, output_remainder);
    }
    for row in 0..height {
        let row_start = (row / GRADIENT_UNIT) * GRADIENT_UNIT;
        let row_end = (row_start + GRADIENT_UNIT).min(height) - 1;
        let row_prev = row.saturating_sub(1).max(row_start);
        let row_prev2 = row.saturating_sub(2).max(row_start);
        let row_next = (row + 1).min(row_end);
        let row_next2 = (row + 2).min(row_end);
        let double = row + 1 > row_end || row < row_start + 1;
        let row_slice = |row| &values[row * width..][..width];
        let output = &mut vertical[row * width..][..width];
        vertical_gradient_row(
            row_slice(row_prev),
            row_slice(row_prev2),
            row_slice(row_next),
            row_slice(row_next2),
            double,
            output,
        );
    }
}

fn horizontal_gradient_unit(source: &[i16; GRADIENT_UNIT], output: &mut [i16; GRADIENT_UNIT]) {
    for col in [0, 1, GRADIENT_UNIT - 2, GRADIENT_UNIT - 1] {
        output[col] = horizontal_gradient_scalar(source, col);
    }
    let next = Simd::<i16, 8>::from_slice(&source[3..11]).cast::<i32>();
    let prev = Simd::<i16, 8>::from_slice(&source[1..9]).cast::<i32>();
    let next2 = Simd::<i16, 8>::from_slice(&source[4..12]).cast::<i32>();
    let prev2 = Simd::<i16, 8>::from_slice(&source[..8]).cast::<i32>();
    let value = Simd::splat(42) * (next - prev) - Simd::splat(5) * (next2 - prev2);
    output[2..10].copy_from_slice(&round2_signed_simd(value, 7).cast::<i16>().to_array()); // splot-copy-ok: publish SIMD horizontal gradients into caller scratch

    let next = Simd::<i16, 4>::from_slice(&source[11..15]).cast::<i32>();
    let prev = Simd::<i16, 4>::from_slice(&source[9..13]).cast::<i32>();
    let next2 = Simd::<i16, 4>::from_slice(&source[12..]).cast::<i32>();
    let prev2 = Simd::<i16, 4>::from_slice(&source[8..12]).cast::<i32>();
    let value = Simd::splat(42) * (next - prev) - Simd::splat(5) * (next2 - prev2);
    output[10..14].copy_from_slice(&round2_signed_simd(value, 7).cast::<i16>().to_array()); // splot-copy-ok: publish SIMD horizontal gradients into caller scratch
}

fn horizontal_gradient_partial(source: &[i16], output: &mut [i16]) {
    if source.is_empty() {
        return;
    }
    let col_end = source.len() - 1;
    let mut col = 0;
    while col <= col_end {
        if col >= 2 && col + 5 <= col_end {
            let next = Simd::<i16, 4>::from_slice(&source[col + 1..]).cast::<i32>();
            let prev = Simd::<i16, 4>::from_slice(&source[col - 1..]).cast::<i32>();
            let next2 = Simd::<i16, 4>::from_slice(&source[col + 2..]).cast::<i32>();
            let prev2 = Simd::<i16, 4>::from_slice(&source[col - 2..]).cast::<i32>();
            let value = Simd::splat(42) * (next - prev) - Simd::splat(5) * (next2 - prev2);
            output[col..col + 4]
                .copy_from_slice(&round2_signed_simd(value, 7).cast::<i16>().to_array()); // splot-copy-ok: publish SIMD horizontal gradients into caller scratch
            col += 4;
            continue;
        }
        output[col] = horizontal_gradient_scalar(source, col);
        col += 1;
    }
}

fn horizontal_gradient_scalar(source: &[i16], col: usize) -> i16 {
    let col_end = source.len() - 1;
    let col_prev = col.saturating_sub(1);
    let col_prev2 = col.saturating_sub(2);
    let col_next = (col + 1).min(col_end);
    let col_next2 = (col + 2).min(col_end);
    let mut value = 42 * (i32::from(source[col_next]) - i32::from(source[col_prev]))
        - 5 * (i32::from(source[col_next2]) - i32::from(source[col_prev2]));
    if col + 1 > col_end || col < 1 {
        value *= 2;
    }
    round2_signed_i32(value, 7) as i16
}

fn vertical_gradient_row(
    prev: &[i16],
    prev2: &[i16],
    next: &[i16],
    next2: &[i16],
    double: bool,
    output: &mut [i16],
) {
    let (prev_chunks, prev_remainder) = prev.as_chunks::<8>();
    let (prev2_chunks, prev2_remainder) = prev2.as_chunks::<8>();
    let (next_chunks, next_remainder) = next.as_chunks::<8>();
    let (next2_chunks, next2_remainder) = next2.as_chunks::<8>();
    let (output_chunks, output_remainder) = output.as_chunks_mut::<8>();
    for ((((prev, prev2), next), next2), output) in prev_chunks
        .iter()
        .zip(prev2_chunks)
        .zip(next_chunks)
        .zip(next2_chunks)
        .zip(output_chunks)
    {
        let mut value = (Simd::from_array(*next).cast::<i32>()
            - Simd::from_array(*prev).cast::<i32>())
            * Simd::splat(42);
        value -= (Simd::from_array(*next2).cast::<i32>() - Simd::from_array(*prev2).cast::<i32>())
            * Simd::splat(5);
        if double {
            value += value;
        }
        *output = round2_signed_simd(value, 7).cast::<i16>().to_array();
    }
    for col in 0..output_remainder.len() {
        let mut value = 42 * (i32::from(next_remainder[col]) - i32::from(prev_remainder[col]))
            - 5 * (i32::from(next2_remainder[col]) - i32::from(prev2_remainder[col]));
        if double {
            value *= 2;
        }
        output_remainder[col] = round2_signed_i32(value, 7) as i16;
    }
}

#[allow(clippy::too_many_arguments)]
fn solve_unit(
    gradient_x: &[i16],
    gradient_y: &[i16],
    difference: &[i16],
    stride: usize,
    unit_x: usize,
    unit_y: usize,
    unit_size: usize,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    let [mut su2, mut sv2, mut suv, mut suw, mut svw] = match unit_size {
        4 => solve_unit_sums::<4>(gradient_x, gradient_y, difference, stride, unit_x, unit_y),
        8 => solve_unit_sums::<8>(gradient_x, gradient_y, difference, stride, unit_x, unit_y),
        _ => {
            let area = (unit_size * unit_size) as i32;
            let mut sums = [area, area, 0, 0, 0];
            for row in unit_y..unit_y + unit_size {
                for col in unit_x..unit_x + unit_size {
                    let index = row * stride + col;
                    let u = i32::from(gradient_x[index]);
                    let v = i32::from(gradient_y[index]);
                    let w = i32::from(difference[index]);
                    sums[0] += u * u;
                    sums[1] += v * v;
                    sums[2] += u * v;
                    sums[3] += u * w;
                    sums[4] += v * w;
                }
            }
            [sums[0], sums[1], sums[2], sums[3], sums[4]]
        }
    };

    let max_product_bits = (1 + msb(su2 as u32)) + (1 + msb(sv2 as u32));
    let max_product_bits = max_product_bits
        .max((1 + msb(sv2 as u32)) + (1 + msb(suw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(su2 as u32)) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(suw.unsigned_abs())));
    let reduction = max_product_bits.saturating_sub(MAX_LS_BITS - 3) >> 1;
    su2 = round2_signed_i32(su2, reduction);
    sv2 = round2_signed_i32(sv2, reduction);
    suv = round2_signed_i32(suv, reduction);
    suw = round2_signed_i32(suw, reduction);
    svw = round2_signed_i32(svw, reduction);

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
            (row * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
        [
            (row * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
    ])
}

fn solve_unit_sums<const N: usize>(
    gradient_x: &[i16],
    gradient_y: &[i16],
    difference: &[i16],
    stride: usize,
    unit_x: usize,
    unit_y: usize,
) -> [i32; 5] {
    let mut sums = [Simd::<i32, N>::splat(0); 5];
    for row in unit_y..unit_y + N {
        let index = row * stride + unit_x;
        let u = Simd::<i16, N>::from_slice(&gradient_x[index..]).cast::<i32>();
        let v = Simd::<i16, N>::from_slice(&gradient_y[index..]).cast::<i32>();
        let w = Simd::<i16, N>::from_slice(&difference[index..]).cast::<i32>();
        sums[0] += u * u;
        sums[1] += v * v;
        sums[2] += u * v;
        sums[3] += u * w;
        sums[4] += v * w;
    }
    let area = (N * N) as i32;
    [
        sums[0].reduce_sum() + area,
        sums[1].reduce_sum() + area,
        sums[2].reduce_sum(),
        sums[3].reduce_sum(),
        sums[4].reduce_sum(),
    ]
}

fn divide_and_round(values: [i32; 2], denominator: i32, shift: i32) -> Result<[i32; 2]> {
    let (denominator_shift, inverse) = if denominator == 1 {
        (0i32, 1i32)
    } else {
        let (denominator_shift, inverse) = resolve_divisor_32(denominator as u32)?;
        (i32::from(denominator_shift), i32::from(inverse))
    };
    let inverse_msb = msb(inverse as u32);
    let mut result = [0i32; 2];
    for (output, value) in result.iter_mut().zip(values) {
        if value == 0 {
            continue;
        }
        let sign = value.signum();
        let mut magnitude = value.abs();
        let reduction = (msb(magnitude as u32) + inverse_msb + 4).saturating_sub(MAX_LS_BITS);
        magnitude = round2_signed_i32(magnitude, reduction);
        let increase = shift + reduction as i32 - denominator_shift;
        magnitude = if increase <= -31 {
            let reduced = round2_signed_i32(magnitude, (-increase - 30) as u32);
            round2_signed_i32(reduced * inverse, 30)
        } else if increase >= 0 {
            magnitude * inverse * (1i32 << increase)
        } else {
            round2_signed_i32(magnitude * inverse, (-increase) as u32)
        };
        *output = sign * magnitude;
    }
    Ok(result)
}

fn msb(value: u32) -> u32 {
    u32::BITS - 1 - value.max(1).leading_zeros()
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
    fn into_reuses_scratch_and_delta_allocations() {
        let predictor = vec![80; 8 * 8];
        let mut scratch = OptflowScratch::default();
        derive_optflow_mv_deltas_into(
            &predictor,
            &predictor,
            8,
            8,
            4,
            BitDepth::Eight,
            [1, -2],
            &mut scratch,
        )
        .unwrap();
        let samples_ptr = scratch.samples.as_ptr();
        let deltas_ptr = scratch.deltas.as_ptr();

        derive_optflow_mv_deltas_into(
            &predictor,
            &predictor,
            8,
            8,
            4,
            BitDepth::Eight,
            [1, -2],
            &mut scratch,
        )
        .unwrap();

        assert_eq!(scratch.samples.as_ptr(), samples_ptr);
        assert_eq!(scratch.deltas.as_ptr(), deltas_ptr);
        assert_eq!(scratch.deltas, vec![[[0; 2]; 2]; 4]);
    }

    #[test]
    fn into_clears_deltas_when_validation_fails() {
        let predictor = vec![80; 8 * 8];
        let mut scratch = OptflowScratch {
            samples: Vec::new(),
            deltas: vec![[[1; 2]; 2]],
        };

        assert!(matches!(
            derive_optflow_mv_deltas_into(
                &predictor[..63],
                &predictor,
                8,
                8,
                8,
                BitDepth::Eight,
                [1, -1],
                &mut scratch,
            ),
            Err(ReconError::BufferLengthMismatch {
                expected: 64,
                actual: 63,
            })
        ));
        assert!(scratch.deltas.is_empty());
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
    fn ten_bit_normalization_matches_eight_bit_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        let left_10bit: Vec<u16> = left.iter().map(|sample| sample * 4).collect();
        let right_10bit: Vec<u16> = right.iter().map(|sample| sample * 4).collect();

        assert_eq!(
            derive_optflow_mv_deltas(&left_10bit, &right_10bit, 8, 8, 8, BitDepth::Ten, [1, -1],)
                .unwrap(),
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]).unwrap(),
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

    #[test]
    fn maximum_ten_bit_contrast_fits_i16_scratch_and_i32_solver() {
        let left: Vec<u16> = (0..8 * 8)
            .map(|index| if index & 1 == 0 { 1023 } else { 0 })
            .collect();
        let right: Vec<u16> = left.iter().map(|&value| 1023 - value).collect();

        let deltas =
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Ten, [2, -1]).unwrap();
        assert!(deltas[0].iter().flatten().all(|&value| value.abs() <= 16));
    }

    #[test]
    fn rejects_predictors_outside_active_bit_depth() {
        let mut predictor = vec![0; 8 * 8];
        predictor[7] = 256;
        let other = vec![0; 8 * 8];

        assert!(matches!(
            derive_optflow_mv_deltas(&predictor, &other, 8, 8, 8, BitDepth::Eight, [1, -1],),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 0,
                sample_index: 7,
                value: 256,
                max: 255,
            })
        ));
    }

    #[test]
    fn predictor_range_error_keeps_sample_then_predictor_order() {
        let mut left = vec![0; 8 * 8];
        let mut right = vec![0; 8 * 8];
        right[2] = 256;
        left[3] = 257;

        assert!(matches!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 1,
                sample_index: 2,
                value: 256,
                max: 255,
            })
        ));

        left[2] = 258;
        assert!(matches!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 0,
                sample_index: 2,
                value: 258,
                max: 255,
            })
        ));
    }

    #[test]
    fn strided_predictor_range_error_uses_logical_sample_order() {
        const STRIDE: usize = 12;
        let mut left = vec![0; STRIDE * 8];
        let mut right = vec![0; STRIDE * 8];
        right[STRIDE] = 256;
        left[STRIDE + 1] = 257;
        let mut scratch = OptflowScratch::default();

        assert!(matches!(
            derive_optflow_mv_delta_8x8_strided_into(
                &left,
                0,
                &right,
                0,
                STRIDE,
                BitDepth::Eight,
                [1, -1],
                &mut scratch,
            ),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 1,
                sample_index: 8,
                value: 256,
                max: 255,
            })
        ));
    }
}
