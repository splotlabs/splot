// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder quantization foundation.
//!
//! This module advances `ENC-QUANTIZATION-V0` (the round-to-nearest v0 quantizer)
//! and `ENC-FWD-QUANT-PER-COEFF-AC` (per-coefficient quant over a real 4x4 block).
//! It is encoder-policy arithmetic over already-produced transform coefficients.
//! Decoder-visible dequantization remains delegated to `splot-recon`'s AV2
//! § 7.14.2 / § 7.14.4 implementation
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2` and `#s-7-14-4`).
//!
//! [`QuantizedTransformBlock::dct_dct_4x4`] quantizes all 16 coefficients of a real
//! 4x4 DCT_DCT block per-coefficient (index 0 with the DC quantizer, the rest with
//! the AC quantizer) and dequantizes them through `splot-recon`, so the stored
//! dequantized array is the decoder reconstruction of the emitted levels. It does
//! not emit § 5.20.7.28 quantized coefficient syntax, select rate-control values,
//! tokenize coefficients, write tile bodies, or produce [`crate::Packet`] values.

#![allow(dead_code)]

use splot_recon::{
    BitDepth as ReconBitDepth, DequantBlockParams, PlaneId, PlaneRect, ac_quantizer, dc_quantizer,
    dequantize_block, max_quantizer_index,
};

use crate::error::{Error, Result};
use crate::forward_transform::ForwardTransformBlock;
use crate::quantization_shared::{dequant_visible_range, zero_deltas};

#[allow(unused_imports)]
pub(crate) use crate::quantization_16x16::QuantizedTransformBlock16x16;

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
    /// Quantizes a real 4x4 DCT_DCT transform block: every one of the 16
    /// coefficients is quantized per-coefficient (index 0 with the DC quantizer,
    /// the rest with the AC quantizer) by the round-to-nearest policy, then the
    /// levels are dequantized through `splot-recon`'s AV2 § 7.14 dequantization so
    /// the stored `dequantized` array is exactly what the decoder reconstructs from
    /// the emitted levels. This accepts any [`ForwardTransformBlock`] — a flat
    /// (DC-only) block or one with real non-zero AC content.
    ///
    /// # Errors
    /// Returns the quantization errors of [`quantize_coefficient`] (coefficient out
    /// of the dequant-visible range, arithmetic overflow, or a dequant product
    /// beyond the AV2 24-bit limit), or [`Error::QuantizationDequant`] /
    /// [`Error::QuantizationUnsupportedShape`].
    pub(crate) fn dct_dct_4x4(
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

    /// Quantizes a flat (DC-only) 4x4 DCT_DCT transform block. This is the entry
    /// point the closed loop currently uses for its uniform-residual pairing; the
    /// quantization is identical to [`Self::dct_dct_4x4`] (per-coefficient DC/AC),
    /// so a DC-only forward block quantizes the same through either entry point —
    /// the name documents the input pairing, not a different operation.
    pub(crate) fn dct_dct_4x4_dc_only(
        transformed: &ForwardTransformBlock,
        params: FixedQuantizationParams,
    ) -> Result<Self> {
        Self::dct_dct_4x4(transformed, params)
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::quantization_test_support::expected_level;
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

    fn full_transform(residual: &[i32; DCT_DCT_4X4_COEFF_COUNT]) -> ForwardTransformBlock {
        ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), residual).unwrap()
    }

    const AC_RESIDUAL: [i32; DCT_DCT_4X4_COEFF_COUNT] = [
        40, -12, 7, -3, 18, 5, -22, 9, -30, 14, 2, -8, 11, -6, 25, -17,
    ];

    #[test]
    fn full_block_quantizes_real_ac_per_coefficient() {
        let transformed = full_transform(&AC_RESIDUAL);
        let block = QuantizedTransformBlock::dct_dct_4x4(&transformed, params(1)).unwrap();
        let coeffs = transformed.coefficients();
        let (dcq, acq) = (block.dc_quantizer(), block.ac_quantizer());

        let mut expected = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected[0] = expected_level(coeffs[0], dcq);
        for k in 1..DCT_DCT_4X4_COEFF_COUNT {
            expected[k] = expected_level(coeffs[k], acq);
        }
        assert_eq!(block.quantized(), &expected);
        assert!(
            block.quantized()[1..].iter().any(|&level| level != 0),
            "expected non-zero AC levels for a non-uniform residual"
        );
    }

    #[test]
    fn full_block_dequantized_equals_independent_recon_dequant() {
        let transformed = full_transform(&AC_RESIDUAL);
        let block = QuantizedTransformBlock::dct_dct_4x4(&transformed, params(2)).unwrap();
        let dequant_params = DequantBlockParams {
            dc_quant: block.dc_quantizer(),
            ac_quant: block.ac_quantizer(),
            tx_width: DCT_DCT_4X4_WIDTH,
            tx_height: DCT_DCT_4X4_HEIGHT,
            dq_denom: block.params().dq_denom(),
            bit_depth: block.params().bit_depth(),
        };
        let mut independent = [0; DCT_DCT_4X4_COEFF_COUNT];
        dequantize_block(&dequant_params, block.quantized(), &mut independent).unwrap();
        assert_eq!(block.dequantized(), &independent);
    }

    #[test]
    fn full_block_emitted_levels_satisfy_dequant_product_guard() {
        let transformed = full_transform(&AC_RESIDUAL);
        let block = QuantizedTransformBlock::dct_dct_4x4(&transformed, params(0)).unwrap();
        let (dcq, acq) = (block.dc_quantizer(), block.ac_quantizer());
        for (index, &level) in block.quantized().iter().enumerate() {
            let q = if index == 0 { dcq } else { acq };
            let product = u64::from(level.unsigned_abs()) * u64::from(q);
            assert!(product <= DEQUANT_PRODUCT_MAX, "index {index}");
        }
    }

    #[test]
    fn full_block_decode_verifies_close_to_residual_at_low_q() {
        let transformed = full_transform(&AC_RESIDUAL);
        let block = QuantizedTransformBlock::dct_dct_4x4(&transformed, params(0)).unwrap();
        let reconstructed = inverse_4x4_dct_dct(block.dequantized());
        for (k, (&got, &want)) in reconstructed.iter().zip(AC_RESIDUAL.iter()).enumerate() {
            assert!((got - want).abs() <= 12, "index {k}: {got} vs {want}");
        }
    }

    #[test]
    fn full_block_dc_only_alias_matches_general_entry_point() {
        let transformed = transform(9);
        let general = QuantizedTransformBlock::dct_dct_4x4(&transformed, params(3)).unwrap();
        let alias = QuantizedTransformBlock::dct_dct_4x4_dc_only(&transformed, params(3)).unwrap();
        assert_eq!(general, alias);
    }
}
