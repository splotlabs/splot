// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder 16x16 quantization stage.
//!
//! This module advances `ENC-FORWARD-TRANSFORM-DCT-16X16` (the 16x16 quantizer that
//! pairs with the 16x16 forward DCT in [`crate::forward_transform_16x16`]), the 16x16
//! analogue of [`crate::quantization`]'s 4x4 per-coefficient quantizer. It is
//! encoder-policy arithmetic over already-produced transform coefficients;
//! decoder-visible dequantization remains delegated to `splot-recon`'s AV2 § 7.14.2 /
//! § 7.14.4 implementation (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2` and
//! `#s-7-14-4`). The fixed quantizer `qindex` references AV2 § 5.18.6.1 `base_q_idx`
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`).
//!
//! [`QuantizedTransformBlock16x16::dct_dct_16x16`] quantizes all 256 coefficients of a
//! real 16x16 DCT_DCT block per-coefficient (index 0 with the DC quantizer, the rest
//! with the AC quantizer) and dequantizes them through `splot-recon`, so the stored
//! dequantized array is the decoder reconstruction of the emitted levels. It does not
//! emit § 5.20.7.28 quantized coefficient syntax, select rate-control values,
//! tokenize coefficients, write tile bodies, or produce [`crate::Packet`] values.

#![allow(dead_code)]

use splot_recon::{
    BitDepth as ReconBitDepth, DequantBlockParams, PlaneId, PlaneRect, QuantizerDeltas,
    ac_quantizer, dc_quantizer, dequantize_block, max_quantizer_index,
};

use crate::error::{Error, Result};
use crate::forward_transform_16x16::{
    DCT_DCT_16X16_COEFF_COUNT, DCT_DCT_16X16_HEIGHT, DCT_DCT_16X16_WIDTH, ForwardTransformBlock16x16,
};

const DEQUANT_ROUNDING_SCALE: u128 = 8;
const DEQUANT_PRODUCT_MAX: u64 = 0xFF_FFFF;

/// Quantized and dequantized 16x16 coefficient block for the current private subset.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct QuantizedTransformBlock16x16 {
    plane: PlaneId,
    block: PlaneRect,
    bit_depth: ReconBitDepth,
    qindex: u32,
    dq_denom: u32,
    dc_quantizer: u32,
    ac_quantizer: u32,
    quantized: [i32; DCT_DCT_16X16_COEFF_COUNT],
    dequantized: [i32; DCT_DCT_16X16_COEFF_COUNT],
}

impl QuantizedTransformBlock16x16 {
    /// Quantizes a real 16x16 DCT_DCT transform block: every one of the 256
    /// coefficients is quantized per-coefficient (index 0 with the DC quantizer, the
    /// rest with the AC quantizer) by the round-to-nearest policy, then the levels are
    /// dequantized through `splot-recon`'s AV2 § 7.14 dequantization so the stored
    /// `dequantized` array is exactly what the decoder reconstructs from the emitted
    /// levels. This accepts any [`ForwardTransformBlock16x16`] — a flat (DC-only)
    /// block or one with real non-zero AC content.
    ///
    /// `bit_depth`, `qindex`, and `dq_denom` mirror the 4x4
    /// [`crate::quantization::FixedQuantizationParams`] inputs; `qindex` is AV2
    /// § 5.18.6.1 `base_q_idx` and `dq_denom` is the AV2 § 7.14.4 dequant denominator.
    ///
    /// # Errors
    /// Returns [`Error::QuantizationQIndexOutOfRange`] /
    /// [`Error::QuantizationInvalidDequantDenominator`] for invalid quantizer inputs,
    /// the per-coefficient quantization errors (coefficient out of the dequant-visible
    /// range, arithmetic overflow, or a dequant product beyond the AV2 24-bit limit),
    /// [`Error::QuantizationUnsupportedShape`] for a non-16x16 block, or
    /// [`Error::QuantizationDequant`] when `splot-recon` dequantization fails.
    pub(crate) fn dct_dct_16x16(
        transformed: &ForwardTransformBlock16x16,
        bit_depth: ReconBitDepth,
        qindex: u32,
        dq_denom: u32,
    ) -> Result<Self> {
        let plane = transformed.plane();
        let block = transformed.block();
        validate_16x16_shape(plane, block)?;

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

        let dc_quantizer = dc_quantizer(plane, qindex, zero_deltas(), bit_depth);
        let ac_quantizer = ac_quantizer(plane, qindex, zero_deltas(), bit_depth);
        let coefficients = transformed.coefficients();
        let mut quantized = [0; DCT_DCT_16X16_COEFF_COUNT];
        for (index, (&coefficient, out)) in
            coefficients.iter().zip(quantized.iter_mut()).enumerate()
        {
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

        let mut dequantized = [0; DCT_DCT_16X16_COEFF_COUNT];
        let dequant_params = DequantBlockParams {
            dc_quant: dc_quantizer,
            ac_quant: ac_quantizer,
            tx_width: DCT_DCT_16X16_WIDTH,
            tx_height: DCT_DCT_16X16_HEIGHT,
            dq_denom,
            bit_depth,
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
            bit_depth,
            qindex,
            dq_denom,
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

    /// Returns the active decoded bit depth.
    pub(crate) const fn bit_depth(&self) -> ReconBitDepth {
        self.bit_depth
    }

    /// Returns the fixed quantizer index (AV2 § 5.18.6.1 `base_q_idx`).
    pub(crate) const fn qindex(&self) -> u32 {
        self.qindex
    }

    /// Returns the AV2 § 7.14.4 dequant denominator.
    pub(crate) const fn dq_denom(&self) -> u32 {
        self.dq_denom
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
    pub(crate) const fn quantized(&self) -> &[i32; DCT_DCT_16X16_COEFF_COUNT] {
        &self.quantized
    }

    /// Returns row-major dequantized coefficients from `splot-recon`.
    pub(crate) const fn dequantized(&self) -> &[i32; DCT_DCT_16X16_COEFF_COUNT] {
        &self.dequantized
    }
}

fn validate_16x16_shape(plane: PlaneId, block: PlaneRect) -> Result<()> {
    if block.width() == DCT_DCT_16X16_WIDTH && block.height() == DCT_DCT_16X16_HEIGHT {
        Ok(())
    } else {
        Err(Error::QuantizationUnsupportedShape {
            plane,
            block,
            expected_width: DCT_DCT_16X16_WIDTH,
            expected_height: DCT_DCT_16X16_HEIGHT,
        })
    }
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use splot_recon::{
        InverseTransform1dType, InverseTransform2dDim, InverseTransform2dOuter,
        inverse_transform_2d_outer,
    };

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn uniform(sample: i32) -> [i32; DCT_DCT_16X16_COEFF_COUNT] {
        [sample; DCT_DCT_16X16_COEFF_COUNT]
    }

    fn forward(residual: &[i32; DCT_DCT_16X16_COEFF_COUNT]) -> ForwardTransformBlock16x16 {
        ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), residual).unwrap()
    }

    fn quantized(
        residual: &[i32; DCT_DCT_16X16_COEFF_COUNT],
        qindex: u32,
    ) -> QuantizedTransformBlock16x16 {
        QuantizedTransformBlock16x16::dct_dct_16x16(
            &forward(residual),
            ReconBitDepth::Eight,
            qindex,
            1,
        )
        .unwrap()
    }

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    fn inverse_16x16_dct_dct(coefficients: &[i32; DCT_DCT_16X16_COEFF_COUNT]) -> Vec<i32> {
        let params = InverseTransform2dOuter {
            log2_width: 4,
            log2_height: 4,
            lossless: false,
            plane_tx_type_is_idtx: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 13,
            bit_depth: ReconBitDepth::Eight,
            dpcm: None,
        };
        let mut residual = vec![0; DCT_DCT_16X16_COEFF_COUNT];
        inverse_transform_2d_outer(&params, coefficients, &mut residual).unwrap();
        residual
    }

    /// Independent round-to-nearest level for one coefficient at quantizer `q`
    /// (`dq_denom == 1`), mirroring `quantize_coefficient` so the per-coefficient
    /// quantizer selection is cross-checked rather than re-derived through the
    /// production path.
    fn expected_level(coeff: i32, q: u32) -> i32 {
        if coeff == 0 {
            return 0;
        }
        let numerator = u64::from(coeff.unsigned_abs()) * 8;
        let level = ((numerator + u64::from(q) / 2) / u64::from(q)) as i32;
        if coeff < 0 { -level } else { level }
    }

    // An asymmetric, non-uniform 16x16 residual whose forward DCT carries real
    // non-zero AC (a smooth gradient plus a high-frequency checkerboard so distinct
    // +/- magnitudes appear and a sign-order bug cannot be masked).
    fn ac_residual() -> [i32; DCT_DCT_16X16_COEFF_COUNT] {
        let mut residual = [0i32; DCT_DCT_16X16_COEFF_COUNT];
        for r in 0..DCT_DCT_16X16_HEIGHT {
            for c in 0..DCT_DCT_16X16_WIDTH {
                let gradient = (r as i32) * 3 - (c as i32) * 2;
                let checker = if (r + c) % 2 == 0 { 11 } else { -7 };
                residual[r * DCT_DCT_16X16_WIDTH + c] = gradient + checker;
            }
        }
        residual
    }

    #[test]
    fn flat_dc_only_block_quantizes_and_dequantizes() {
        let block = quantized(&uniform(0), 0);
        assert_eq!(block.plane(), PlaneId::Y);
        assert_eq!(block.block(), rect(16, 16));
        assert_eq!(block.qindex(), 0);
        assert_eq!(block.dq_denom(), 1);
        assert_eq!(block.dc_quantizer(), 64);
        assert_eq!(block.ac_quantizer(), 64);
        assert_eq!(block.quantized(), &[0; DCT_DCT_16X16_COEFF_COUNT]);
        assert_eq!(block.dequantized(), &[0; DCT_DCT_16X16_COEFF_COUNT]);
    }

    #[test]
    fn block_quantizes_real_ac_per_coefficient() {
        // Every one of the 256 coefficients is quantized by the round-to-nearest
        // policy with its selected quantizer (index 0 the DC quantizer, the rest the
        // AC quantizer), matching an independent re-derivation. Real non-zero AC
        // levels appear (not a DC-only degenerate case).
        let residual = ac_residual();
        let transformed = forward(&residual);
        let block = QuantizedTransformBlock16x16::dct_dct_16x16(
            &transformed,
            ReconBitDepth::Eight,
            1,
            1,
        )
        .unwrap();
        let coeffs = transformed.coefficients();
        let (dcq, acq) = (block.dc_quantizer(), block.ac_quantizer());
        let mut expected = [0; DCT_DCT_16X16_COEFF_COUNT];
        expected[0] = expected_level(coeffs[0], dcq);
        for k in 1..DCT_DCT_16X16_COEFF_COUNT {
            expected[k] = expected_level(coeffs[k], acq);
        }
        assert_eq!(block.quantized(), &expected);
        assert!(
            block.quantized()[1..].iter().any(|&level| level != 0),
            "expected non-zero AC levels for a non-uniform residual"
        );
    }

    #[test]
    fn dequantized_equals_independent_recon_dequant() {
        // The stored `dequantized` array is exactly what the decoder reconstructs from
        // the emitted levels: an independent `splot-recon` dequantize_block reproduces
        // it bit-for-bit.
        let residual = ac_residual();
        let block = quantized(&residual, 2);
        let dequant_params = DequantBlockParams {
            dc_quant: block.dc_quantizer(),
            ac_quant: block.ac_quantizer(),
            tx_width: DCT_DCT_16X16_WIDTH,
            tx_height: DCT_DCT_16X16_HEIGHT,
            dq_denom: block.dq_denom(),
            bit_depth: block.bit_depth(),
        };
        let mut independent = [0; DCT_DCT_16X16_COEFF_COUNT];
        dequantize_block(&dequant_params, block.quantized(), &mut independent).unwrap();
        assert_eq!(block.dequantized(), &independent);
    }

    #[test]
    fn closed_loop_random_blocks_reconstruct_within_bound() {
        // CLOSED-LOOP PROOF for the 16x16 quantizer. A deterministic LCG sweeps random
        // 8-bit residual blocks; forward16 -> quant16 -> splot-recon dequant +
        // inverse-16x16 reconstructs the residual within |err| <= BOUND at a fixed
        // qindex.
        //
        // BOUND = 12 at qindex 80: the reconstruction error combines the lossy quant
        // step (the qindex-80 quantizer rounds each coefficient to a multiple of its
        // step) with the small <= 5 DCT16 non-orthogonality residue. The worst error
        // measured over these 1500 random blocks is 10; 12 is the documented bound with
        // a small margin. The flat-DC sub-check below confirms the lossless anchor.
        const QINDEX: u32 = 80;
        const BOUND: i32 = 12;
        let mut state: u64 = 0x243F_6A88_85A3_08D3;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) % 511) as i32 - 255
        };
        let mut worst = 0i32;
        for _ in 0..1500 {
            let mut residual = [0i32; DCT_DCT_16X16_COEFF_COUNT];
            for sample in &mut residual {
                *sample = next();
            }
            let block = quantized(&residual, QINDEX);
            let reconstructed = inverse_16x16_dct_dct(block.dequantized());
            for (&got, &want) in reconstructed.iter().zip(residual.iter()) {
                let err = (got - want).abs();
                worst = worst.max(err);
                assert!(err <= BOUND, "residual {residual:?}: err {err} exceeds bound {BOUND}");
            }
        }
        assert!(worst >= 1, "expected a non-trivial quant error, got {worst}");
    }

    #[test]
    fn closed_loop_flat_dc_is_lossless_anchor() {
        // The flat (DC-only) subset is the lossless anchor: at qindex 0 a uniform
        // residual reconstructs bit-exactly through quant + dequant + inverse.
        for v in [-127, -8, -1, 0, 1, 7, 127] {
            let block = quantized(&uniform(v), 0);
            assert_eq!(inverse_16x16_dct_dct(block.dequantized()), uniform(v), "v {v}");
        }
    }

    #[test]
    fn rejects_qindex_out_of_range() {
        let err = QuantizedTransformBlock16x16::dct_dct_16x16(
            &forward(&uniform(1)),
            ReconBitDepth::Eight,
            256,
            1,
        )
        .unwrap_err();
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
        let err = QuantizedTransformBlock16x16::dct_dct_16x16(
            &forward(&uniform(1)),
            ReconBitDepth::Eight,
            0,
            0,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::QuantizationInvalidDequantDenominator { dq_denom: 0 }
        ));
    }
}
