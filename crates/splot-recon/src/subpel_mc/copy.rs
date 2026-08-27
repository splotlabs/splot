// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Writes one output row of a single-reference prediction.
///
/// This is the disjoint-rectangle counterpart of
/// [`subpel_predict_block_strided_into`]: callers whose destination is split
/// into exact row slices can reconstruct without first materializing a packed
/// block.
///
/// # Errors
/// Returns the same errors as [`subpel_predict_block_strided_into`], or
/// [`ReconError::BufferLengthMismatch`] when `row` or `output` is invalid.
pub fn subpel_predict_block_row_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    row: usize,
    output: &mut [u16],
) -> Result<()> {
    if row >= params.h || output.len() < params.w {
        return Err(ReconError::BufferLengthMismatch {
            expected: params.w,
            actual: output.len(),
        });
    }
    let mut row_params = *params;
    row_params.h = 1;
    row_params.start_y = params
        .start_y
        .checked_add(
            i32::try_from(row)
                .map_err(|_| ReconError::ArithmeticOverflow {
                    context: "subpel output row",
                })?
                .checked_mul(params.step_y)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "subpel output row position",
                })?,
        )
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel output row position",
        })?;
    let intermediate_height = validate_subpel_params(&row_params)?;
    subpel_predict_block_internal_into_validated(
        reference,
        &row_params,
        params.h,
        INTER_ROUND1_NON_COMPOUND,
        intermediate_height,
        None,
        output,
        params.w,
        ClippedU16SubpelOutput {
            max_sample: i32::from(params.bit_depth.max_sample()),
        },
    )
}

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and the supplied `cwpWeight`, then applies the final § 4.8
/// `Clip1`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_weighted(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }

    Ok(pred0
        .iter()
        .zip(pred1.iter())
        .map(|(&left, &right)| {
            blend_compound_average_weighted_sample(left, right, bit_depth, cwp_weight)
        })
        .collect())
}

/// Blends one pair of § 7.13.3.18 compound intermediate samples with the
/// supplied § 7.13.3.16 `cwpWeight`, then applies the final § 4.8 `Clip1`.
#[inline]
pub fn blend_compound_average_weighted_sample(
    pred0: i32,
    pred1: i32,
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> u16 {
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let blended = round2_i32(
        forward * pred0 + backward * pred1,
        4 + compound_inter_post_round(),
    );
    blended.clamp(0, i32::from(bit_depth.max_sample())) as u16
}

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and `CWP_EQUAL` (`cwpWeight == 8`), then applies the final
/// § 4.8 `Clip1`.
///
/// With `InterRound0 == 3`, compound `InterRound1 == 7`, and `FILTER_BITS == 7`,
/// `InterPostRound == 4`; the equal-weight formula
/// `Round2(8 * p0 + 8 * p1, 4 + InterPostRound)` simplifies exactly to
/// `Round2(p0 + p1, 1 + InterPostRound)`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_equal(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
) -> Result<Vec<u16>> {
    blend_compound_average_weighted(pred0, pred1, bit_depth, 8)
}

pub(super) const fn compound_inter_post_round() -> u32 {
    2 * FILTER_BITS - (INTER_ROUND0 + INTER_ROUND1_COMPOUND)
}

/// The zero-phase unscaled § 7.13.3.18 special case: with `stepX == stepY ==
/// (1 << SCALE_SUBPEL_BITS)` and both sub-pel phases zero, every filter row is
/// the pure `{ .., 128, .. }` tap, so the two-pass convolution is exactly the
/// clipped reference sample scaled by `1 << (2 * FILTER_BITS - (InterRound0 +
/// InterRound1))` — `Round2(128 * v, 3) == 16 * v` and `Round2(2048 * v, 11)
/// == v` / `Round2(2048 * v, 7) == 16 * v` hold exactly for every `v >= 0`
/// because each partial product is a multiple of the rounding divisor.
pub(super) fn subpel_copy_block_into<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    shift_up: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    for r in 0..params.h {
        let row = (y0 + r as i32).clamp(params.first_y, params.last_y) as usize;
        let output = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let row = row.min(reference.readable_rows - 1);
            let start = row * reference.stride + x;
            for (out, sample) in output
                .iter_mut()
                .zip(&reference.samples[start..start + params.w])
            {
                *out = finish.one(i32::from(sample.to_u16()) << shift_up);
            }
        } else {
            let source = reference.row(row);
            let first_x = params.first_x.clamp(0, reference.width as i32 - 1);
            let last_x = params.last_x.clamp(0, reference.width as i32 - 1);
            let leading = (i64::from(first_x) - i64::from(x0)).clamp(0, params.w as i64) as usize;
            let middle_end =
                (i64::from(last_x) - i64::from(x0) + 1).clamp(0, params.w as i64) as usize;
            let first = i32::from(source[first_x as usize].to_u16()) << shift_up;
            for out in &mut output[..leading] {
                *out = finish.one(first);
            }
            if leading < middle_end {
                let middle =
                    &source[(x0 + leading as i32) as usize..(x0 + middle_end as i32) as usize];
                for (out, sample) in output[leading..middle_end].iter_mut().zip(middle) {
                    *out = finish.one(i32::from(sample.to_u16()) << shift_up);
                }
            }
            let last = i32::from(source[last_x as usize].to_u16()) << shift_up;
            for out in &mut output[middle_end..] {
                *out = finish.one(last);
            }
        }
    }
}

