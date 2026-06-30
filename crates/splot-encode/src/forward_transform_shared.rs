// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared encoder forward-transform arithmetic.

use splot_recon::{PlaneId, PlaneRect};

use crate::error::{Error, Result};

pub(crate) fn validate_forward_shape(
    plane: PlaneId,
    block: PlaneRect,
    expected_width: usize,
    expected_height: usize,
) -> Result<()> {
    if block.width() == expected_width && block.height() == expected_height {
        Ok(())
    } else {
        Err(Error::ForwardTransformUnsupportedShape {
            plane,
            block,
            expected_width,
            expected_height,
        })
    }
}

pub(crate) fn validate_forward_input_length(
    plane: PlaneId,
    block: PlaneRect,
    expected: usize,
    actual: usize,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::ForwardTransformInputLengthMismatch {
            plane,
            block,
            expected,
            actual,
        })
    }
}

pub(crate) fn forward_dct_dct_square<const N: usize, const COEFF_COUNT: usize>(
    plane: PlaneId,
    block: PlaneRect,
    residual: &[i32],
    kernel: &[[i32; N]; N],
    row_shift: u32,
    col_shift: u32,
) -> Result<[i32; COEFF_COUNT]> {
    validate_forward_shape(plane, block, N, N)?;
    let expected_len = N
        .checked_mul(N)
        .ok_or(Error::ForwardTransformInputLengthMismatch {
            plane,
            block,
            expected: usize::MAX,
            actual: residual.len(),
        })?;
    validate_forward_input_length(plane, block, expected_len, residual.len())?;
    validate_forward_input_length(plane, block, expected_len, COEFF_COUNT)?;

    let mut intermediate = [0i64; COEFF_COUNT];
    for (r, row_samples) in residual.chunks_exact(N).enumerate() {
        let mut row = [0i64; N];
        for (slot, &sample) in row.iter_mut().zip(row_samples.iter()) {
            *slot = i64::from(sample);
        }
        let transformed = forward_dct_1d(kernel, &row, row_shift);
        for (c, &value) in transformed.iter().enumerate() {
            let index = r * N + c;
            if let Some(slot) = intermediate.get_mut(index) {
                *slot = value;
            }
        }
    }

    let mut coefficients = [0; COEFF_COUNT];
    for c in 0..N {
        let mut column = [0i64; N];
        for (r, slot) in column.iter_mut().enumerate() {
            if let Some(&value) = intermediate.get(r * N + c) {
                *slot = value;
            }
        }
        let transformed = forward_dct_1d(kernel, &column, col_shift);
        for (r, &value) in transformed.iter().enumerate() {
            let index = r * N + c;
            let Some(slot) = coefficients.get_mut(index) else {
                continue;
            };
            *slot = i32::try_from(value).map_err(|_| {
                Error::ForwardTransformCoefficientRangeExceeded {
                    plane,
                    block,
                    index,
                    value,
                }
            })?;
        }
    }

    Ok(coefficients)
}

pub(crate) fn forward_round2(value: i64, shift: u32) -> i64 {
    if shift == 0 {
        return value;
    }
    ((i128::from(value) + (1i128 << (shift - 1))) >> shift) as i64
}

fn forward_dct_1d<const N: usize>(
    kernel: &[[i32; N]; N],
    input: &[i64; N],
    shift: u32,
) -> [i64; N] {
    let mut out = [0i64; N];
    for (r, slot) in out.iter_mut().enumerate() {
        let mut sum = 0i64;
        for (i, &sample) in input.iter().enumerate() {
            sum += i64::from(kernel[r][i]) * sample;
        }
        *slot = forward_round2(sum, shift);
    }
    out
}
