// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

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
