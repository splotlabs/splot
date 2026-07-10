// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, InterIntraMode,
    InterpolationFilter as ReconInterpolationFilter, OutputIndex, PixelFormat, PlaneId, PlaneRect,
    PlaneSize,
};

use super::prediction::{leaf_predicts_chroma, sub8x8_chroma_disables_compound};
use super::{
    chroma_smooth_grid_dimensions, ensure_intra_leaf_quantizer_delta_scope,
    inter_residual_geometry_supported_flags, inter_skip_txfm_ctx, predict_interintra_planes,
    read_inter_intra_syntax_enabled,
};
use crate::bitstream::tile_payload::{
    BlockSize, FrameCdfSubset, TileBlockDecodedState, TileCdfSelector,
};
use crate::error::DecodeError;
use crate::prediction::inter::SPEC_MODE_INFO;
use crate::prediction::inter::{
    BawpSyntax, InterBlock, InterIntraPrediction, Mv, PlacedInterBlock,
    mc::{CompoundBlend, McBlockRect},
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn skip_mode_selects_the_upper_skip_txfm_context_bank() {
    assert_eq!(inter_skip_txfm_ctx(0, false), 0);
    assert_eq!(inter_skip_txfm_ctx(2, false), 2);
    assert_eq!(inter_skip_txfm_ctx(0, true), 3);
    assert_eq!(inter_skip_txfm_ctx(2, true), 5);
}

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
fn shared_inter_leaves_predict_chroma_before_the_offset_owner() {
    assert!(leaf_predicts_chroma(true, false));
    assert!(!leaf_predicts_chroma(true, true));
    assert!(!leaf_predicts_chroma(false, false));
}

#[test]
fn shared_chroma_size_disables_compound_prediction() -> TestResult {
    let block_8x8 = BlockSize::new(3)?;
    let block_16x16 = BlockSize::new(6)?;

    assert!(!sub8x8_chroma_disables_compound(block_8x8, block_8x8));
    assert!(sub8x8_chroma_disables_compound(block_8x8, block_16x16));
    Ok(())
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
fn regular_interintra_syntax_precedes_drl() -> TestResult {
    let bsize_group = super::SIZE_GROUP_LOOKUP[super::BLOCK_8X8];
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::InterIntra { bsize_group },
        1,
    )?;
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::InterIntraMode { bsize_group },
        3,
    )?;
    write_symbol(&mut tile, &mut encoder, TileCdfSelector::WedgeInterIntra, 0)?;
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::DrlMode { idx: 0, ctx: 1 },
        0,
    )?;
    let payload = encoder.finish()?.into_bytes();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )?;

    let interintra = read_inter_intra_syntax_enabled(
        &mut tile,
        &mut symbols,
        true,
        super::BLOCK_8X8,
        2,
        2,
        ByteOffset::new(0),
    )?;
    let drl = super::read_drl_idx(&mut tile, &mut symbols, 1, 3, ByteOffset::new(0))?;

    assert!(interintra.enabled);
    assert_eq!(interintra.mode, Some(3));
    assert!(!interintra.use_wedge);
    assert_eq!(drl, 0);
    Ok(())
}

#[test]
fn small_globalmv_blocks_still_signal_interp_filter() {
    assert!(super::single_inter_needs_interp_filter(
        1,
        2,
        super::SINGLE_MODE_GLOBALMV
    ));
    assert!(!super::single_inter_needs_interp_filter(
        2,
        2,
        super::SINGLE_MODE_GLOBALMV
    ));
    assert!(super::single_inter_needs_interp_filter(
        2,
        2,
        super::SINGLE_MODE_NEARMV
    ));
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

fn write_symbol(
    tile: &mut crate::bitstream::tile_payload::TileCdfSubset,
    encoder: &mut SymbolEncoder,
    selector: TileCdfSelector,
    value: u8,
) -> TestResult {
    tile.with_row_mut(selector, |row| {
        encoder.write_symbol(row, Symbol::new(value))
    })??;
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
        predict_chroma: false,
        chroma_first_reference_only: false,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: 0,
            ref_frame1: None,
            mv: Mv { row: 0, col: 0 },
            mv1: Mv { row: 0, col: 0 },
            interp: ReconInterpolationFilter::EightTap,
            warp_params: [None, None],
            bawp: BawpSyntax::default(),
            interintra: Some(InterIntraPrediction::SmoothMask { mode }),
            compound_blend: CompoundBlend::default(),
            optflow_distances: None,
            residual: None,
        },
    }
}

#[test]
fn offset_chroma_motion_compensation_stays_leaf_scoped() {
    let mut placed = placed_luma_block(16, 4, 8, 4, InterIntraMode::Dc);
    placed.chroma_luma_x = 16;
    placed.chroma_luma_y = 0;
    placed.chroma_luma_w = 8;
    placed.chroma_luma_h = 8;
    placed.predict_chroma = true;

    assert_eq!(
        placed.motion_compensation_rect(),
        McBlockRect {
            luma_x: 16,
            luma_y: 4,
            luma_w: 8,
            luma_h: 4,
            chroma_luma_x: 16,
            chroma_luma_y: 4,
            chroma_luma_w: 8,
            chroma_luma_h: 4,
        }
    );
}
