// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder closed-loop reconstruction foundation.
//!
//! This module advances `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`. It composes the
//! existing private encoder-policy stages (residual, forward transform,
//! quantization) with the decoder-visible `splot-recon` reconstruction process to
//! prove the encoder's quantized decisions reconstruct to exactly the samples a
//! conforming AV2 decoder would produce.
//!
//! Every decoder-visible step is performed by `splot-recon`:
//!
//! - AV2 §7.13.2.10 DC intra prediction
//!   (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10`), with the
//!   no-neighbor midpoint that applies to the top-left block of a frame;
//! - AV2 §7.14.2 / §7.14.4 dequantization
//!   (`#s-7-14-2`, `#s-7-14-4`);
//! - AV2 §7.15.4 inverse transform (`#s-7-15-4`);
//! - AV2 §7.14.3 reconstruct / residual addition with clip (`#s-7-14-3`).
//!
//! The reconstructed block is frozen into a `splot-recon` current-frame workspace
//! and hashed with the decoded-frame hash contract, so the deterministic artifact
//! external decoders are later compared against is produced and tested now.
//!
//! The current subset is the top-left 8-bit luma 4x4 DCT_DCT DC-only uniform
//! block only. The module does not emit tile payloads, write packets, store
//! references, expose a public encoder API, or reconstruct chroma, inter,
//! multi-block, non-uniform, or non-4x4 content.

#![allow(dead_code)]

use splot_recon::{
    BitDepth as ReconBitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameHash,
    DecodedFrameHashInput, DecodedFrameInfo, DequantBlockParams, IntraDcEdges,
    IntraSquareBlockSize, InverseTransform1dType, InverseTransform2dDim, InverseTransform2dOuter,
    OutputIndex, PixelFormat, PlaneId, PlaneRect, PlaneRef, PlaneSize, QuantizerDeltas,
    ac_quantizer, dc_quantizer, dequantize_block, inverse_transform_2d_outer,
    predict_intra_dc_square_into, reconstruct_add_residual, transform_shift,
};

use crate::error::{Error, Result};
use crate::forward_transform::ForwardTransformBlock;
use crate::quantization::{FixedQuantizationParams, QuantizedTransformBlock};
use crate::residual::ResidualBlock;

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const DCT_DCT_4X4_LOG2: u8 = 2;

/// Reconstructed coefficients and decoder-visible samples for one private block.
#[derive(Debug)]
pub(crate) struct MinimalClosedLoopReconstruction {
    prediction: [u8; DCT_DCT_4X4_COEFF_COUNT],
    quantized_block: QuantizedTransformBlock,
    reconstructed: [u8; DCT_DCT_4X4_COEFF_COUNT],
    hash: DecodedFrameHash,
}

impl MinimalClosedLoopReconstruction {
    /// Reconstructs the current 8-bit luma 4x4 DCT_DCT DC-only top-left subset.
    ///
    /// `source` is a borrowed input plane view whose visible size must be exactly
    /// 4x4. The block is predicted with AV2 §7.13.2.10 no-neighbor DC intra
    /// prediction, a uniform residual is formed and quantized through the private
    /// encoder stages, and the decoder-visible samples are reconstructed entirely
    /// through `splot-recon`.
    pub(crate) fn reconstruct_luma_4x4_dc_only(
        source: PlaneRef<'_, u8>,
        params: FixedQuantizationParams,
    ) -> Result<Self> {
        let plane = PlaneId::Y;
        let bit_depth = params.bit_depth();
        if bit_depth != ReconBitDepth::Eight {
            return Err(Error::ClosedLoopUnsupportedBitDepth { bit_depth });
        }

        let visible = source.visible_size();
        if visible.width() != DCT_DCT_4X4_WIDTH || visible.height() != DCT_DCT_4X4_HEIGHT {
            return Err(Error::ClosedLoopUnsupportedSourceSize {
                plane,
                actual: visible,
                expected_width: DCT_DCT_4X4_WIDTH,
                expected_height: DCT_DCT_4X4_HEIGHT,
            });
        }
        let block = transform_block_rect()?;

        // AV2 §7.13.2.10 no-neighbor DC intra prediction (decoder-visible, recon).
        let prediction = predict_dc_no_neighbor(plane, bit_depth)?;

        // Encoder-policy residual = source - prediction.
        let residual = ResidualBlock::from_plane_prediction(
            plane,
            source,
            block,
            &prediction,
            DCT_DCT_4X4_WIDTH,
        )?;
        let mut residual_i32 = [0i32; DCT_DCT_4X4_COEFF_COUNT];
        for (slot, &sample) in residual_i32.iter_mut().zip(residual.samples()) {
            *slot = i32::from(sample);
        }

        // Encoder-policy forward transform and fixed quantization.
        let transformed = ForwardTransformBlock::dct_dct_4x4_dc_only(plane, block, &residual_i32)?;
        let quantized_block = QuantizedTransformBlock::dct_dct_4x4_dc_only(&transformed, params)?;

        // Decoder-visible dequant -> inverse transform -> residual addition (recon).
        let reconstructed = reconstruct_dc_only_from_quantized(
            &prediction,
            params,
            plane,
            quantized_block.quantized(),
        )?;

        // Freeze the reconstructed block into a recon current-frame workspace and hash it.
        let hash = reconstructed_frame_hash(bit_depth, &reconstructed)?;

        Ok(Self {
            prediction,
            quantized_block,
            reconstructed,
            hash,
        })
    }

