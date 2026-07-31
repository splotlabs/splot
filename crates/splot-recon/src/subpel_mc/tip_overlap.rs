// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const SHIFT: usize = 8;
const RECOMPUTED_COLUMNS: [usize; 10] = [0, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Reuses the six stable columns shared by horizontally adjacent 16x16
/// bilinear TIP refine-MV predictors and fills the other ten columns.
///
/// `output` must initially contain the prediction for the same reference,
/// motion vector, and vertical position eight samples left of `params.start_x`.
/// Returns `Ok(false)` when the current predictor is not an eligible unscaled
/// 16x16 bilinear block.
///
/// # Errors
///
/// Returns the same parameter and output-length errors as
/// [`subpel_predict_block_into`].
pub fn subpel_predict_16x16_bilinear_horizontal_overlap_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<bool> {
    if params.interp != InterpolationFilter::Bilinear
        || params.step_x != 1 << SCALE_SUBPEL_BITS
        || params.step_y != 1 << SCALE_SUBPEL_BITS
        || params.w != 16
        || params.h != 16
    {
        return Ok(false);
    }
    validate_subpel_params(params)?;
    let output_len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "overlapping subpel prediction sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }

    let h_phase = (params.start_x >> 6) & SUBPEL_MASK;
    let v_phase = (params.start_y >> 6) & SUBPEL_MASK;
    if h_phase == 0 && v_phase == 0 {
        reuse_fullpel(reference, params, output);
        return Ok(true);
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    if params.first_x == x0 + 1
        && params.last_x == x0 + 15
        && params.first_y == y0 + 1
        && params.last_y == y0 + 15
        && fixed_16x16_window_in_bounds(reference, x0, y0)
        && let Some(samples) = T::u16_slice(reference.samples)
    {
        let first = (x0 + 1) as usize;
        let middle = (x0 + 7) as usize;
        let vector = (x0 + 8) as usize;
        let max_sample = i32::from(params.bit_depth.max_sample());
        for row in 0..params.h {
            let top = (y0 + (row as i32).clamp(1, 15)) as usize;
            let bottom = (y0 + (row as i32 + 1).clamp(1, 15)) as usize;
            let top = &samples[top * reference.stride..];
            let bottom = &samples[bottom * reference.stride..];
            let destination = &mut output[row * params.w..][..params.w];
            destination.copy_within(SHIFT.., 0); // splot-copy-ok: retain stable TIP predictor columns
            let filtered =
                overlap_bilinear_u16x8(top, bottom, vector, None, h_phase, v_phase).to_array();
            destination[SHIFT..].copy_from_slice(&filtered); // splot-copy-ok: publish fixed-window TIP overlap lanes
            for (column, left, right) in [(0, first, first), (7, middle, middle + 1)] {
                let top_left = top[left];
                destination[column] = match (h_phase, v_phase) {
                    (0, v_phase) => bilinear_sample(top_left, bottom[left], v_phase),
                    (h_phase, 0) => bilinear_sample(top_left, top[right], h_phase),
                    (h_phase, v_phase) => {
                        let top_right = i32::from(top[right]);
                        let bottom_left = i32::from(bottom[left]);
                        let bottom_right = i32::from(bottom[right]);
                        let top = (16 - h_phase) * i32::from(top_left) + h_phase * top_right;
                        let bottom = (16 - h_phase) * bottom_left + h_phase * bottom_right;
                        round2_i32((16 - v_phase) * top + v_phase * bottom, 8).clamp(0, max_sample)
                            as u16
                    }
                };
            }
        }
        return Ok(true);
    }
    let clipped_columns = RECOMPUTED_COLUMNS.map(|column| {
        [
            (x0 + column as i32)
                .clamp(params.first_x, params.last_x)
                .clamp(0, reference.width as i32 - 1) as usize,
            (x0 + column as i32 + 1)
                .clamp(params.first_x, params.last_x)
                .clamp(0, reference.width as i32 - 1) as usize,
        ]
    });
    let vector_source = (0..8)
        .all(|lane| clipped_columns[lane + 2][0] == clipped_columns[2][0] + lane)
        .then_some(clipped_columns[2][0])
        .and_then(|start| {
            if h_phase == 0 {
                Some((start, None))
            } else if (0..8).all(|lane| clipped_columns[lane + 2][1] == start + lane + 1) {
                Some((start, Some(start + 1)))
            } else {
                (0..8)
                    .all(|lane| clipped_columns[lane + 2][1] == start + (lane + 1).min(7))
                    .then_some((start, None))
            }
        });
    let max_sample = i32::from(params.bit_depth.max_sample());
    for row in 0..params.h {
        let top = (y0 + row as i32)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.height as i32 - 1) as usize;
        let bottom = (y0 + row as i32 + 1)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.height as i32 - 1) as usize;
        let top_samples = reference.row(top);
        let bottom_samples = reference.row(bottom);
        let destination = &mut output[row * params.w..][..params.w];
        destination.copy_within(SHIFT.., 0); // splot-copy-ok: retain stable TIP predictor columns
        let vectorized = if let (Some((start, right_start)), Some(top), Some(bottom)) = (
            vector_source,
            T::u16_slice(top_samples),
            T::u16_slice(bottom_samples),
        ) {
            let filtered =
                overlap_bilinear_u16x8(top, bottom, start, right_start, h_phase, v_phase)
                    .to_array();
            destination[SHIFT..].copy_from_slice(&filtered); // splot-copy-ok: publish eight clamped TIP overlap lanes
            true
        } else {
            false
        };
        let scalar_columns = if vectorized {
            2
        } else {
            RECOMPUTED_COLUMNS.len()
        };
        for (&column, &[left, right]) in RECOMPUTED_COLUMNS[..scalar_columns]
            .iter()
            .zip(&clipped_columns)
        {
            let top_left = top_samples[left].to_u16();
            let value = match (h_phase, v_phase) {
                (0, v_phase) => {
                    let bottom_left = bottom_samples[left].to_u16();
                    bilinear_sample(top_left, bottom_left, v_phase)
                }
                (h_phase, 0) => {
                    let top_right = top_samples[right].to_u16();
                    bilinear_sample(top_left, top_right, h_phase)
                }
                (h_phase, v_phase) => {
                    let top_right = i32::from(top_samples[right].to_u16());
                    let bottom_left = i32::from(bottom_samples[left].to_u16());
                    let bottom_right = i32::from(bottom_samples[right].to_u16());
                    let top = (16 - h_phase) * i32::from(top_left) + h_phase * top_right;
                    let bottom = (16 - h_phase) * bottom_left + h_phase * bottom_right;
                    round2_i32((16 - v_phase) * top + v_phase * bottom, 8).clamp(0, max_sample)
                        as u16
                }
            };
            destination[column] = value;
        }
    }
    Ok(true)
}

