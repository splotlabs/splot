// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, InterIntraMode,
    InterpolationFilter as ReconInterpolationFilter, OutputIndex, PixelFormat, PlaneId, PlaneRect,
    PlaneSize,
};

use super::{
    chroma_smooth_grid_dimensions, ensure_intra_leaf_quantizer_delta_scope,
    inter_residual_geometry_supported_flags, predict_interintra_planes,
};
use crate::bitstream::tile_payload::TileBlockDecodedState;
use crate::error::DecodeError;
use crate::prediction::inter::SPEC_MODE_INFO;
use crate::prediction::inter::{
    BawpSyntax, InterBlock, InterIntraPrediction, Mv, PlacedInterBlock, mc::CompoundBlend,
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn inter_residual_geometry_allows_shared_leaves() {
    assert!(inter_residual_geometry_supported_flags(false, false));
}

#[test]
fn inter_residual_geometry_rejects_chroma_partitioned_leaves() {
    assert!(!inter_residual_geometry_supported_flags(true, false));
    assert!(!inter_residual_geometry_supported_flags(false, true));
}

#[test]
fn inter_frame_intra_leaf_rejects_nonzero_quantizer_deltas() {
    let result = ensure_intra_leaf_quantizer_delta_scope(false, false, ByteOffset::new(13));
    assert!(matches!(
        &result,
        Err(DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == "inter_block_intra_leaf_nonzero_quantizer_delta"
                && unsupported.spec_section() == SPEC_MODE_INFO
                && unsupported.byte_offset() == Some(ByteOffset::new(13))
    ));
}

#[test]
fn intra_leaf_quantizer_delta_guard_allows_installed_or_zero_delta_scope() {
    assert!(ensure_intra_leaf_quantizer_delta_scope(true, false, ByteOffset::new(0)).is_ok());
    assert!(ensure_intra_leaf_quantizer_delta_scope(false, true, ByteOffset::new(0)).is_ok());
}

#[test]
fn chroma_smooth_grid_dimensions_follow_chroma_sampling() {
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv420),
        (9, 10)
    );
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv422),
        (17, 10)
    );
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv444),
        (17, 19)
    );
}

#[test]
fn warp_interintra_mode_three_maps_to_smooth() -> TestResult {
    let prediction = super::warp::interintra_prediction_mode(
        super::warp::WarpInterIntraSyntax {
            enabled: true,
            mode: Some(3),
            use_wedge: false,
            wedge_index: None,
        },
        ByteOffset::new(19),
    )?;

    assert_eq!(
        prediction,
        Some(InterIntraPrediction::SmoothMask {
            mode: InterIntraMode::Smooth
        })
    );
    Ok(())
}

#[test]
fn interintra_smooth_builds_prediction_from_intra_edges() -> TestResult {
    let workspace = CurrentFrameWorkspace::<u8>::new(monochrome_info(8, 8)?, 128)?;
    let block_decoded = TileBlockDecodedState::new(1, 1, 1, 16, 16, 16)?;
    let placed = placed_luma_block(0, 0, 8, 8, InterIntraMode::Smooth);

    let planes = predict_interintra_planes(
        &workspace,
        &placed,
        &block_decoded,
        InterIntraMode::Smooth,
        false,
        BitDepth::Eight,
        ByteOffset::new(0),
    )?;

    assert_eq!(planes.len(), 1);
    assert_eq!(planes[0].plane, PlaneId::Y);
    assert_eq!(planes[0].samples.len(), 64);
    assert!(
        planes[0]
            .samples
            .iter()
            .all(|sample| (127..=129).contains(sample))
    );
    Ok(())
}

fn monochrome_info(width: usize, height: usize) -> splot_recon::Result<DecodedFrameInfo> {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(width, height)?,
        PlaneRect::new(0, 0, width, height)?,
    )
}

fn placed_luma_block(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    mode: InterIntraMode,
) -> PlacedInterBlock {
    PlacedInterBlock {
        luma_x: x,
        luma_y: y,
        luma_w: width,
        luma_h: height,
        chroma_luma_x: x,
        chroma_luma_y: y,
        chroma_luma_w: width,
        chroma_luma_h: height,
        has_chroma: false,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: 0,
            ref_frame1: None,
            mv: Mv { row: 0, col: 0 },
            mv1: Mv { row: 0, col: 0 },
            interp: ReconInterpolationFilter::EightTap,
            warp_params: None,
            bawp: BawpSyntax::default(),
            interintra: Some(InterIntraPrediction::SmoothMask { mode }),
            compound_blend: CompoundBlend::default(),
            residual: None,
        },
    }
}