    /// Returns the source plane identity.
    pub(crate) const fn plane(&self) -> PlaneId {
        self.quantized_block.plane()
    }

    /// Returns the visible-plane-relative transform block rectangle.
    pub(crate) const fn block(&self) -> PlaneRect {
        self.quantized_block.block()
    }

    /// Returns the fixed quantization parameters used for this block.
    pub(crate) const fn params(&self) -> FixedQuantizationParams {
        self.quantized_block.params()
    }

    /// Returns the AV2 §7.13.2.10 DC intra prediction samples.
    pub(crate) const fn prediction(&self) -> &[u8; DCT_DCT_4X4_COEFF_COUNT] {
        &self.prediction
    }

    /// Returns the underlying quantized transform block.
    pub(crate) const fn quantized_block(&self) -> &QuantizedTransformBlock {
        &self.quantized_block
    }

    /// Returns row-major quantized coefficients.
    pub(crate) const fn quantized(&self) -> &[i32; DCT_DCT_4X4_COEFF_COUNT] {
        self.quantized_block.quantized()
    }

    /// Returns row-major dequantized coefficients from `splot-recon`.
    pub(crate) const fn dequantized(&self) -> &[i32; DCT_DCT_4X4_COEFF_COUNT] {
        self.quantized_block.dequantized()
    }

    /// Returns the decoder-visible reconstructed samples.
    pub(crate) const fn reconstructed(&self) -> &[u8; DCT_DCT_4X4_COEFF_COUNT] {
        &self.reconstructed
    }

    /// Returns the decoded-frame hash of the reconstructed workspace.
    pub(crate) const fn hash(&self) -> &DecodedFrameHash {
        &self.hash
    }
}

/// Reconstructs decoder-visible samples from already-quantized coefficients.
///
/// Used both by the closed loop and by the emitted-decision equivalence proof so
/// the decoded coefficient stream reconstructs through the exact same
/// `splot-recon` dequant -> inverse transform -> residual addition path.
pub(crate) fn reconstruct_dc_only_from_quantized(
    prediction: &[u8; DCT_DCT_4X4_COEFF_COUNT],
    params: FixedQuantizationParams,
    plane: PlaneId,
    quantized: &[i32; DCT_DCT_4X4_COEFF_COUNT],
) -> Result<[u8; DCT_DCT_4X4_COEFF_COUNT]> {
    let bit_depth = params.bit_depth();
    let block = transform_block_rect()?;

    // AV2 §7.14.2 / §7.14.4 dequantization (decoder-visible, recon).
    let dequantized = dequantize_dc_only(params, plane, block, quantized)?;

    // AV2 §7.15.4 inverse transform (decoder-visible, recon). The §7.15.4
    // `Transform_Shift` values are sourced from `splot-recon`, not hand-copied,
    // so a recon table correction cannot silently desync the closed loop.
    let (row_shift, col_shift) =
        transform_shift(u32::from(DCT_DCT_4X4_LOG2), u32::from(DCT_DCT_4X4_LOG2)).map_err(
            |source| Error::ClosedLoopTransformShift {
                plane,
                block,
                source,
            },
        )?;
    let mut reconstructed_residual = [0i32; DCT_DCT_4X4_COEFF_COUNT];
    inverse_transform_2d_outer(
        &dct_dct_4x4_inverse_params(bit_depth, row_shift, col_shift),
        &dequantized,
        &mut reconstructed_residual,
    )
    .map_err(|source| Error::ClosedLoopInverseTransform {
        plane,
        block,
        source,
    })?;

    // AV2 §7.14.3 reconstruct / residual addition with clip (decoder-visible, recon).
    let mut reconstructed = [0u8; DCT_DCT_4X4_COEFF_COUNT];
    reconstruct_add_residual(
        prediction,
        &reconstructed_residual,
        bit_depth,
        &mut reconstructed,
    )
    .map_err(|source| Error::ClosedLoopResidualAdd {
        plane,
        block,
        source,
    })?;

    Ok(reconstructed)
}