#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
pub(super) fn overlap_bilinear_u16x8(
    top: &[u16],
    bottom: &[u16],
    start: usize,
    right_start: Option<usize>,
    h_phase: i32,
    v_phase: i32,
) -> Simd<u16, 8> {
    let top_left_samples = Simd::<u16, 8>::from_slice(&top[start..]);
    match (h_phase, v_phase) {
        (0, v_phase) => {
            let v_phase = v_phase as u16;
            (top_left_samples * Simd::splat(16 - v_phase)
                + Simd::<u16, 8>::from_slice(&bottom[start..]) * Simd::splat(v_phase)
                + Simd::splat(8))
                >> 4
        }
        (h_phase, 0) => {
            let h_phase = h_phase as u16;
            let top_right = right_start.map_or_else(
                || top_left_samples.shift_elements_left::<1>(top_left_samples[7]),
                |right_start| Simd::<u16, 8>::from_slice(&top[right_start..]),
            );
            (top_left_samples * Simd::splat(16 - h_phase)
                + top_right * Simd::splat(h_phase)
                + Simd::splat(8))
                >> 4
        }
        (h_phase, v_phase) => {
            let h_phase = h_phase as u16;
            let bottom_left_samples = Simd::<u16, 8>::from_slice(&bottom[start..]);
            let (top_right, bottom_right) = right_start.map_or_else(
                || {
                    (
                        top_left_samples.shift_elements_left::<1>(top_left_samples[7]),
                        bottom_left_samples.shift_elements_left::<1>(bottom_left_samples[7]),
                    )
                },
                |right_start| {
                    (
                        Simd::<u16, 8>::from_slice(&top[right_start..]),
                        Simd::<u16, 8>::from_slice(&bottom[right_start..]),
                    )
                },
            );
            let top =
                top_left_samples * Simd::splat(16 - h_phase) + top_right * Simd::splat(h_phase);
            let bottom = bottom_left_samples * Simd::splat(16 - h_phase)
                + bottom_right * Simd::splat(h_phase);
            let blended = tap_mac(
                tap_mac(Simd::splat(0), top.cast(), 16 - v_phase),
                bottom.cast(),
                v_phase,
            );
            round2_simd(blended, 8).cast::<u16>()
        }
    }
}

fn reuse_fullpel<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    let retained = params.w - SHIFT;
    for row in 0..params.h {
        let source_row = (y0 + row as i32).clamp(params.first_y, params.last_y) as usize;
        let destination = &mut output[row * params.w..][..params.w];
        destination.copy_within(SHIFT.., 0); // splot-copy-ok: retain seven stable TIP predictor columns
        if let Some(x) = direct_x {
            for (slot, sample) in destination[retained..]
                .iter_mut()
                .zip(&reference.row(source_row)[x + retained..x + retained + SHIFT])
            {
                *slot = sample.to_u16();
            }
        } else {
            for (col, slot) in destination[retained..].iter_mut().enumerate() {
                let source_col =
                    (x0 + (retained + col) as i32).clamp(params.first_x, params.last_x) as usize;
                *slot = reference.sample(source_row, source_col) as u16;
            }
        }
        let first_col = x0.clamp(params.first_x, params.last_x) as usize;
        destination[0] = reference.sample(source_row, first_col) as u16;
    }
}
