// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A two-axis 16x16 `BILINEAR` block at `(x0, y0)` whose clip bounds are
/// `[x0 + 1, x0 + 15] x [y0 + 1, y0 + 15]`, from `[x0 + 1, y0 + 1]`.
///
/// Its clipped columns read `[1, 1, 2, .., 15]` and `[1, 2, .., 15, 15]`, so
/// two loads and one lane slide per half form both tap windows of a row. Each
/// source row is filtered horizontally once: row `r` blends the rows
/// `clamp(r, 1, 15)` and `clamp(r + 1, 1, 15)`.
pub(super) fn bilinear_16x16_clipped<T: ReconSample, O: BilinearOutput>(
    reference: &ReferencePlaneView<'_, T>,
    [x, y]: [usize; 2],
    [h_phase, v_phase]: [u16; 2],
    max_sample: u16,
    output: &mut [O],
    output_stride: usize,
) {
    let horizontal = |row: usize| {
        let source = reference.row(y + row);
        let (low, high) = (
            reference_lanes::<8, T>(source, x),
            reference_lanes::<8, T>(source, x + 7),
        );
        let left = [
            std::simd::simd_swizzle!(low, [0, 0, 1, 2, 3, 4, 5, 6]),
            high,
        ];
        let right = [
            low,
            std::simd::simd_swizzle!(high, [1, 2, 3, 4, 5, 6, 7, 7]),
        ];
        [0, 1]
            .map(|half| left[half] * Simd::splat(16 - h_phase) + right[half] * Simd::splat(h_phase))
    };
    let mut top = horizontal(0);
    for row in 0..16 {
        let bottom = if (1..15).contains(&row) {
            horizontal(row)
        } else {
            top
        };
        let destination = &mut output[row * output_stride..][..16];
        for (half, lanes) in destination.chunks_exact_mut(8).enumerate() {
            let blended = tap_mac(
                tap_mac(Simd::splat(0), top[half].cast(), i32::from(16 - v_phase)),
                bottom[half].cast(),
                i32::from(v_phase),
            );
            let filtered = round2_simd(blended, 8)
                .cast::<u16>()
                .simd_min(Simd::splat(max_sample));
            O::store(filtered, lanes);
        }
        top = bottom;
    }
}

/// The clipped column windows `[1, 1, 2, .., 15]` (`left`) and
/// `[1, 2, .., 15, 15]` (`right`) of [`bilinear_16x16_clipped`] row `row`.
#[allow(clippy::inline_always, reason = "measured bilinear subpel hot path")]
#[inline(always)]
fn clipped_windows<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    [x, y]: [usize; 2],
    row: usize,
) -> [[Simd<u16, 8>; 2]; 2] {
    let source = reference.row(y + row);
    let (low, high) = (
        reference_lanes::<8, T>(source, x),
        reference_lanes::<8, T>(source, x + 7),
    );
    [
        [
            std::simd::simd_swizzle!(low, [0, 0, 1, 2, 3, 4, 5, 6]),
            high,
        ],
        [
            low,
            std::simd::simd_swizzle!(high, [1, 2, 3, 4, 5, 6, 7, 7]),
        ],
    ]
}

/// The one-axis kernels of [`bilinear_16x16_clipped`]: each row blends its
/// two column windows, or with `VERTICAL` the `left` windows of rows
/// `clamp(r, 1, 15)` and `clamp(r + 1, 1, 15)`.
pub(super) fn bilinear_16x16_clipped_one_axis<
    T: ReconSample,
    O: BilinearOutput,
    const VERTICAL: bool,
>(
    reference: &ReferencePlaneView<'_, T>,
    origin: [usize; 2],
    phase: i32,
    output: &mut [O],
    output_stride: usize,
) {
    let mut top = clipped_windows(reference, origin, 0)[0];
    for row in 0..16 {
        let destination = &mut output[row * output_stride..][..16];
        let [left, right] = if VERTICAL {
            let bottom = if (1..15).contains(&row) {
                clipped_windows(reference, origin, row)[0]
            } else {
                top
            };
            let pair = [top, bottom];
            top = bottom;
            pair
        } else {
            clipped_windows(reference, origin, row.max(1) - 1)
        };
        for (half, lanes) in destination.chunks_exact_mut(8).enumerate() {
            O::store(bilinear_u16(left[half], right[half], phase), lanes);
        }
    }
}

/// Output columns 4..12 of the last block that
/// [`subpel_predict_12x12_bilinear_overlap_into`] predicted, with the reference
/// and geometry it read them at. Only that function writes it; the held
/// reference borrow keeps those samples unchanged while the carry lives.
#[derive(Default)]
pub struct BilinearOverlapCarry<'a, T: ReconSample> {
    source: Option<(&'a [T], usize, [i32; 4])>,
    columns: [[u16; 8]; 12],
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
    let block = (window, ref_stride, output, stride, &mut carry.columns);
    if reuse {
        bilinear_12x12_block::<T, true>(block, (h_phase, v_phase));
    } else {
        bilinear_12x12_block::<T, false>(block, (h_phase, v_phase));
    }
    carry.source = Some((reference.samples, ref_stride, geometry));
    Ok(true)
}

/// A source window and its stride, an output and its stride, and the carry
/// columns.
type Block12x12<'s, T> = (&'s [T], usize, &'s mut [u16], usize, &'s mut [[u16; 8]; 12]);