fn predict_dc_no_neighbor(
    plane: PlaneId,
    bit_depth: ReconBitDepth,
) -> Result<[u8; DCT_DCT_4X4_COEFF_COUNT]> {
    let block_size = IntraSquareBlockSize::new(DCT_DCT_4X4_LOG2)
        .map_err(|source| Error::ClosedLoopPredict { plane, source })?;
    let mut prediction = [0u8; DCT_DCT_4X4_COEFF_COUNT];
    predict_intra_dc_square_into(
        bit_depth,
        block_size,
        IntraDcEdges::none(),
        &mut prediction,
        DCT_DCT_4X4_WIDTH,
    )
    .map_err(|source| Error::ClosedLoopPredict { plane, source })?;
    Ok(prediction)
}

fn dequantize_dc_only(
    params: FixedQuantizationParams,
    plane: PlaneId,
    block: PlaneRect,
    quantized: &[i32; DCT_DCT_4X4_COEFF_COUNT],
) -> Result<[i32; DCT_DCT_4X4_COEFF_COUNT]> {
    let deltas = QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    };
    let dequant_params = DequantBlockParams {
        dc_quant: dc_quantizer(plane, params.qindex(), deltas, params.bit_depth()),
        ac_quant: ac_quantizer(plane, params.qindex(), deltas, params.bit_depth()),
        tx_width: DCT_DCT_4X4_WIDTH,
        tx_height: DCT_DCT_4X4_HEIGHT,
        dq_denom: params.dq_denom(),
        bit_depth: params.bit_depth(),
    };
    let mut dequantized = [0i32; DCT_DCT_4X4_COEFF_COUNT];
    dequantize_block(&dequant_params, quantized, &mut dequantized).map_err(|source| {
        Error::ClosedLoopDequant {
            plane,
            block,
            source,
        }
    })?;
    Ok(dequantized)
}

fn reconstructed_frame_hash(
    bit_depth: ReconBitDepth,
    reconstructed: &[u8; DCT_DCT_4X4_COEFF_COUNT],
) -> Result<DecodedFrameHash> {
    let frame = build_reconstructed_frame(bit_depth, reconstructed)?;
    Ok(DecodedFrameHashInput::new(&frame).compute_hash())
}

fn build_reconstructed_frame(
    bit_depth: ReconBitDepth,
    reconstructed: &[u8; DCT_DCT_4X4_COEFF_COUNT],
) -> Result<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_HEIGHT).map_err(|source| {
        Error::ClosedLoopWorkspace {
            context: "reconstructed luma size",
            source,
        }
    })?;
    let luma_rect = transform_block_rect()?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        PixelFormat::Monochrome,
        luma_size,
        luma_rect,
    )
    .map_err(|source| Error::ClosedLoopWorkspace {
        context: "reconstructed frame info",
        source,
    })?;
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(info, 0).map_err(|source| Error::ClosedLoopWorkspace {
            context: "reconstructed workspace",
            source,
        })?;
    workspace
        .write_rect(PlaneId::Y, luma_rect, reconstructed, DCT_DCT_4X4_WIDTH)
        .map_err(|source| Error::ClosedLoopWorkspace {
            context: "reconstructed block write",
            source,
        })?;
    workspace
        .freeze()
        .map_err(|source| Error::ClosedLoopWorkspace {
            context: "reconstructed workspace freeze",
            source,
        })
}

fn transform_block_rect() -> Result<PlaneRect> {
    PlaneRect::new(0, 0, DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_HEIGHT).map_err(|source| {
        Error::ClosedLoopWorkspace {
            context: "transform block rect",
            source,
        }
    })
}