pub(super) fn subpel_copy_block_u16_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    output_stride: usize,
) -> Result<()> {
    const LANES: usize = 8;
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    let max_sample = params.bit_depth.max_sample();
    let limit = Simd::<u16, LANES>::splat(max_sample);
    for r in 0..params.h {
        let row = (y0 + r as i32).clamp(params.first_y, params.last_y) as usize;
        let output = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let row = row.min(reference.readable_rows - 1);
            let start = row * reference.stride + x;
            let source = &reference.samples[start..start + params.w];
            if let Some(source) = T::u16_slice(source) {
                let mut chunks = source.chunks_exact(LANES);
                for (output, source) in output.chunks_exact_mut(LANES).zip(&mut chunks) {
                    output.copy_from_slice(&Simd::from_slice(source).simd_min(limit).to_array()); // splot-copy-ok: publish SIMD prediction lanes into caller output
                }
                let copied = source.len() - chunks.remainder().len();
                for (output, &source) in output[copied..].iter_mut().zip(chunks.remainder()) {
                    *output = source.min(max_sample);
                }
            } else {
                for (output, source) in output.iter_mut().zip(source) {
                    *output = source.to_u16();
                }
            }
        } else {
            let row = row.min(reference.readable_rows - 1);
            if let Some(source) = T::u16_slice(reference.row(row)) {
                let first_x = params.first_x.clamp(0, reference.width as i32 - 1);
                let last_x = params.last_x.clamp(0, reference.width as i32 - 1);
                let leading =
                    (i64::from(first_x) - i64::from(x0)).clamp(0, params.w as i64) as usize;
                let middle_end =
                    (i64::from(last_x) - i64::from(x0) + 1).clamp(0, params.w as i64) as usize;
                output[..leading].fill(source[first_x as usize].min(max_sample));
                if leading < middle_end {
                    let middle =
                        &source[(x0 + leading as i32) as usize..(x0 + middle_end as i32) as usize];
                    let mut chunks = middle.chunks_exact(LANES);
                    for (output, source) in output[leading..middle_end]
                        .chunks_exact_mut(LANES)
                        .zip(&mut chunks)
                    {
                        output
                            .copy_from_slice(&Simd::from_slice(source).simd_min(limit).to_array()); // splot-copy-ok: publish SIMD prediction lanes into caller output
                    }
                    let copied = middle.len() - chunks.remainder().len();
                    for (output, &source) in output[leading + copied..middle_end]
                        .iter_mut()
                        .zip(chunks.remainder())
                    {
                        *output = source.min(max_sample);
                    }
                }
                output[middle_end..].fill(source[last_x as usize].min(max_sample));
            } else {
                for (c, output) in output.iter_mut().enumerate() {
                    let col = (x0 + c as i32).clamp(params.first_x, params.last_x) as usize;
                    *output = (reference.sample(row, col) as u16).min(max_sample);
                }
            }
        }
    }
    Ok(())
}

