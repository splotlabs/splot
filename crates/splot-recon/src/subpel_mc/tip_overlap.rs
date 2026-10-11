// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
pub(super) fn overlap_bilinear_u16x8<T: ReconSample>(
    top: &[T],
    bottom: &[T],
    start: usize,
    right_start: Option<usize>,
    h_phase: i32,
    v_phase: i32,
) -> Simd<u16, 8> {
    let top_left_samples = reference_lanes::<8, T>(top, start);
    match (h_phase, v_phase) {
        (0, v_phase) => {
            let v_phase = v_phase as u16;
            (top_left_samples * Simd::splat(16 - v_phase)
                + reference_lanes::<8, T>(bottom, start) * Simd::splat(v_phase)
                + Simd::splat(8))
                >> 4
        }
        (h_phase, 0) => {
            let h_phase = h_phase as u16;
            let top_right = right_start.map_or_else(
                || top_left_samples.shift_elements_left::<1>(top_left_samples[7]),
                |right_start| reference_lanes::<8, T>(top, right_start),
            );
            (top_left_samples * Simd::splat(16 - h_phase)
                + top_right * Simd::splat(h_phase)
                + Simd::splat(8))
                >> 4
        }
        (h_phase, v_phase) => {
            let h_phase = h_phase as u16;
            let bottom_left_samples = reference_lanes::<8, T>(bottom, start);
            let (top_right, bottom_right) = right_start.map_or_else(
                || {
                    (
                        top_left_samples.shift_elements_left::<1>(top_left_samples[7]),
                        bottom_left_samples.shift_elements_left::<1>(bottom_left_samples[7]),
                    )
                },
                |right_start| {
                    (
                        reference_lanes::<8, T>(top, right_start),
                        reference_lanes::<8, T>(bottom, right_start),
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

/// Predicts an unscaled 12x12 bilinear block whose every tap lies inside both
/// the clip bounds and the plane, so no sample is clamped, writing it at
/// `stride`; returns `Ok(false)` without writing for any other block.
///
/// With `reuse`, `output` must hold the same reference's prediction for the
/// same motion vector eight samples left; its columns 8..12 become columns
/// 0..4 and only columns 4..12 are filtered.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when `output` cannot hold the
/// strided block.
pub fn subpel_predict_12x12_bilinear_overlap_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    stride: usize,
    reuse: bool,
) -> Result<bool> {
    const SIZE: usize = 12;
    if params.interp != InterpolationFilter::Bilinear
        || params.step_x != 1 << SCALE_SUBPEL_BITS
        || params.step_y != 1 << SCALE_SUBPEL_BITS
        || params.w != SIZE
        || params.h != SIZE
        || stride < SIZE
    {
        return Ok(false);
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    if x0 < params.first_x.max(0)
        || y0 < params.first_y.max(0)
        || x0 + SIZE as i32 > params.last_x.min(reference.width as i32 - 1)
        || y0 + SIZE as i32 > params.last_y.min(reference.readable_rows as i32 - 1)
    {
        return Ok(false);
    }
    let needed = (SIZE - 1) * stride + SIZE;
    if output.len() < needed {
        return Err(ReconError::BufferLengthMismatch {
            expected: needed,
            actual: output.len(),
        });
    }
    let output = &mut output[..needed];
    let h_phase = (params.start_x >> 6) & SUBPEL_MASK;
    let v_phase = (params.start_y >> 6) & SUBPEL_MASK;
    let (x0, y0) = (x0 as usize, y0 as usize);
    let ref_stride = reference.stride;
    let origin = y0 * ref_stride + x0;
    let Some(window) = reference
        .samples
        .get(origin..origin + SIZE * ref_stride + SIZE + 1)
    else {
        return Ok(false);
    };
    if reuse {
        for row in 0..SIZE {
            output.copy_within(row * stride + 8..row * stride + SIZE, row * stride); // splot-copy-ok: retain the four columns the left neighbour already filtered
        }
    } else {
        bilinear_12x12_block::<T, 0>(window, ref_stride, (h_phase, v_phase), output, stride);
    }
    bilinear_12x12_block::<T, 4>(window, ref_stride, (h_phase, v_phase), output, stride);
    Ok(true)
}

/// Filters the eight columns from `COL` of a 12x12 bilinear block whose
/// 13x13 source `window` holds every tap unclamped.
#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
fn bilinear_12x12_block<T: ReconSample, const COL: usize>(
    window: &[T],
    ref_stride: usize,
    (h_phase, v_phase): (i32, i32),
    output: &mut [u16],
    stride: usize,
) {
    const SIZE: usize = 12;
    let at = |row: usize| &window[row * ref_stride + COL..];
    let mut store = |row: usize, lanes: Simd<u16, 8>| {
        lanes.copy_to_slice(&mut output[row * stride + COL..row * stride + COL + 8]);
    };
    match (h_phase, v_phase) {
        (0, 0) => (0..SIZE).for_each(|row| store(row, reference_lanes::<8, T>(at(row), 0))),
        (h_phase, 0) => (0..SIZE).for_each(|row| {
            let source = at(row);
            let lanes = bilinear_u16(
                reference_lanes::<8, T>(source, 0),
                reference_lanes::<8, T>(source, 1),
                h_phase,
            );
            store(row, lanes);
        }),
        (0, v_phase) => {
            let mut top = reference_lanes::<8, T>(at(0), 0);
            for row in 0..SIZE {
                let bottom = reference_lanes::<8, T>(at(row + 1), 0);
                store(row, bilinear_u16(top, bottom, v_phase));
                top = bottom;
            }
        }
        (h_phase, v_phase) => {
            let left_weight = Simd::splat(16 - h_phase as u16);
            let right_weight = Simd::splat(h_phase as u16);
            let horizontal = |row: usize| {
                let source = at(row);
                reference_lanes::<8, T>(source, 0) * left_weight
                    + reference_lanes::<8, T>(source, 1) * right_weight
            };
            let mut top = horizontal(0);
            for row in 0..SIZE {
                let bottom = horizontal(row + 1);
                let blended = tap_mac(
                    tap_mac(Simd::splat(0), top.cast(), 16 - v_phase),
                    bottom.cast(),
                    v_phase,
                );
                store(row, round2_simd(blended, 8).cast::<u16>());
                top = bottom;
            }
        }
    }
}