fn dct_dct_4x4_inverse_params(
    bit_depth: ReconBitDepth,
    row_shift: u8,
    col_shift: u8,
) -> InverseTransform2dOuter {
    InverseTransform2dOuter {
        log2_width: u32::from(DCT_DCT_4X4_LOG2),
        log2_height: u32::from(DCT_DCT_4X4_LOG2),
        lossless: false,
        plane_tx_type_is_idtx: false,
        row_type: InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
        col_type: InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
        row_shift,
        col_shift,
        bit_depth,
        dpcm: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::coefficient_tokenization::{
        CoefficientTokenSyntax, CoefficientTokenizationPlan, roundtrip_entropy_tokens,
        tokenize_quantized_4x4_dct_dct_dc_only,
    };

    const DC_NO_NEIGHBOR_8BIT: u8 = 128;

    fn rect_4x4() -> PlaneRect {
        PlaneRect::new(0, 0, DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_HEIGHT).unwrap()
    }

    fn source_view(samples: &[u8; DCT_DCT_4X4_COEFF_COUNT]) -> PlaneRef<'_, u8> {
        PlaneRef::new(samples, DCT_DCT_4X4_WIDTH, rect_4x4()).unwrap()
    }

    fn params(qindex: u32) -> FixedQuantizationParams {
        FixedQuantizationParams::new(ReconBitDepth::Eight, qindex).unwrap()
    }

    fn reconstruct(
        samples: &[u8; DCT_DCT_4X4_COEFF_COUNT],
        qindex: u32,
    ) -> MinimalClosedLoopReconstruction {
        MinimalClosedLoopReconstruction::reconstruct_luma_4x4_dc_only(
            source_view(samples),
            params(qindex),
        )
        .unwrap()
    }

    /// Recovers the quantized DC coefficient implied by a decoded token stream.
    fn recover_quantized_dc(plan: &CoefficientTokenizationPlan, decoded_symbols: &[u8]) -> i32 {
        assert_eq!(plan.tokens().len(), decoded_symbols.len());
        if plan.eob() == 0 {
            return 0;
        }
        let mut magnitude = 0i32;
        let mut negative = false;
        for (token, &symbol) in plan.tokens().iter().zip(decoded_symbols) {
            match token.syntax() {
                CoefficientTokenSyntax::CoeffBaseEob => magnitude = i32::from(symbol) + 1,
                CoefficientTokenSyntax::CoeffBr => magnitude += i32::from(symbol),
                CoefficientTokenSyntax::DcSign => negative = symbol == 1,
                // This helper recovers the single-DC closed-loop magnitude, which
                // never carries a non-EOB `coeff_base` (that is a multi-coefficient
                // trace symbol), so it is a no-op here.
                CoefficientTokenSyntax::AllZero
                | CoefficientTokenSyntax::EobPt16
                | CoefficientTokenSyntax::CoeffBase => {}
            }
        }
        if negative { -magnitude } else { magnitude }
    }

    #[test]
    fn predicts_no_neighbor_dc_midpoint_for_top_left_block() {
        let block = reconstruct(&[140; DCT_DCT_4X4_COEFF_COUNT], 0);

        assert_eq!(block.plane(), PlaneId::Y);
        assert_eq!(block.block(), rect_4x4());
        assert_eq!(
            block.prediction(),
            &[DC_NO_NEIGHBOR_8BIT; DCT_DCT_4X4_COEFF_COUNT]
        );
    }

    #[test]
    fn qindex_zero_flat_block_reconstructs_losslessly() {
        for value in [128u8, 129, 131, 124] {
            let block = reconstruct(&[value; DCT_DCT_4X4_COEFF_COUNT], 0);

            assert_eq!(
                block.reconstructed(),
                &[value; DCT_DCT_4X4_COEFF_COUNT],
                "value {value}"
            );
        }
    }

    #[test]
    fn reconstruction_and_hash_are_deterministic() {
        let first = reconstruct(&[129; DCT_DCT_4X4_COEFF_COUNT], 0);
        let second = reconstruct(&[129; DCT_DCT_4X4_COEFF_COUNT], 0);

        assert_eq!(first.reconstructed(), second.reconstructed());
        assert_eq!(first.hash(), second.hash());
    }

    #[test]
    fn hash_matches_independently_built_workspace() {
        let value = 129u8;
        let block = reconstruct(&[value; DCT_DCT_4X4_COEFF_COUNT], 0);
        assert_eq!(block.reconstructed(), &[value; DCT_DCT_4X4_COEFF_COUNT]);

        // Build an independent monochrome 4x4 frame filled with the reconstructed
        // value (fill path, not the write_rect path) and compare its hash.
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            ReconBitDepth::Eight,
            PixelFormat::Monochrome,
            PlaneSize::new(DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_HEIGHT).unwrap(),
            rect_4x4(),
        )
        .unwrap();
        let independent = CurrentFrameWorkspace::<u8>::new(info, value)
            .unwrap()
            .freeze()
            .unwrap();
        let expected = DecodedFrameHashInput::new(&independent).compute_hash();

        assert_eq!(block.hash(), &expected);
    }

    #[test]
    fn emitted_coefficient_decisions_reconstruct_identically() {
        // Flat source 129 -> residual 1 -> DC coeff 32 -> qindex0 quantized DC 4
        // (within the base-symbol tier), reconstructing losslessly to the source.
        let value = 129u8;
        let block = reconstruct(&[value; DCT_DCT_4X4_COEFF_COUNT], 0);
        assert_eq!(block.quantized()[0], 4);
        assert_eq!(block.reconstructed(), &[value; DCT_DCT_4X4_COEFF_COUNT]);

        // Tokenize the same quantized block and roundtrip the emitted decisions
        // through the in-tree AV2 §8.2 symbol coder.
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(block.quantized_block()).unwrap();
        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();

        // The decoded token stream recovers the exact quantized DC coefficient...
        let recovered_dc = recover_quantized_dc(&plan, proof.decoded_symbols());
        assert_eq!(recovered_dc, block.quantized()[0]);

        // ...and reconstructing from that recovered coefficient yields the same
        // decoder-visible samples as the local closed loop.
        let mut recovered_quantized = [0i32; DCT_DCT_4X4_COEFF_COUNT];
        recovered_quantized[0] = recovered_dc;
        let reconstructed_from_emitted = reconstruct_dc_only_from_quantized(
            block.prediction(),
            block.params(),
            PlaneId::Y,
            &recovered_quantized,
        )
        .unwrap();

        assert_eq!(&reconstructed_from_emitted, block.reconstructed());
    }

    #[test]
    fn negative_dc_emitted_decisions_reconstruct_identically() {
        // Flat source 127 -> residual -1 -> negative DC coefficient.
        let value = 127u8;
        let block = reconstruct(&[value; DCT_DCT_4X4_COEFF_COUNT], 0);
        assert_eq!(block.quantized()[0], -4);
        assert_eq!(block.reconstructed(), &[value; DCT_DCT_4X4_COEFF_COUNT]);

        let plan = tokenize_quantized_4x4_dct_dct_dc_only(block.quantized_block()).unwrap();
        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();
        let recovered_dc = recover_quantized_dc(&plan, proof.decoded_symbols());

        assert_eq!(recovered_dc, block.quantized()[0]);
        let mut recovered_quantized = [0i32; DCT_DCT_4X4_COEFF_COUNT];
        recovered_quantized[0] = recovered_dc;
        let reconstructed_from_emitted = reconstruct_dc_only_from_quantized(
            block.prediction(),
            block.params(),
            PlaneId::Y,
            &recovered_quantized,
        )
        .unwrap();

        assert_eq!(&reconstructed_from_emitted, block.reconstructed());
    }

    #[test]
    fn all_zero_block_reconstructs_to_prediction() {
        // Flat source equal to the no-neighbor DC midpoint -> zero residual.
        let block = reconstruct(&[DC_NO_NEIGHBOR_8BIT; DCT_DCT_4X4_COEFF_COUNT], 0);

        assert_eq!(block.quantized(), &[0; DCT_DCT_4X4_COEFF_COUNT]);
        assert_eq!(
            block.reconstructed(),
            &[DC_NO_NEIGHBOR_8BIT; DCT_DCT_4X4_COEFF_COUNT]
        );

        let plan = tokenize_quantized_4x4_dct_dct_dc_only(block.quantized_block()).unwrap();
        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();
        assert_eq!(recover_quantized_dc(&plan, proof.decoded_symbols()), 0);
    }

    #[test]
    fn rejects_non_uniform_source_block() {
        let mut samples = [130u8; DCT_DCT_4X4_COEFF_COUNT];
        samples[5] = 131;
        let err = MinimalClosedLoopReconstruction::reconstruct_luma_4x4_dc_only(
            source_view(&samples),
            params(0),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::ForwardTransformNonUniformResidual {
                plane: PlaneId::Y,
                ..
            }
        ));
    }

    #[test]
    fn rejects_source_view_that_is_not_4x4() {
        let samples = [128u8; 8];
        let view = PlaneRef::new(&samples, 4, PlaneRect::new(0, 0, 4, 2).unwrap()).unwrap();
        let err = MinimalClosedLoopReconstruction::reconstruct_luma_4x4_dc_only(view, params(0))
            .unwrap_err();

        assert!(matches!(
            err,
            Error::ClosedLoopUnsupportedSourceSize {
                plane: PlaneId::Y,
                expected_width: 4,
                expected_height: 4,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_bit_depth() {
        let samples = [128u8; DCT_DCT_4X4_COEFF_COUNT];
        let err = MinimalClosedLoopReconstruction::reconstruct_luma_4x4_dc_only(
            source_view(&samples),
            FixedQuantizationParams::new(ReconBitDepth::Ten, 0).unwrap(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::ClosedLoopUnsupportedBitDepth {
                bit_depth: ReconBitDepth::Ten,
            }
        ));
    }
}