pub(super) fn subpel_copy_compound_average_u16_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    pred0: &[i32],
    cwp_weight: i16,
    output: &mut [u16],
    output_stride: usize,
) -> Result<bool> {
    const LANES: usize = 4;
    if params.step_x != 1 << SCALE_SUBPEL_BITS
        || params.step_y != 1 << SCALE_SUBPEL_BITS
        || (params.start_x >> 6) & SUBPEL_MASK != 0
        || (params.start_y >> 6) & SUBPEL_MASK != 0
        || T::u16_slice(reference.samples).is_none()
    {
        return Ok(false);
    }
    let Some(x) = subpel_direct_copy_x(reference, params) else {
        return Ok(false);
    };
    if output_stride < params.w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: output_stride,
            storage_width: params.w,
        });
    }
    let output_len = (params.h - 1)
        .checked_mul(output_stride)
        .and_then(|len| len.checked_add(params.w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided compound prediction sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let max_sample = i32::from(params.bit_depth.max_sample());
    for row in 0..params.h {
        let source_row = (y0 + row as i32).clamp(params.first_y, params.last_y) as usize;
        let source = &reference.row(source_row)[x..x + params.w];
        let source = T::u16_slice(source).ok_or(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: core::any::type_name::<T>(),
            bit_depth: params.bit_depth,
        })?;
        let pred0 = &pred0[row * params.w..][..params.w];
        let output = &mut output[row * output_stride..][..params.w];
        let mut source_chunks = source.chunks_exact(LANES);
        for ((output, pred0), source) in output
            .chunks_exact_mut(LANES)
            .zip(pred0.chunks_exact(LANES))
            .zip(&mut source_chunks)
        {
            let pred0 = Simd::<i32, LANES>::from_slice(pred0);
            let pred1 = Simd::<u16, LANES>::from_slice(source).cast::<i32>() << 4;
            let blended = round2_simd(
                pred0 * Simd::splat(forward) + pred1 * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_max(Simd::splat(0))
            .simd_min(Simd::splat(max_sample))
            .cast::<u16>();
            output.copy_from_slice(&blended.to_array()); // splot-copy-ok: publish SIMD blend lanes into caller output
        }
        let copied = source.len() - source_chunks.remainder().len();
        for ((output, &pred0), &source) in output[copied..]
            .iter_mut()
            .zip(&pred0[copied..])
            .zip(source_chunks.remainder())
        {
            *output = blend_compound_average_weighted_sample(
                pred0,
                i32::from(source) << 4,
                params.bit_depth,
                cwp_weight,
            );
        }
    }
    Ok(true)
}

pub(super) fn subpel_direct_copy_x<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<usize> {
    usize::try_from(params.start_x >> SCALE_SUBPEL_BITS)
        .ok()
        .filter(|&x| {
            x >= usize::try_from(params.first_x.max(0)).unwrap_or(usize::MAX)
                && x.checked_add(params.w).is_some_and(|end| {
                    end <= reference.width
                        && i32::try_from(end - 1).is_ok_and(|last| last <= params.last_x)
                })
        })
}

pub(super) fn subpel_horizontal_window_x<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<usize> {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    (x0 - 3 >= params.first_x.max(0)
        && x0 + params.w as i32 + 3 <= params.last_x.min(reference.width as i32 - 1))
    .then(|| (x0 - 3) as usize)
}

pub(super) fn subpel_horizontal_only_into<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let h_filter = params.interp.pass_index(params.w as u32) as usize;
    let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
    let full_taps = &SUBPEL_FILTERS[h_filter][phase];
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[h_filter][phase];
    let taps = &full_taps[tap_start..tap_end];
    let full_span = tap_start == 0 && tap_end == NUM_TAPS;
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let x_window_start = subpel_horizontal_window_x(reference, params);

    for r in 0..params.h {
        let ref_row = ((params.start_y >> SCALE_SUBPEL_BITS) + r as i32)
            .clamp(params.first_y, params.last_y) as usize;
        let ref_row = ref_row.min(reference.readable_rows - 1);
        let row_out = &mut output[r * output_stride..][..params.w];
        if let Some(window_start) = x_window_start {
            let row_base = ref_row * reference.stride + window_start;
            let taps_end = row_base + params.w + NUM_TAPS - 1;
            let window = reference
                .samples
                .get(row_base..taps_end + SLIDE_RESERVE)
                .unwrap_or(&reference.samples[row_base..taps_end]);
            if let Some(window) = T::u16_slice(window) {
                let available = window.len();
                let vector_width8 = params.w - params.w % 8;
                for c in (0..vector_width8).step_by(8) {
                    let sum = if full_span && Simd::<i32, 8>::admits(available, c) {
                        Simd::<i32, 8>::slid_tap_sum(window, c, full_taps)
                    } else {
                        let mut sum = Simd::<i32, 8>::splat(0);
                        for (tap_offset, &tap) in taps.iter().enumerate() {
                            sum = tap_mac(
                                sum,
                                Simd::<u16, 8>::from_slice(&window[c + tap_start + tap_offset..])
                                    .cast(),
                                tap,
                            );
                        }
                        sum
                    };
                    let values = round2_simd(
                        round2_simd(sum, INTER_ROUND0) << FILTER_BITS as i32,
                        inter_round1,
                    );
                    finish.eight(values, &mut row_out[c..c + 8]);
                }
                let vector_width4 = params.w - params.w % 4;
                for c in (vector_width8..vector_width4).step_by(4) {
                    let sum = if full_span && Simd::<i32, 4>::admits(available, c) {
                        Simd::<i32, 4>::slid_tap_sum(window, c, full_taps)
                    } else {
                        let mut sum = Simd::<i32, 4>::splat(0);
                        for (tap_offset, &tap) in taps.iter().enumerate() {
                            sum = tap_mac(
                                sum,
                                Simd::<u16, 4>::from_slice(&window[c + tap_start + tap_offset..])
                                    .cast(),
                                tap,
                            );
                        }
                        sum
                    };
                    let values = round2_simd(
                        round2_simd(sum, INTER_ROUND0) << FILTER_BITS as i32,
                        inter_round1,
                    );
                    finish.four(values, &mut row_out[c..c + 4]);
                }
                for c in vector_width4..params.w {
                    let mut sum = 0i32;
                    for (tap_offset, &tap) in taps.iter().enumerate() {
                        sum += tap * i32::from(window[c + tap_start + tap_offset]);
                    }
                    let horizontal = round2_i32(sum, INTER_ROUND0);
                    row_out[c] = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
                }
                continue;
            }
            for (out, win) in row_out.iter_mut().zip(window.windows(NUM_TAPS)) {
                let samples = &win[tap_start..tap_start + taps.len()];
                let mut sum = 0i32;
                for (&tap, &sample) in taps.iter().zip(samples) {
                    sum += tap * i32::from(sample.to_u16());
                }
                let horizontal = round2_i32(sum, INTER_ROUND0);
                *out = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
            }
            continue;
        }
        if clipped_edges::horizontal_only(
            reference,
            params,
            ref_row,
            taps,
            tap_start,
            tap_end,
            inter_round1,
            row_out,
            finish,
        ) {
            continue;
        }
        for (c, out) in row_out.iter_mut().enumerate() {
            let mut sum = 0i32;
            for (tap_offset, &tap) in taps.iter().enumerate() {
                let t = tap_start + tap_offset;
                let ref_col =
                    (x0 + c as i32 + t as i32 - 3).clamp(params.first_x, params.last_x) as usize;
                sum += tap * reference.sample(ref_row, ref_col);
            }
            let horizontal = round2_i32(sum, INTER_ROUND0);
            *out = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
        }
    }
}

