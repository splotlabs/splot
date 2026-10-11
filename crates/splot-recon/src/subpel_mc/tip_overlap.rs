// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use std::simd::simd_swizzle;

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

/// The last four output columns of the last block that
/// [`subpel_predict_12x12_bilinear_overlap_into`] predicted, with the reference
/// and geometry it read them at. Only that function writes it; the held
/// reference borrow keeps those samples unchanged while the carry lives.
#[derive(Default)]
pub struct BilinearOverlapCarry<'a, T: ReconSample> {
    source: Option<(&'a [T], usize, [i32; 4])>,
    columns: [[u16; 4]; 12],
}

/// Predicts an unscaled 12x12 bilinear block whose every tap lies inside both
/// the clip bounds and the plane, so no sample is clamped, writing it at
/// `stride`; returns `Ok(false)` without writing for any other block.
///
/// When `carry` holds the block eight samples left in the same reference, row
/// and phases, its columns become columns 0..4 and only columns 4..12 are
/// filtered. A predicted block replaces the carry; a declined block or an
/// error leaves it unchanged.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when `output` cannot hold the
/// strided block, or [`ReconError::ArithmeticOverflow`] when its length
/// overflows.
pub fn subpel_predict_12x12_bilinear_overlap_into<'a, T: ReconSample>(
    reference: &ReferencePlaneView<'a, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    stride: usize,
    carry: &mut BilinearOverlapCarry<'a, T>,
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
    let needed = stride
        .checked_mul(SIZE - 1)
        .and_then(|rows| rows.checked_add(SIZE))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "12x12 bilinear overlap output length",
        })?;
    if output.len() < needed {
        return Err(ReconError::BufferLengthMismatch {
            expected: needed,
            actual: output.len(),
        });
    }
    let output = &mut output[..needed];
    let h_phase = (params.start_x >> 6) & SUBPEL_MASK;
    let v_phase = (params.start_y >> 6) & SUBPEL_MASK;
    let geometry = [x0, y0, h_phase, v_phase];
    let (x0, y0) = (x0 as usize, y0 as usize);
    let ref_stride = reference.stride;
    let origin = y0 * ref_stride + x0;
    let Some(window) = reference
        .samples
        .get(origin..origin + SIZE * ref_stride + SIZE + 1)
    else {
        return Ok(false);
    };
    let reuse = carry
        .source
        .is_some_and(|(samples, carry_stride, [x, y, h, v])| {
            samples.as_ptr() == reference.samples.as_ptr()
                && carry_stride == ref_stride
                && [x + 8, y, h, v] == geometry
        });
    let phases = (h_phase, v_phase);
    if !reuse {
        bilinear_12x12_block::<T, 0>(window, ref_stride, phases, |row, lanes| {
            lanes.copy_to_slice(&mut output[row * stride..row * stride + 8]);
        });
    }
    let columns = &mut carry.columns;
    bilinear_12x12_block::<T, 4>(window, ref_stride, phases, |row, lanes| {
        let output = &mut output[row * stride..row * stride + SIZE];
        if reuse {
            output[..4].copy_from_slice(&columns[row]); // splot-copy-ok: the left neighbour's last four filtered columns
        }
        lanes.copy_to_slice(&mut output[4..]);
        columns[row] = simd_swizzle!(lanes, [4, 5, 6, 7]).to_array();
    });
    carry.source = Some((reference.samples, ref_stride, geometry));
    Ok(true)
}

