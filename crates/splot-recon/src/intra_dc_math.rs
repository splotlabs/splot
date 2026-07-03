// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared DC intra prediction math.

use crate::intra::{IntraDcEdge, IntraRectBlockSize};
use crate::{BitDepth, ReconError, ReconSample, Result};

pub(crate) const DIV_LUT_BITS: u8 = 7;
const DIV_LUT_PREC_BITS: u8 = 9;
#[rustfmt::skip]
const DIV_LUT: [u16; 129] = [
    512, 508, 504, 500, 496, 493, 489, 485, 482, 478, 475, 471, 468, 465, 462, 458, 455, 452, 449, 446, 443, 440, 437, 434, 431, 428, 426, 423, 420, 417, 415, 412,
    410, 407, 405, 402, 400, 397, 395, 392, 390, 388, 386, 383, 381, 379, 377, 374, 372, 370, 368, 366, 364, 362, 360, 358, 356, 354, 352, 350, 349, 347, 345, 343,
    341, 340, 338, 336, 334, 333, 331, 329, 328, 326, 324, 323, 321, 320, 318, 317, 315, 314, 312, 311, 309, 308, 306, 305, 303, 302, 301, 299, 298, 297, 295, 294,
    293, 291, 290, 289, 287, 286, 285, 284, 282, 281, 280, 279, 278, 277, 275, 274, 273, 272, 271, 270, 269, 267, 266, 265, 264, 263, 262, 261, 260, 259, 258, 257, 256,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DcEdgeSum {
    pub(crate) sum: u64,
    pub(crate) count: u64,
}

pub(crate) fn predict_intra_dc_rect_value_from_sums<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    left_sum: Option<u64>,
    above_sum: Option<u64>,
) -> Result<T> {
    validate_sample_type::<T>(bit_depth)?;
    let predicted = match (left_sum, above_sum) {
        (Some(left), Some(above)) => clip1(
            approx_divide(left + above, (size.width() + size.height()) as u64)?,
            bit_depth,
        ),
        (Some(left), None) => round2(left, size.log2_height()),
        (None, Some(above)) => round2(above, size.log2_width()),
        (None, None) => dc_midpoint(bit_depth),
    };

    T::try_from_u16(predicted)
}

pub(crate) fn validate_sample_type<T: ReconSample>(bit_depth: BitDepth) -> Result<()> {
    if T::supports_bit_depth(bit_depth) {
        Ok(())
    } else {
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth,
        })
    }
}

pub(crate) fn validate_dc_edge<T: ReconSample>(
    edge: IntraDcEdge,
    samples: Option<&[T]>,
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<Option<u64>> {
    Ok(
        validate_dc_edge_sampled_sum(edge, samples, expected_len, 1, bit_depth)?
            .map(|sampled| sampled.sum),
    )
}

pub(crate) fn validate_dc_edge_sampled_sum<T: ReconSample>(
    edge: IntraDcEdge,
    samples: Option<&[T]>,
    expected_len: usize,
    step: usize,
    bit_depth: BitDepth,
) -> Result<Option<DcEdgeSum>> {
    if step == 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra DC edge sampling step",
        });
    }

    let Some(samples) = samples else {
        return Ok(None);
    };

    if samples.len() != expected_len {
        return Err(ReconError::IntraPredictionEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    let max = bit_depth.max_sample();
    let mut sum = 0u64;
    let mut count = 0u64;
    for (sample_index, sample) in samples.iter().enumerate() {
        let value = sample.to_u16();
        if value > max {
            return Err(ReconError::IntraPredictionSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            });
        }
        if sample_index % step == 0 {
            sum = sum
                .checked_add(u64::from(value))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "intra DC edge sample sum",
                })?;
            count = count.checked_add(1).ok_or(ReconError::ArithmeticOverflow {
                context: "intra DC edge sample count",
            })?;
        }
    }
    Ok(Some(DcEdgeSum { sum, count }))
}

/// Validates intra prediction output buffer geometry, returning the required
/// buffer length. `context` labels the arithmetic-overflow error so each
/// predictor reports its own buffer-length context.
pub(crate) fn validate_output_shape(
    size: IntraRectBlockSize,
    output_len: usize,
    stride_samples: usize,
    context: &'static str,
) -> Result<usize> {
    let width = size.width();
    if stride_samples < width {
        return Err(ReconError::IntraPredictionStrideTooSmall {
            stride_samples,
            width,
        });
    }

    let required = (size.height() - 1)
        .checked_mul(stride_samples)
        .and_then(|prefix| prefix.checked_add(width))
        .ok_or(ReconError::ArithmeticOverflow { context })?;
    if output_len < required {
        return Err(ReconError::IntraPredictionOutputTooSmall {
            expected: required,
            actual: output_len,
        });
    }
    Ok(required)
}

