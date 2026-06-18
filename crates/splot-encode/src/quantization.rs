// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder quantization foundation.
//!
//! This module advances `ENC-QUANTIZATION-V0`. It is encoder-policy arithmetic
//! over already-produced transform coefficients. Decoder-visible dequantization
//! remains delegated to `splot-recon`'s AV2 § 7.14.2 / § 7.14.4 implementation
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2` and
//! `#s-7-14-4`).
//!
//! The current subset handles only the existing private 4x4 DCT_DCT DC-only
//! transform block. It does not emit § 5.20.7.28 quantized coefficient syntax,
//! select rate-control values, tokenize coefficients, write tile bodies, or
//! produce [`crate::Packet`] values.

#![allow(dead_code)]

use splot_recon::{
    BitDepth as ReconBitDepth, DequantBlockParams, PlaneId, PlaneRect, QuantizerDeltas,
    ac_quantizer, dc_quantizer, dequantize_block, max_quantizer_index,
};

use crate::error::{Error, Result};
use crate::forward_transform::ForwardTransformBlock;

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const DEQUANT_ROUNDING_SCALE: u128 = 8;
const DEQUANT_PRODUCT_MAX: u64 = 0xFF_FFFF;

/// Fixed quantizer inputs for the current private encoder subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FixedQuantizationParams {
    bit_depth: ReconBitDepth,
    qindex: u32,
    dq_denom: u32,
}

impl FixedQuantizationParams {
    /// Builds fixed quantization parameters with `dq_denom == 1`.
    pub(crate) fn new(bit_depth: ReconBitDepth, qindex: u32) -> Result<Self> {
        Self::with_dequant_denom(bit_depth, qindex, 1)
    }

    /// Builds fixed quantization parameters with a caller-resolved dequant denominator.
    pub(crate) fn with_dequant_denom(
        bit_depth: ReconBitDepth,
        qindex: u32,
        dq_denom: u32,
    ) -> Result<Self> {
        let max = max_quantizer_index(bit_depth);
        if qindex > max {
            return Err(Error::QuantizationQIndexOutOfRange {
                bit_depth,
                qindex,
                max,
            });
        }
        if dq_denom == 0 {
            return Err(Error::QuantizationInvalidDequantDenominator { dq_denom });
        }
        Ok(Self {
            bit_depth,
            qindex,
            dq_denom,
        })
    }

    /// Returns the active decoded bit depth.
    pub(crate) const fn bit_depth(self) -> ReconBitDepth {
        self.bit_depth
    }

    /// Returns the fixed quantizer index.
    pub(crate) const fn qindex(self) -> u32 {
        self.qindex
    }

    /// Returns the AV2 § 7.14.4 dequant denominator.
    pub(crate) const fn dq_denom(self) -> u32 {
        self.dq_denom
    }

    fn dc_quantizer(self, plane: PlaneId) -> u32 {
        dc_quantizer(plane, self.qindex, zero_deltas(), self.bit_depth)
    }

    fn ac_quantizer(self, plane: PlaneId) -> u32 {
        ac_quantizer(plane, self.qindex, zero_deltas(), self.bit_depth)
    }
}

/// Quantized and dequantized coefficient block for the current private subset.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct QuantizedTransformBlock {
    plane: PlaneId,
    block: PlaneRect,
    params: FixedQuantizationParams,
    dc_quantizer: u32,
    ac_quantizer: u32,
    quantized: [i32; DCT_DCT_4X4_COEFF_COUNT],
    dequantized: [i32; DCT_DCT_4X4_COEFF_COUNT],
}

impl QuantizedTransformBlock {
    /// Quantizes the current 4x4 DCT_DCT DC-only transform subset.
    pub(crate) fn dct_dct_4x4_dc_only(
        transformed: &ForwardTransformBlock,
        params: FixedQuantizationParams,
    ) -> Result<Self> {
        let plane = transformed.plane();
        let block = transformed.block();
        validate_4x4_shape(plane, block)?;

        let dc_quantizer = params.dc_quantizer(plane);
        let ac_quantizer = params.ac_quantizer(plane);
        let coefficients = transformed.coefficients();
        let mut quantized = [0; DCT_DCT_4X4_COEFF_COUNT];
        for (index, (&coefficient, out)) in
            coefficients.iter().zip(quantized.iter_mut()).enumerate()
        {
            let quantizer = if index == 0 {
                dc_quantizer
            } else {
                ac_quantizer
            };
            *out = quantize_coefficient(plane, block, index, coefficient, quantizer, params)?;
        }

        let mut dequantized = [0; DCT_DCT_4X4_COEFF_COUNT];
        let dequant_params = DequantBlockParams {
            dc_quant: dc_quantizer,
            ac_quant: ac_quantizer,
            tx_width: DCT_DCT_4X4_WIDTH,
            tx_height: DCT_DCT_4X4_HEIGHT,
            dq_denom: params.dq_denom(),
            bit_depth: params.bit_depth(),
        };
        dequantize_block(&dequant_params, &quantized, &mut dequantized).map_err(|source| {
            Error::QuantizationDequant {
                plane,
                block,
                source,
            }
        })?;

        Ok(Self {
            plane,
            block,
            params,
            dc_quantizer,
            ac_quantizer,
            quantized,
            dequantized,
        })
    }