/// Filters the eight columns from `COL` of a 12x12 bilinear block whose
/// 13x13 source `window` holds every tap unclamped, passing each row to
/// `store`.
#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
fn bilinear_12x12_block<T: ReconSample, const COL: usize>(
    window: &[T],
    ref_stride: usize,
    (h_phase, v_phase): (i32, i32),
    mut store: impl FnMut(usize, Simd<u16, 8>),
) {
    const SIZE: usize = 12;
    let at = |row: usize| &window[row * ref_stride + COL..];
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::full_pel_params;
    use super::*;

    #[test]
    fn bilinear_12x12_overlap_matches_the_block_prediction() {
        let (ref_w, ref_h) = (48usize, 40usize);
        let wide = (0..ref_w * ref_h)
            .map(|index| ((index * 37 + index / ref_w * 19) % 1024) as u16)
            .collect::<Vec<u16>>();
        let narrow = wide
            .iter()
            .map(|&sample| (sample >> 2) as u8)
            .collect::<Vec<u8>>();
        check_bilinear_12x12(&wide, &wide.clone(), BitDepth::Ten);
        check_bilinear_12x12(&narrow, &narrow.clone(), BitDepth::Eight);
    }

    #[test]
    fn bilinear_12x12_overlap_rejects_an_overflowing_stride() {
        let samples = vec![0u16; 48 * 40];
        let view = ReferencePlaneView::new(&samples, 48, 40).unwrap();
        let params = full_pel_params(InterpolationFilter::Bilinear, 12, 12, 8, 8, 48, 40);
        let (mut output, mut carry) = ([0; 11 * 16 + 12], BilinearOverlapCarry::default());
        let stride = usize::MAX / 11 + 1;
        let kernel = subpel_predict_12x12_bilinear_overlap_into;
        let predicted = kernel(&view, &params, &mut output, stride, &mut carry);
        assert!(matches!(
            predicted,
            Err(ReconError::ArithmeticOverflow { .. })
        ));
        assert!(carry.source.is_none());
    }

    /// Predicts `query` into a garbage buffer with a carry that first predicted
    /// `setup`, returning its 12x12 block. `poison` overwrites the carried columns
    /// before `query`, so a block that took them differs from the prediction.
    fn carried_12x12<'a, T: ReconSample>(
        setup: &[(ReferencePlaneView<'a, T>, SubpelPredictParams)],
        (view, params): (ReferencePlaneView<'a, T>, SubpelPredictParams),
        poison: bool,
    ) -> Vec<u16> {
        let stride = 16;
        let mut carry = BilinearOverlapCarry::default();
        let mut output = vec![0; 11 * stride + 12];
        for (view, params) in setup {
            subpel_predict_12x12_bilinear_overlap_into(
                view,
                params,
                &mut output,
                stride,
                &mut carry,
            )
            .unwrap();
        }
        if poison {
            carry.columns = [[u16::MAX; 4]; 12];
        }
        output.fill(0x5a5a);
        let predicted = subpel_predict_12x12_bilinear_overlap_into(
            &view,
            &params,
            &mut output,
            stride,
            &mut carry,
        );
        assert!(predicted.unwrap(), "{params:?}");
        (0..12)
            .flat_map(|row| output[row * stride..][..12].iter().copied())
            .collect()
    }

    /// For every phase pair, predicts the 12x12 centres of TIP-like units at
    /// `(x, y)` (bounds `x - 3..=x + 11`) after other units. Each matches the
    /// block prediction whatever the output held before, and takes the carried
    /// columns only after the unit eight samples left in the same plane, stride,
    /// row and phases; declined units keep the carry. A unit clamped at the left
    /// plane edge, a clipped unit and units one sample past the right plane edge
    /// or the last readable row are declined.
    fn check_bilinear_12x12<T: ReconSample>(samples: &[T], copy: &[T], bit_depth: BitDepth) {
        let plane = |samples, stride| {
            ReferencePlaneView::from_published_strided(samples, stride, 40, 40, 30).unwrap()
        };
        let (view, copied, restrided) = (plane(samples, 48), plane(copy, 48), plane(samples, 47));
        let (width, rows) = (view.width as i32, view.readable_rows as i32);
        for (h_phase, v_phase) in (0..16).flat_map(|h| (0..16).map(move |v| (h, v))) {
            let at = |x: i32, y: i32, (h_phase, v_phase): (i32, i32)| SubpelPredictParams {
                interp: InterpolationFilter::Bilinear,
                w: 12,
                h: 12,
                start_x: ((x - 2) << SCALE_SUBPEL_BITS) + (h_phase << 6),
                start_y: ((y - 2) << SCALE_SUBPEL_BITS) + (v_phase << 6),
                step_x: 1 << SCALE_SUBPEL_BITS,
                step_y: 1 << SCALE_SUBPEL_BITS,
                first_x: (x - 3).max(0),
                first_y: (y - 3).max(0),
                last_x: x + 11,
                last_y: y + 11,
                bit_depth,
            };
            let unit = |x, y| at(x, y, (h_phase, v_phase));
            let clipped = SubpelPredictParams {
                last_x: 5 + 11,
                ..unit(7, 8)
            };
            let declined = [
                unit(-1, 8),
                clipped,
                unit(width - 10, 8),
                unit(7, rows - 10),
            ];
            let kernel = subpel_predict_12x12_bilinear_overlap_into;
            for params in &declined {
                let (mut output, mut carry) = ([0; 11 * 16 + 12], BilinearOverlapCarry::default());
                let predicted = kernel(&view, params, &mut output, 16, &mut carry);
                assert!(!predicted.unwrap(), "{params:?}");
            }
            let left = vec![(view, unit(7, 8))];
            let mut kept = left.clone();
            kept.extend(declined.map(|params| (view, params)));
            let right_edge = vec![(view, unit(width - 19, 8))];
            let last_rows = vec![(view, unit(7, rows - 11))];
            let other_h = at(15, 8, ((h_phase + 1) % 16, v_phase));
            let other_v = at(15, 8, (h_phase, (v_phase + 1) % 16));
            let cases = [
                (&left, (view, unit(15, 8)), true),
                (&kept, (view, unit(15, 8)), true),
                (&right_edge, (view, unit(width - 11, 8)), true),
                (&last_rows, (view, unit(15, rows - 11)), true),
                (&left, (view, unit(15, 9)), false),
                (&left, (view, unit(11, 8)), false),
                (&left, (view, other_h), false),
                (&left, (view, other_v), false),
                (&left, (copied, unit(15, 8)), false),
                (&left, (restrided, unit(15, 8)), false),
            ];
            for (setup, query, reuse) in cases {
                let mut expected = vec![0; 12 * 12];
                subpel_predict_block_into(&query.0, &query.1, &mut expected).unwrap();
                let predicted = carried_12x12(setup, query, false);
                assert_eq!(predicted, expected, "{:?}", query.1);
                let poisoned = carried_12x12(setup, query, true);
                assert_eq!(poisoned != expected, reuse, "{:?}", query.1);
            }
        }
    }
}
