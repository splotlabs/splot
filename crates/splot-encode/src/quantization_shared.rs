// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared encoder quantizer helpers for the 4x4 ([`crate::quantization`]) and
//! 16x16 ([`crate::quantization_16x16`]) per-coefficient quantizers.
//!
//! This is encoder-policy arithmetic only; it emits no AV2 syntax.

use splot_recon::{
    BitDepth as ReconBitDepth, DequantBlockParams, PlaneId, PlaneRect, QuantizerDeltas,
    dequantize_block,
};

use crate::error::{Error, Result};

pub(crate) const DEQUANT_PRODUCT_MAX: u64 = 0xFF_FFFF;

const DEQUANT_ROUNDING_SCALE: u128 = 8;

/// Inclusive `(min, max)` range a pre-quantization coefficient may occupy for the
/// given reconstruction bit depth, matching the decoder's dequant visible range.
pub(crate) fn dequant_visible_range(bit_depth: ReconBitDepth) -> (i32, i32) {
    let bound = 1i32 << (7 + u32::from(bit_depth.bits()));
    (-bound, bound - 1)
}

/// All-zero [`QuantizerDeltas`] used when the encoder applies no per-plane DC/AC
/// quantizer offsets.
pub(crate) const fn zero_deltas() -> QuantizerDeltas {
    QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    }
}

pub(crate) fn validate_quantization_shape(
    plane: PlaneId,
    block: PlaneRect,
    expected_width: usize,
    expected_height: usize,
) -> Result<()> {
    if block.width() == expected_width && block.height() == expected_height {
        Ok(())
    } else {
        Err(Error::QuantizationUnsupportedShape {
            plane,
            block,
            expected_width,
            expected_height,
        })
    }
}

pub(crate) fn quantize_coefficients<const N: usize>(
    plane: PlaneId,
    block: PlaneRect,
    coefficients: &[i32; N],
    dc_quantizer: u32,
    ac_quantizer: u32,
    bit_depth: ReconBitDepth,
    dq_denom: u32,
) -> Result<[i32; N]> {
    let mut quantized = [0; N];
    for (index, (&coefficient, out)) in coefficients.iter().zip(quantized.iter_mut()).enumerate() {
        let quantizer = if index == 0 {
            dc_quantizer
        } else {
            ac_quantizer
        };
        *out = quantize_coefficient(
            plane,
            block,
            index,
            coefficient,
            quantizer,
            bit_depth,
            dq_denom,
        )?;
    }
    Ok(quantized)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dequantize_coefficients<const N: usize>(
    plane: PlaneId,
    block: PlaneRect,
    quantized: &[i32; N],
    dc_quantizer: u32,
    ac_quantizer: u32,
    tx_width: usize,
    tx_height: usize,
    bit_depth: ReconBitDepth,
    dq_denom: u32,
) -> Result<[i32; N]> {
    let mut dequantized = [0; N];
    let dequant_params = DequantBlockParams {
        dc_quant: dc_quantizer,
        ac_quant: ac_quantizer,
        tx_width,
        tx_height,
        dq_denom,
        bit_depth,
    };
    dequantize_block(&dequant_params, quantized, &mut dequantized).map_err(|source| {
        Error::QuantizationDequant {
            plane,
            block,
            source,
        }
    })?;
    Ok(dequantized)
}

#[allow(clippy::too_many_arguments)]
fn quantize_coefficient(
    plane: PlaneId,
    block: PlaneRect,
    index: usize,
    coefficient: i32,
    quantizer: u32,
    bit_depth: ReconBitDepth,
    dq_denom: u32,
) -> Result<i32> {
    let (min, max) = dequant_visible_range(bit_depth);
    if coefficient < min || coefficient > max {
        return Err(Error::QuantizationCoefficientOutOfRange {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            min,
            max,
            bit_depth,
        });
    }
    if coefficient == 0 {
        return Ok(0);
    }

    let magnitude = u128::from(coefficient.unsigned_abs());
    let numerator = magnitude
        .checked_mul(u128::from(dq_denom))
        .and_then(|value| value.checked_mul(DEQUANT_ROUNDING_SCALE))
        .ok_or(Error::QuantizationCoefficientOverflow {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            quantizer,
            dq_denom,
            context: "quantization numerator",
        })?;
    let divisor = u128::from(quantizer);
    let rounded =
        numerator
            .checked_add(divisor / 2)
            .ok_or(Error::QuantizationCoefficientOverflow {
                plane,
                block,
                coefficient_index: index,
                value: coefficient,
                quantizer,
                dq_denom,
                context: "quantization rounding",
            })?
            / divisor;
    if rounded > i32::MAX as u128 {
        return Err(Error::QuantizationCoefficientOverflow {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            quantizer,
            dq_denom,
            context: "quantized coefficient",
        });
    }

    let product = rounded
        .checked_mul(divisor)
        .ok_or(Error::QuantizationCoefficientOverflow {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            quantizer,
            dq_denom,
            context: "dequant product",
        })?;
    if product > u128::from(DEQUANT_PRODUCT_MAX) {
        return Err(Error::QuantizationDequantProductOverflow {
            plane,
            block,
            coefficient_index: index,
            quantized_abs: rounded as u64,
            quantizer,
            max_product: DEQUANT_PRODUCT_MAX,
        });
    }

    let quantized_abs = rounded as i32;
    if coefficient < 0 {
        Ok(-quantized_abs)
    } else {
        Ok(quantized_abs)
    }
}