    /// Returns the source plane identity.
    pub(crate) const fn plane(&self) -> PlaneId {
        self.plane
    }

    /// Returns the visible-plane-relative transform block rectangle.
    pub(crate) const fn block(&self) -> PlaneRect {
        self.block
    }

    /// Returns the fixed quantization parameters used for this block.
    pub(crate) const fn params(&self) -> FixedQuantizationParams {
        self.params
    }

    /// Returns the resolved DC quantizer.
    pub(crate) const fn dc_quantizer(&self) -> u32 {
        self.dc_quantizer
    }

    /// Returns the resolved AC quantizer.
    pub(crate) const fn ac_quantizer(&self) -> u32 {
        self.ac_quantizer
    }

    /// Returns row-major quantized coefficients.
    pub(crate) const fn quantized(&self) -> &[i32; DCT_DCT_4X4_COEFF_COUNT] {
        &self.quantized
    }

    /// Returns row-major dequantized coefficients from `splot-recon`.
    pub(crate) const fn dequantized(&self) -> &[i32; DCT_DCT_4X4_COEFF_COUNT] {
        &self.dequantized
    }
}

fn validate_4x4_shape(plane: PlaneId, block: PlaneRect) -> Result<()> {
    if block.width() == DCT_DCT_4X4_WIDTH && block.height() == DCT_DCT_4X4_HEIGHT {
        Ok(())
    } else {
        Err(Error::QuantizationUnsupportedShape {
            plane,
            block,
            expected_width: DCT_DCT_4X4_WIDTH,
            expected_height: DCT_DCT_4X4_HEIGHT,
        })
    }
}