/// Reports the first source row of an unscaled vertical-pass block whose whole
/// `h + NUM_TAPS - 1` tap-row window already lies inside `[firstY, lastY]` and
/// inside the plane, so every § 7.13.3.18 `Clip3` and the plane clamp are the
/// identity and the tap rows sit exactly `stride` apart.
fn vertical_interior_top<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    y0: i32,
) -> Option<usize> {
    let top = usize::try_from(i64::from(y0) - 3).ok()?;
    let bottom = top + params.h + NUM_TAPS - 2;
    (i64::from(params.first_y) <= top as i64
        && bottom as i64 <= i64::from(params.last_y)
        && bottom < reference.readable_rows)
        .then_some(top)
}

/// Runs the § 7.13.3.18 vertical pass over a block whose tap rows need no
/// clipping.
///
/// The eight taps run over the whole filter row instead of its active span: a
/// zero tap contributes exactly zero to the integer accumulator, so the sum and
/// the order of its non-zero terms are those of the clipped path, while the
/// constant trip count keeps the tap coefficients and the eight row bases in
/// registers across the column loop.
#[allow(clippy::too_many_arguments)]
fn subpel_vertical_interior_into<O>(
    source: &[u16],
    stride: usize,
    params: &SubpelPredictParams,
    taps: &[i32; NUM_TAPS],
    top: usize,
    x: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let vector_width8 = params.w - params.w % 8;
    let vector_width4 = params.w - params.w % 4;
    for r in 0..params.h {
        let base = (top + r) * stride + x;
        let rows: [&[u16]; NUM_TAPS] =
            core::array::from_fn(|t| &source[base + t * stride..][..params.w]);
        let row_out = &mut output[r * output_stride..][..params.w];
        for c in (0..vector_width8).step_by(8) {
            let mut sum = Simd::<i32, 8>::splat(0);
            for t in 0..NUM_TAPS {
                sum = tap_mac(
                    sum,
                    Simd::<u16, 8>::from_slice(&rows[t][c..]).cast(),
                    taps[t],
                );
            }
            let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
            finish.eight(values, &mut row_out[c..c + 8]);
        }
        for c in (vector_width8..vector_width4).step_by(4) {
            let mut sum = Simd::<i32, 4>::splat(0);
            for t in 0..NUM_TAPS {
                sum = tap_mac(
                    sum,
                    Simd::<u16, 4>::from_slice(&rows[t][c..]).cast(),
                    taps[t],
                );
            }
            let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
            finish.four(values, &mut row_out[c..c + 4]);
        }
        for c in vector_width4..params.w {
            let mut sum = 0i32;
            for t in 0..NUM_TAPS {
                sum += taps[t] * i32::from(rows[t][c]);
            }
            row_out[c] = finish.one(round2_i32(
                sum << (FILTER_BITS - INTER_ROUND0),
                inter_round1,
            ));
        }
    }
}