/// Filters a 12x12 bilinear block whose 13x13 source window holds every tap
/// unclamped into the strided output, keeping columns 4..12 in the carry
/// columns. With `REUSE`, carried columns 8..12 become columns 0..4.
#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
fn bilinear_12x12_block<T: ReconSample, const REUSE: bool>(
    block: Block12x12<'_, T>,
    (h_phase, v_phase): (i32, i32),
) {
    let lanes = |line: &[T], col: usize| reference_lanes::<8, T>(line, col);
    let (left_weight, right_weight) = (
        Simd::splat(16 - h_phase as u16),
        Simd::splat(h_phase as u16),
    );
    let horizontal = |line: &[T], col: usize| {
        lanes(line, col) * left_weight + lanes(line, col + 1) * right_weight
    };
    match (h_phase, v_phase) {
        (0, 0) => bilinear_12x12_rows::<T, REUSE>(block, lanes, |top, _| top),
        (_, 0) => {
            bilinear_12x12_rows::<T, REUSE>(block, horizontal, |top, _| {
                (top + Simd::splat(8)) >> 4
            });
        }
        (0, v_phase) => bilinear_12x12_rows::<T, REUSE>(block, lanes, |top, bottom| {
            bilinear_u16(top, bottom, v_phase)
        }),
        (_, v_phase) => bilinear_12x12_rows::<T, REUSE>(block, horizontal, |top, bottom| {
            let blended = tap_mac(
                tap_mac(Simd::splat(0), top.cast(), 16 - v_phase),
                bottom.cast(),
                v_phase,
            );
            round2_simd(blended, 8).cast::<u16>()
        }),
    }
}

/// Runs the rows of [`bilinear_12x12_block`] in one pass: `horizontal`
/// filters eight columns of one source line and `vertical` blends two such
/// results.
#[allow(clippy::inline_always, reason = "measured TIP predictor hot path")]
#[inline(always)]
fn bilinear_12x12_rows<T: ReconSample, const REUSE: bool>(
    (window, ref_stride, output, stride, columns): Block12x12<'_, T>,
    horizontal: impl Fn(&[T], usize) -> Simd<u16, 8>,
    vertical: impl Fn(Simd<u16, 8>, Simd<u16, 8>) -> Simd<u16, 8>,
) {
    let line = |row: usize| &window[row * ref_stride..row * ref_stride + 13];
    let mut top = [horizontal(line(0), 0), horizontal(line(0), 4)];
    for (row, carried) in columns.iter_mut().enumerate() {
        let source = line(row + 1);
        let bottom = [horizontal(source, 0), horizontal(source, 4)];
        let output = &mut output[row * stride..row * stride + 12];
        if REUSE {
            output[..4].copy_from_slice(&carried[4..]); // splot-copy-ok: the left neighbour's last four filtered columns
        } else {
            vertical(top[0], bottom[0]).copy_to_slice(&mut output[..8]);
        }
        let right = vertical(top[1], bottom[1]);
        right.copy_to_slice(&mut output[4..]);
        *carried = right.to_array();
        top = bottom;
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

    /// An overflowing stride or a short output fails without changing the
    /// carry, so the next unit still takes the carried columns: unchanged,
    /// they match the block prediction; poisoned, they do not.
    #[test]
    fn bilinear_12x12_overlap_errors_keep_the_carry() {
        let samples = (0..48 * 40)
            .map(|index| (index * 37 % 256) as u16)
            .collect::<Vec<u16>>();
        let view = ReferencePlaneView::new(&samples, 48, 40).unwrap();
        let unit = |x: i32| SubpelPredictParams {
            start_x: (x << SCALE_SUBPEL_BITS) + (5 << 6),
            start_y: (8 << SCALE_SUBPEL_BITS) + (9 << 6),
            ..full_pel_params(InterpolationFilter::Bilinear, 12, 12, x, 8, 48, 40)
        };
        let mut expected = vec![0; 12 * 12];
        subpel_predict_block_into(&view, &unit(16), &mut expected).unwrap();
        let kernel = subpel_predict_12x12_bilinear_overlap_into;
        for poison in [false, true] {
            let (mut output, mut carry) = (vec![0; 11 * 16 + 12], BilinearOverlapCarry::default());
            assert!(kernel(&view, &unit(8), &mut output, 16, &mut carry).unwrap());
            let overflow = kernel(
                &view,
                &unit(16),
                &mut output,
                usize::MAX / 11 + 1,
                &mut carry,
            );
            assert!(matches!(
                overflow,
                Err(ReconError::ArithmeticOverflow { .. })
            ));
            let short = kernel(&view, &unit(16), &mut output[1..], 16, &mut carry);
            assert!(matches!(
                short,
                Err(ReconError::BufferLengthMismatch { .. })
            ));
            if poison {
                carry.columns = [[u16::MAX; 8]; 12];
            }
            output.fill(0x5a5a);
            assert!(kernel(&view, &unit(16), &mut output, 16, &mut carry).unwrap());
            let block = (0..12)
                .flat_map(|row| output[row * 16..][..12].iter().copied())
                .collect::<Vec<u16>>();
            assert_eq!(block == expected, !poison);
        }
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
            carry.columns = [[u16::MAX; 8]; 12];
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
    /// columns only when the last predicted unit lies eight samples left in the
    /// same plane, stride, row and phases; declined units keep the carry. A
    /// unit clamped at the left plane edge, a clipped unit and units one sample
    /// past the right plane edge or the last readable row are declined.
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
            let replaced = vec![(view, unit(7, 8)), (view, unit(23, 8))];
            let other_h = at(15, 8, ((h_phase + 1) % 16, v_phase));
            let other_v = at(15, 8, (h_phase, (v_phase + 1) % 16));
            let cases = [
                (&left, (view, unit(15, 8)), true),
                (&kept, (view, unit(15, 8)), true),
                (&right_edge, (view, unit(width - 11, 8)), true),
                (&last_rows, (view, unit(15, rows - 11)), true),
                (&left, (view, unit(15, 9)), false),
                (&replaced, (view, unit(15, 8)), false),
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