fn quantize_coefficient(
    plane: PlaneId,
    block: PlaneRect,
    index: usize,
    coefficient: i32,
    quantizer: u32,
    params: FixedQuantizationParams,
) -> Result<i32> {
    let (min, max) = dequant_visible_range(params.bit_depth());
    if coefficient < min || coefficient > max {
        return Err(Error::QuantizationCoefficientOutOfRange {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            min,
            max,
            bit_depth: params.bit_depth(),
        });
    }
    if coefficient == 0 {
        return Ok(0);
    }

    let magnitude = u128::from(coefficient.unsigned_abs());
    let numerator = magnitude
        .checked_mul(u128::from(params.dq_denom()))
        .and_then(|value| value.checked_mul(DEQUANT_ROUNDING_SCALE))
        .ok_or(Error::QuantizationCoefficientOverflow {
            plane,
            block,
            coefficient_index: index,
            value: coefficient,
            quantizer,
            dq_denom: params.dq_denom(),
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
                dq_denom: params.dq_denom(),
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
            dq_denom: params.dq_denom(),
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
            dq_denom: params.dq_denom(),
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

fn dequant_visible_range(bit_depth: ReconBitDepth) -> (i32, i32) {
    let bound = 1i32 << (7 + u32::from(bit_depth.bits()));
    (-bound, bound - 1)
}

const fn zero_deltas() -> QuantizerDeltas {
    QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_recon::{
        InverseTransform1dType, InverseTransform2dDim, InverseTransform2dOuter,
        inverse_transform_2d_outer,
    };

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn uniform(sample: i32) -> [i32; DCT_DCT_4X4_COEFF_COUNT] {
        [sample; DCT_DCT_4X4_COEFF_COUNT]
    }

    fn transform(sample: i32) -> ForwardTransformBlock {
        ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &uniform(sample))
            .unwrap()
    }

    fn params(qindex: u32) -> FixedQuantizationParams {
        FixedQuantizationParams::new(ReconBitDepth::Eight, qindex).unwrap()
    }

    fn quantized(sample: i32, qindex: u32) -> QuantizedTransformBlock {
        QuantizedTransformBlock::dct_dct_4x4_dc_only(&transform(sample), params(qindex)).unwrap()
    }

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    fn inverse_4x4_dct_dct(coefficients: &[i32; DCT_DCT_4X4_COEFF_COUNT]) -> Vec<i32> {
        let params = InverseTransform2dOuter {
            log2_width: 2,
            log2_height: 2,
            lossless: false,
            plane_tx_type_is_idtx: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 7,
            col_shift: 10,
            bit_depth: ReconBitDepth::Eight,
            dpcm: None,
        };
        let mut residual = vec![0; DCT_DCT_4X4_COEFF_COUNT];
        inverse_transform_2d_outer(&params, coefficients, &mut residual).unwrap();
        residual
    }

    #[test]
    fn zero_dc_only_block_quantizes_to_all_zero() {
        let block = quantized(0, 0);

        assert_eq!(block.plane(), PlaneId::Y);
        assert_eq!(block.block(), rect(4, 4));
        assert_eq!(block.params(), params(0));
        assert_eq!(block.dc_quantizer(), 64);
        assert_eq!(block.ac_quantizer(), 64);
        assert_eq!(block.quantized(), &[0; DCT_DCT_4X4_COEFF_COUNT]);
        assert_eq!(block.dequantized(), &[0; DCT_DCT_4X4_COEFF_COUNT]);
    }

    #[test]
    fn positive_dc_only_block_quantizes_at_qindex_zero() {
        let block = quantized(7, 0);
        let mut expected_quant = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected_quant[0] = 28;
        let mut expected_dequant = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected_dequant[0] = 224;

        assert_eq!(block.quantized(), &expected_quant);
        assert_eq!(block.dequantized(), &expected_dequant);
    }

    #[test]
    fn negative_dc_only_block_quantizes_at_qindex_zero() {
        let block = quantized(-8, 0);
        let mut expected_quant = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected_quant[0] = -32;
        let mut expected_dequant = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected_dequant[0] = -256;

        assert_eq!(block.quantized(), &expected_quant);
        assert_eq!(block.dequantized(), &expected_dequant);
    }

    #[test]
    fn nonzero_qindex_rounding_is_deterministic_and_monotonic() {
        let one = quantized(1, 1);
        let two = quantized(2, 1);
        let three = quantized(3, 1);

        assert_eq!(one.dc_quantizer(), 40);
        assert_eq!(one.quantized()[0], 6);
        assert_eq!(one.dequantized()[0], 30);
        assert_eq!(two.quantized()[0], 13);
        assert_eq!(two.dequantized()[0], 65);
        assert_eq!(three.quantized()[0], 19);
        assert_eq!(three.dequantized()[0], 95);
        assert!(one.quantized()[0] < two.quantized()[0]);
        assert!(two.quantized()[0] < three.quantized()[0]);
    }

    #[test]
    fn dequant_and_inverse_reconstruct_qindex_zero_subset() {
        for sample in [-16, -1, 0, 1, 15] {
            let block = quantized(sample, 0);

            assert_eq!(
                inverse_4x4_dct_dct(block.dequantized()),
                uniform(sample),
                "sample {sample}"
            );
        }
    }

    #[test]
    fn rejects_qindex_outside_active_bit_depth() {
        let err = FixedQuantizationParams::new(ReconBitDepth::Eight, 256).unwrap_err();

        assert!(matches!(
            err,
            Error::QuantizationQIndexOutOfRange {
                bit_depth: ReconBitDepth::Eight,
                qindex: 256,
                max: 255,
            }
        ));
    }

    #[test]
    fn rejects_zero_dequant_denominator() {
        let err =
            FixedQuantizationParams::with_dequant_denom(ReconBitDepth::Eight, 0, 0).unwrap_err();

        assert!(matches!(
            err,
            Error::QuantizationInvalidDequantDenominator { dq_denom: 0 }
        ));
    }

    #[test]
    fn rejects_coefficient_outside_dequant_visible_range() {
        let transformed = transform(2000);
        let err =
            QuantizedTransformBlock::dct_dct_4x4_dc_only(&transformed, params(0)).unwrap_err();

        assert!(matches!(
            err,
            Error::QuantizationCoefficientOutOfRange {
                plane: PlaneId::Y,
                coefficient_index: 0,
                value: 64_000,
                min: -32768,
                max: 32767,
                bit_depth: ReconBitDepth::Eight,
                ..
            }
        ));
    }

    #[test]
    fn rejects_quantized_coefficient_overflow() {
        let transformed = transform(1023);
        let params =
            FixedQuantizationParams::with_dequant_denom(ReconBitDepth::Eight, 0, u32::MAX).unwrap();
        let err = QuantizedTransformBlock::dct_dct_4x4_dc_only(&transformed, params).unwrap_err();

        assert!(matches!(
            err,
            Error::QuantizationCoefficientOverflow {
                plane: PlaneId::Y,
                coefficient_index: 0,
                value: 32736,
                quantizer: 64,
                dq_denom: u32::MAX,
                context: "quantized coefficient",
                ..
            }
        ));
    }

    #[test]
    fn rejects_dequant_product_overflow() {
        let transformed = transform(1023);
        let params =
            FixedQuantizationParams::with_dequant_denom(ReconBitDepth::Eight, 0, 1024).unwrap();
        let err = QuantizedTransformBlock::dct_dct_4x4_dc_only(&transformed, params).unwrap_err();

        assert!(matches!(
            err,
            Error::QuantizationDequantProductOverflow {
                plane: PlaneId::Y,
                coefficient_index: 0,
                quantizer: 64,
                max_product: DEQUANT_PRODUCT_MAX,
                ..
            }
        ));
    }
}
