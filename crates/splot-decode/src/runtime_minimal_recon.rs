// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal traced reconstruction handoff for the documented runtime tier.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraCardinalDirection,
    IntraCardinalEdges, IntraRectBlockSize, IntraSquareBlockSize, OutputIndex, PixelFormat,
    PlaneId, PlaneRect, PlaneSize, predict_intra_cardinal_directional_rect_into,
};

use crate::Result;
use crate::tile_payload::MinimalRuntimeReconstructionTrace;

const MINIMAL_LUMA_WIDTH: usize = 64;
const MINIMAL_LUMA_HEIGHT: usize = 64;
const MINIMAL_CHROMA_WIDTH: usize = 32;
const MINIMAL_CHROMA_HEIGHT: usize = 32;
const MINIMAL_LUMA_LOG2_SIZE: u8 = 6;
const MINIMAL_CHROMA_LOG2_SIZE: u8 = 5;
const TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE: u8 = 129;

/// Reconstructs the current traced minimal runtime frame.
pub(crate) fn reconstruct_minimal_traced_frame(
    trace: MinimalRuntimeReconstructionTrace,
) -> Result<DecodedFrame<u8>> {
    match trace {
        MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64 => {
            reconstruct_luma_dc_chroma_h_pred_8bit420_64x64()
        }
    }
}

fn reconstruct_luma_dc_chroma_h_pred_8bit420_64x64() -> Result<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let luma_rect = PlaneRect::new(0, 0, MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;

    let mut workspace = CurrentFrameWorkspace::<u8>::new(info, 0)?;
    let luma_block = IntraSquareBlockSize::new(MINIMAL_LUMA_LOG2_SIZE)?;
    workspace.predict_intra_dc_square(PlaneId::Y, 0, 0, luma_block)?;

    // AV2 §7.13.2.1 uses (1 << (BitDepth - 1)) + 1 for LeftCol when no
    // neighbor is available. The traced top-left chroma blocks use H_PRED
    // (pAngle 180 via §7.13.2.8 and §9.2), so prepare that left edge
    // explicitly for this narrow minimal tier instead of claiming broad edge
    // preparation.
    let chroma_block = IntraRectBlockSize::new(MINIMAL_CHROMA_LOG2_SIZE, MINIMAL_CHROMA_LOG2_SIZE)?;
    let chroma_left = [TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE; MINIMAL_CHROMA_HEIGHT];
    let mut chroma_prediction = [0u8; MINIMAL_CHROMA_WIDTH * MINIMAL_CHROMA_HEIGHT];
    predict_intra_cardinal_directional_rect_into(
        BitDepth::Eight,
        chroma_block,
        IntraCardinalDirection::Horizontal,
        IntraCardinalEdges::left(&chroma_left),
        &mut chroma_prediction,
        MINIMAL_CHROMA_WIDTH,
    )?;
    workspace.write_rect_block(PlaneId::U, 0, 0, chroma_block, &chroma_prediction)?;
    workspace.write_rect_block(PlaneId::V, 0, 0, chroma_block, &chroma_prediction)?;

    Ok(workspace.freeze()?)
}

/// Assembles a decoded 8-bit 4:2:0 64x64 frame from three already-reconstructed
/// planes (luma 64x64, chroma U/V 32x32) for the general intra decode path.
pub(crate) fn assemble_general_intra_frame(
    luma: &[u8],
    chroma_u: &[u8],
    chroma_v: &[u8],
) -> Result<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let luma_rect = PlaneRect::new(0, 0, MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;
    let mut workspace = CurrentFrameWorkspace::<u8>::new(info, 0)?;
    let luma_block = IntraRectBlockSize::new(MINIMAL_LUMA_LOG2_SIZE, MINIMAL_LUMA_LOG2_SIZE)?;
    workspace.write_rect_block(PlaneId::Y, 0, 0, luma_block, luma)?;
    let chroma_block = IntraRectBlockSize::new(MINIMAL_CHROMA_LOG2_SIZE, MINIMAL_CHROMA_LOG2_SIZE)?;
    workspace.write_rect_block(PlaneId::U, 0, 0, chroma_block, chroma_u)?;
    workspace.write_rect_block(PlaneId::V, 0, 0, chroma_block, chroma_v)?;
    Ok(workspace.freeze()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_recon::DecodedFrameHashInput;

    use super::*;

    const EXPECTED_DIGEST: &str =
        "dd244844938e78b226240de27e9c0acd39fc7ec2c1631319d13250fbe5f08496";

    fn reconstruct() -> DecodedFrame<u8> {
        reconstruct_minimal_traced_frame(
            MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
        )
        .unwrap()
    }

    #[test]
    fn traced_luma_dc_chroma_h_pred_reconstruction_predicts_visible_samples() {
        let frame = reconstruct();

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert_eq!(
            frame.v().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert!(frame.y().samples().iter().all(|sample| *sample == 128));
        assert!(
            frame
                .u()
                .unwrap()
                .samples()
                .iter()
                .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
        );
        assert!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
        );
        assert!(!frame.y().samples().contains(&0));
        assert!(!frame.u().unwrap().samples().contains(&0));
        assert!(!frame.v().unwrap().samples().contains(&0));
    }

    #[test]
    fn traced_luma_dc_chroma_h_pred_reconstruction_hash_matches_minimal_contract() {
        let frame = reconstruct();
        let hash = DecodedFrameHashInput::new(&frame).compute_hash();

        assert_eq!(hash.to_hex(), EXPECTED_DIGEST);
    }
}