pub(crate) fn fill_validated_output_shape<T: ReconSample>(
    size: IntraRectBlockSize,
    output: &mut [T],
    stride_samples: usize,
    required: usize,
    sample: T,
) {
    for row_index in 0..size.height() {
        let row_start = row_index * stride_samples;
        let row_end = row_start + size.width();
        output[row_start..row_end].fill(sample);
    }
    debug_assert!(required <= output.len());
}

pub(crate) fn approx_divide(num: u64, den: u64) -> Result<u16> {
    if den == 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra DC approximate divisor",
        });
    }
    let (shift, scale) = resolve_divisor(den)?;
    let scaled = num
        .checked_mul(u64::from(scale))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "intra DC approximate division product",
        })?;
    Ok(round2(scaled, shift))
}

pub(crate) fn resolve_division(num: i64, den: i64, shift: u8) -> i16 {
    if num == 0 || den <= 0 {
        return 0;
    }
    let sign_negative = num < 0;
    let n_abs = num.unsigned_abs();
    let d = den as u64;
    let shift_n = floor_log2(n_abs);
    let shift_d = floor_log2(d);
    let e_d = d - (1u64 << shift_d);
    let f_d = if shift_d > DIV_LUT_BITS {
        round2_u64(e_d, shift_d - DIV_LUT_BITS) as usize
    } else {
        (e_d << (DIV_LUT_BITS - shift_d)) as usize
    };
    let f_n = if shift_n > DIV_LUT_BITS {
        round2_u64(n_abs, shift_n - DIV_LUT_BITS)
    } else {
        n_abs << (DIV_LUT_BITS - shift_n)
    };
    let shift_add = i32::from(shift_d) - i32::from(shift_n) - i32::from(shift);
    let max = (2i64 << shift) - 1;
    let mut ret = if shift_add <= 1 {
        let shift0 = i32::from(DIV_LUT_PREC_BITS) + i32::from(DIV_LUT_BITS) + shift_add;
        if shift0 >= 0 {
            let scale = i64::from(DIV_LUT.get(f_d).copied().unwrap_or(0));
            (scale * i64::try_from(f_n).unwrap_or(i64::MAX)) >> shift0
        } else {
            max
        }
    } else {
        0
    };
    ret = ret.min(max);
    if sign_negative {
        ret = -ret;
    }
    ret as i16
}

/// AV2 §7.13.2.9 / §7.13.2.12 `resolve_divisor(D)`: decomposes `D` so that
/// `1/D ≈ scale / 2^shift`, returning `(shift, scale)` with `scale` at
/// `DIV_LUT_PREC_BITS` precision. Matches AVM `resolve_divisor_32`
/// (`warped_motion.h`): `shift = get_msb(D)`, the lookup index `f` is the top
/// `DIV_LUT_BITS` bits of `D` after resetting its MSB, and the returned shift is
/// `get_msb(D) + DIV_LUT_PREC_BITS`. Shared by the IBP DC modifier, the
/// §7.13.2.9 IBP angular weights process, and the §7.13.3.22 warp division.
///
/// # Errors
/// Returns [`ReconError::ArithmeticOverflow`] for a zero divisor or an
/// out-of-table lookup index.
pub fn resolve_divisor(den: u64) -> Result<(u8, u16)> {
    if den == 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra DC divisor resolution",
        });
    }

    let n = floor_log2(den);
    let e = den - (1u64 << n);
    let f = if n > DIV_LUT_BITS {
        round2_u64(e, n - DIV_LUT_BITS) as usize
    } else {
        (e << (DIV_LUT_BITS - n)) as usize
    };
    let scale = DIV_LUT
        .get(f)
        .copied()
        .ok_or(ReconError::ArithmeticOverflow {
            context: "intra DC divisor lookup",
        })?;
    Ok((n + DIV_LUT_PREC_BITS, scale))
}

fn floor_log2(value: u64) -> u8 {
    (u64::BITS - 1 - value.leading_zeros()) as u8
}

pub(crate) fn round2(value: u64, shift: u8) -> u16 {
    round2_u64(value, shift) as u16
}

fn round2_u64(value: u64, shift: u8) -> u64 {
    if shift == 0 {
        return value;
    }
    let rounding = 1u64 << (shift - 1);
    (value + rounding) >> shift
}

pub(crate) fn clip1(value: u16, bit_depth: BitDepth) -> u16 {
    value.min(bit_depth.max_sample())
}

pub(crate) fn dc_midpoint(bit_depth: BitDepth) -> u16 {
    1u16 << (bit_depth.bits() - 1)
}
