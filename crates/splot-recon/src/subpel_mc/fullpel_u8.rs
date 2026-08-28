// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Writes one eight-bit single-reference prediction into row-strided `u8` storage.
///
/// `output` starts at the prediction's top-left sample. The complete parameter
/// and output geometry is validated before the first sample is written.
///
/// # Errors
/// Returns the same parameter and output-layout errors as
/// [`subpel_predict_block_strided_into`], and rejects non-eight-bit output or
/// inverted reference clipping bounds before writing any sample.
pub fn subpel_predict_block_strided_into_u8<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
    output_stride: usize,
) -> Result<()> {
    let intermediate_height = validate_subpel_u8_output(params, output, output_stride)?;
    let params = SubpelPredictParams {
        first_x: params.first_x.clamp(0, reference.width as i32 - 1),
        first_y: params.first_y.clamp(0, reference.readable_rows as i32 - 1),
        last_x: params.last_x.clamp(0, reference.width as i32 - 1),
        last_y: params.last_y.clamp(0, reference.readable_rows as i32 - 1),
        ..*params
    };
    let params = &params;
    if params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
        && (params.start_x >> 6) & SUBPEL_MASK == 0
        && (params.start_y >> 6) & SUBPEL_MASK == 0
    {
        return subpel_copy_block_u8_into(reference, params, output, output_stride);
    }
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        INTER_ROUND1_NON_COMPOUND,
        intermediate_height,
        None,
        output,
        output_stride,
        ClippedU8SubpelOutput,
    )
}

fn validate_subpel_u8_output(
    params: &SubpelPredictParams,
    output: &[u8],
    output_stride: usize,
) -> Result<usize> {
    crate::intra_dc_math::validate_sample_type::<u8>(params.bit_depth)?;
    let intermediate_height = validate_subpel_params(params)?;
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    if params.first_x > params.last_x || params.first_y > params.last_y {
        return Err(ReconError::SubpelReferenceBoundsInvalid {
            first_x: params.first_x,
            first_y: params.first_y,
            last_x: params.last_x,
            last_y: params.last_y,
        });
    }
    Ok(intermediate_height)
}

fn subpel_copy_block_u8_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
    output_stride: usize,
) -> Result<()> {
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
    for r in 0..params.h {
        let row = (y0 + r as i32)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.readable_rows as i32 - 1) as usize;
        let output = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let source = &reference.samples
                [row * reference.stride + x..row * reference.stride + x + params.w];
            if let Some(source) = T::u8_slice(source) {
                output.copy_from_slice(source); // splot-copy-ok: direct full-pel prediction into canonical u8 storage
            } else {
                for (output, sample) in output.iter_mut().zip(source) {
                    *output = sample.to_u16().min(u16::from(u8::MAX)) as u8;
                }
            }
        } else {
            for (c, output) in output.iter_mut().enumerate() {
                let col = (x0 + c as i32)
                    .clamp(params.first_x, params.last_x)
                    .clamp(0, reference.width as i32 - 1) as usize;
                *output = reference.sample(row, col).min(i32::from(u8::MAX)) as u8;
            }
        }
    }
    Ok(())
}

/// Directly blends two zero-phase unscaled predictors into eight-bit output,
/// returning `Ok(false)` outside the direct source-window subset.
///
/// # Errors
/// Returns validation and output-layout errors.
pub fn subpel_predict_block_compound_average_fullpel_strided_into_u8<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [u8],
    output_stride: usize,
) -> Result<bool> {
    if !subpel_params_are_valid_fullpel(params0) || !subpel_params_are_valid_fullpel(params1) {
        validate_subpel_params(params0)?;
        validate_subpel_params(params1)?;
        return Ok(false);
    }
    if params0.w != params1.w || params0.h != params1.h || params0.bit_depth != params1.bit_depth {
        return Ok(false);
    }
    let (Some(x0), Some(x1)) = (
        subpel_direct_u8_copy_x(reference0, params0),
        subpel_direct_u8_copy_x(reference1, params1),
    ) else {
        return Ok(false);
    };
    validate_compound_output(params0, output, output_stride)?;

    let y0 = [
        params0.start_y >> SCALE_SUBPEL_BITS,
        params1.start_y >> SCALE_SUBPEL_BITS,
    ];
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    for row in 0..params0.h {
        let source_row = [
            (y0[0] + row as i32).clamp(params0.first_y, params0.last_y) as usize,
            (y0[1] + row as i32).clamp(params1.first_y, params1.last_y) as usize,
        ];
        let left = &reference0.row(source_row[0])[x0..x0 + params0.w];
        let right = &reference1.row(source_row[1])[x1..x1 + params0.w];
        let (Some(left), Some(right)) = (T::u8_slice(left), T::u8_slice(right)) else {
            return Ok(false);
        };
        let destination = &mut output[row * output_stride..][..params0.w];
        if forward == 8 {
            for ((slot, &left), &right) in destination.iter_mut().zip(left).zip(right) {
                *slot = ((u16::from(left) + u16::from(right) + 1) >> 1) as u8;
            }
        } else {
            for ((slot, &left), &right) in destination.iter_mut().zip(left).zip(right) {
                *slot = round2_i32(forward * i32::from(left) + backward * i32::from(right), 4)
                    .clamp(0, 255) as u8;
            }
        }
    }
    Ok(true)
}

