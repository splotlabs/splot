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
    if let Some(view) = reference.as_u8()
        && block::put_u8(&view, params, output, output_stride)
    {
        return Ok(());
    }
    validate_subpel_u8_output(params, output, output_stride)?;
    let sink = Sink::Put {
        output,
        stride: output_stride,
    };
    block::predict(reference, params, sink, None);
    Ok(())
}

/// [`subpel_predict_block_strided_into_u8`] of a block read in place from
/// eight-bit storage, returning `false` without writing any other block.
#[inline]
pub fn subpel_put_u8_in_place<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
    output_stride: usize,
) -> bool {
    reference
        .as_u8()
        .is_some_and(|view| block::put_u8(&view, params, output, output_stride))
}

/// [`subpel_put_u8_in_place`] of the two chroma planes of one block, which
/// share `params`, into their outputs: `false` without writing either when
/// the planes differ in geometry or the first would decline.
#[inline]
pub fn subpel_put_u8_pair_in_place<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: &SubpelPredictParams,
    outputs: [(&mut [u8], usize); 2],
) -> bool {
    let [Some(first), Some(second)] = references.map(ReferencePlaneView::as_u8) else {
        return false;
    };
    block::put_u8_planes([&first, &second], params, outputs)
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
    crate::intra_dc_math::validate_sample_type::<u8>(params0.bit_depth)?;
    if !subpel_params_are_valid_fullpel(params0) || !subpel_params_are_valid_fullpel(params1) {
        validate_subpel_params(params0)?;
        validate_subpel_params(params1)?;
        return Ok(false);
    }
    if params0.w != params1.w || params0.h != params1.h || params0.bit_depth != params1.bit_depth {
        return Ok(false);
    }
    validate_compound_output(params0, output, output_stride)?;
    Ok(compound_average(
        [reference0, reference1],
        [params0, params1],
        cwp_weight,
        &mut [],
        output,
        output_stride,
    ))
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
