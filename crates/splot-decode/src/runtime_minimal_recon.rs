// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal traced reconstruction handoff for the documented runtime tier.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraSquareBlockSize,
    OutputIndex, PixelFormat, PlaneId, PlaneRect, PlaneSize,
};

use crate::Result;
use crate::tile_payload::MinimalRuntimeReconstructionTrace;

const MINIMAL_LUMA_WIDTH: usize = 64;
const MINIMAL_LUMA_HEIGHT: usize = 64;
const MINIMAL_CHROMA_WIDTH: usize = 32;
const MINIMAL_CHROMA_HEIGHT: usize = 32;
const MINIMAL_LUMA_LOG2_SIZE: u8 = 6;
const NEUTRAL_CHROMA_SAMPLE: u8 = 128;

/// Reconstructs the current traced minimal runtime frame.
pub(crate) fn reconstruct_minimal_traced_frame(
    trace: MinimalRuntimeReconstructionTrace,
) -> Result<DecodedFrame<u8>> {
    match trace {
        MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64 => {
            reconstruct_luma_dc_neutral_chroma_8bit420_64x64()
        }
    }
}

fn reconstruct_luma_dc_neutral_chroma_8bit420_64x64() -> Result<DecodedFrame<u8>> {
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

    // The current traced fixture decodes chroma symbol 6 (H_PRED), but the repo
    // does not yet expose a horizontal-prediction primitive. Keep the existing
    // minimal output contract honest by materializing neutral chroma through the
    // checked workspace only; the matrix row keeps broad/chroma reconstruction
    // partial until H/V prediction lands.
    let chroma_rect = PlaneRect::new(0, 0, MINIMAL_CHROMA_WIDTH, MINIMAL_CHROMA_HEIGHT)?;
    workspace.fill_rect(PlaneId::U, chroma_rect, NEUTRAL_CHROMA_SAMPLE)?;
    workspace.fill_rect(PlaneId::V, chroma_rect, NEUTRAL_CHROMA_SAMPLE)?;

    Ok(workspace.freeze()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_recon::DecodedFrameHashInput;

    use super::*;

    const EXPECTED_DIGEST: &str =
        "cb11e05cb5da949c0e0f5b5a7cb310df35a96a22c45d1ada70d950859fe697d1";

    fn reconstruct() -> DecodedFrame<u8> {
        reconstruct_minimal_traced_frame(
            MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
        )
        .unwrap()
    }

    #[test]
    fn traced_luma_dc_neutral_chroma_reconstruction_predicts_visible_samples() {
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
                .all(|sample| *sample == 128)
        );
        assert!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .all(|sample| *sample == 128)
        );
        assert!(!frame.y().samples().contains(&0));
        assert!(!frame.u().unwrap().samples().contains(&0));
        assert!(!frame.v().unwrap().samples().contains(&0));
    }

    #[test]
    fn traced_luma_dc_neutral_chroma_reconstruction_hash_matches_minimal_contract() {
        let frame = reconstruct();
        let hash = DecodedFrameHashInput::new(&frame).compute_hash();

        assert_eq!(hash.to_hex(), EXPECTED_DIGEST);
    }
}