#[inline]
fn subpel_direct_u8_copy_x<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<usize> {
    let x = usize::try_from(params.start_x >> SCALE_SUBPEL_BITS).ok()?;
    let end = x.checked_add(params.w)?;
    let last = i32::try_from(end.checked_sub(1)?).ok()?;
    (x >= params.first_x.max(0) as usize && end <= reference.width && last <= params.last_x)
        .then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(bit_depth: BitDepth) -> SubpelPredictParams {
        SubpelPredictParams {
            interp: InterpolationFilter::EightTap,
            w: 2,
            h: 2,
            start_x: 1 << SCALE_SUBPEL_BITS,
            start_y: 1 << SCALE_SUBPEL_BITS,
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: 3,
            last_y: 3,
            bit_depth,
        }
    }

    #[test]
    fn single_u8_rejects_bit_depth_and_inverted_bounds_before_mutation() -> Result<()> {
        let ten_bit_samples = [512u16; 16];
        let ten_bit_view = ReferencePlaneView::new(&ten_bit_samples, 4, 4)?;
        let sentinel = 0xa5;
        let mut output = [sentinel; 5];
        assert_eq!(
            subpel_predict_block_strided_into_u8(
                &ten_bit_view,
                &params(BitDepth::Ten),
                &mut output,
                3,
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            })
        );
        assert_eq!(output, [sentinel; 5]);

        let samples = [91u8; 16];
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        for invalid in [
            SubpelPredictParams {
                first_x: 2,
                last_x: 1,
                ..params(BitDepth::Eight)
            },
            SubpelPredictParams {
                first_y: 2,
                last_y: 1,
                ..params(BitDepth::Eight)
            },
        ] {
            assert_eq!(
                subpel_predict_block_strided_into_u8(&view, &invalid, &mut output, 3),
                Err(ReconError::SubpelReferenceBoundsInvalid {
                    first_x: invalid.first_x,
                    first_y: invalid.first_y,
                    last_x: invalid.last_x,
                    last_y: invalid.last_y,
                })
            );
            assert_eq!(output, [sentinel; 5]);
        }
        Ok(())
    }

    #[test]
    fn single_u8_fullpel_clamps_negative_bounds_before_index_conversion() -> Result<()> {
        let samples = (0..16u8).collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        let params = SubpelPredictParams {
            start_x: -2 * (1 << SCALE_SUBPEL_BITS),
            start_y: -2 * (1 << SCALE_SUBPEL_BITS),
            first_x: -2,
            first_y: -2,
            last_x: -1,
            last_y: -1,
            ..params(BitDepth::Eight)
        };
        let mut output = [u8::MAX; 4];

        subpel_predict_block_strided_into_u8(&view, &params, &mut output, params.w)?;

        assert_eq!(output, [samples[0]; 4]);
        Ok(())
    }

    #[test]
    fn single_u8_filtered_clamps_negative_bounds_before_index_conversion() -> Result<()> {
        let samples = (0..16u8).map(|value| value * 13).collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        let negative = SubpelPredictParams {
            interp: InterpolationFilter::EightTapSharp,
            start_x: -2 * (1 << SCALE_SUBPEL_BITS) + 5 * (1 << 6),
            start_y: -2 * (1 << SCALE_SUBPEL_BITS) + 7 * (1 << 6),
            step_x: 896,
            step_y: 1152,
            first_x: -2,
            first_y: -2,
            last_x: -1,
            last_y: -1,
            ..params(BitDepth::Eight)
        };
        let normalized = SubpelPredictParams {
            first_x: 0,
            first_y: 0,
            last_x: 0,
            last_y: 0,
            ..negative
        };
        let mut expected = [u16::MAX; 4];
        let mut actual = [u8::MAX; 4];

        subpel_predict_block_strided_into(&view, &normalized, &mut expected, normalized.w)?;
        subpel_predict_block_strided_into_u8(&view, &negative, &mut actual, negative.w)?;

        assert_eq!(actual.map(u16::from), expected);
        Ok(())
    }
}