pub(super) fn subpel_vertical_only_into<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    filter_height: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let v_filter = params.interp.pass_index(filter_height as u32) as usize;
    let phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
    let full_taps = &SUBPEL_FILTERS[v_filter][phase];
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[v_filter][phase];
    let taps = &full_taps[tap_start..tap_end];
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);

    if let (Some(x), Some(source), Some(top)) = (
        direct_x,
        T::u16_slice(reference.samples),
        vertical_interior_top(reference, params, y0),
    ) {
        subpel_vertical_interior_into(
            source,
            reference.stride,
            params,
            full_taps,
            top,
            x,
            inter_round1,
            output,
            output_stride,
            finish,
        );
        return;
    }

    let mut acc = [0i32; MAX_BLOCK_DIM];

    for r in 0..params.h {
        if let (Some(x), Some(source)) = (direct_x, T::u16_slice(reference.samples)) {
            let vector_width8 = params.w - params.w % 8;
            for c in (0..vector_width8).step_by(8) {
                let mut sum = Simd::<i32, 8>::splat(0);
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    let ref_row = (y0 + r as i32 + t as i32 - 3)
                        .clamp(params.first_y, params.last_y)
                        as usize;
                    let ref_row = ref_row.min(reference.readable_rows - 1);
                    let start = ref_row * reference.stride + x + c;
                    sum = tap_mac(
                        sum,
                        Simd::<u16, 8>::from_slice(&source[start..]).cast(),
                        tap,
                    );
                }
                let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
                finish.eight(values, &mut output[r * output_stride + c..][..8]);
            }
            let vector_width4 = params.w - params.w % 4;
            for c in (vector_width8..vector_width4).step_by(4) {
                let mut sum = Simd::<i32, 4>::splat(0);
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    let ref_row = (y0 + r as i32 + t as i32 - 3)
                        .clamp(params.first_y, params.last_y)
                        as usize;
                    let ref_row = ref_row.min(reference.readable_rows - 1);
                    let start = ref_row * reference.stride + x + c;
                    sum = tap_mac(
                        sum,
                        Simd::<u16, 4>::from_slice(&source[start..]).cast(),
                        tap,
                    );
                }
                let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
                finish.four(values, &mut output[r * output_stride + c..][..4]);
            }
            for c in vector_width4..params.w {
                let mut sum = 0i32;
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    let ref_row = (y0 + r as i32 + t as i32 - 3)
                        .clamp(params.first_y, params.last_y)
                        as usize;
                    let ref_row = ref_row.min(reference.readable_rows - 1);
                    sum += tap * reference.sample(ref_row, x + c);
                }
                output[r * output_stride + c] = finish.one(round2_i32(
                    sum << (FILTER_BITS - INTER_ROUND0),
                    inter_round1,
                ));
            }
            continue;
        }
        if clipped_edges::vertical_only(
            reference,
            params,
            r,
            taps,
            tap_start,
            inter_round1,
            output,
            output_stride,
            finish,
        ) {
            continue;
        }
        let acc = &mut acc[..params.w];
        acc.fill(0);
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_row =
                (y0 + r as i32 + t as i32 - 3).clamp(params.first_y, params.last_y) as usize;
            let ref_row = ref_row.min(reference.readable_rows - 1);
            if let Some(x) = direct_x {
                let start = ref_row * reference.stride + x;
                for (sum, sample) in acc
                    .iter_mut()
                    .zip(&reference.samples[start..start + params.w])
                {
                    *sum += tap * i32::from(sample.to_u16());
                }
            } else {
                for (c, sum) in acc.iter_mut().enumerate() {
                    let ref_col = (x0 + c as i32).clamp(params.first_x, params.last_x) as usize;
                    *sum += tap * reference.sample(ref_row, ref_col);
                }
            }
        }
        let row_out = &mut output[r * output_stride..][..params.w];
        for (out, &sum) in row_out.iter_mut().zip(acc.iter()) {
            *out = finish.one(round2_i32(
                sum << (FILTER_BITS - INTER_ROUND0),
                inter_round1,
            ));
        }
    }
}
