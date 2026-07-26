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

use super::interintra::InterIntraScratch;
use super::prediction::{leaf_predicts_chroma, sub8x8_chroma_disables_compound};
use super::warp::extend_warp_base_position;
use super::{
    chroma_smooth_tile_ranges, inter_skip_txfm_ctx, leaf_uses_general_intra,
    predict_interintra_planes, read_inter_intra_syntax_enabled, validate_segment_id,
};
use crate::bitstream::tile_payload::{
    BlockSize, FrameCdfSubset, TileBlockDecodedState, TileCdfSelector,
};
use crate::error::DecodeError;
use crate::prediction::inter::{
    BawpSyntax, InterBlock, InterIntraPrediction, Mv, PlacedInterBlock,
    find_mv_stack::{BlockPrecisionRecord, MvBlockContext, NeighbourMvGrid},
    mc::{CompoundBlend, McBlockRect},
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn extend_warp_base_rejects_tip_candidate_and_uses_matching_fallback() -> TestResult {
    let mut grid = NeighbourMvGrid::new(16, 16).ok_or("valid neighbour grid")?;
    grid.record_tip_block(
        8,
        0,
        8,
        8,
        false,
        Mv::ZERO,
        false,
        3,
        false,
        false,
        BlockPrecisionRecord::default(),
    );
    let block = MvBlockContext {
        mi_row: 8,
        mi_col: 8,
        bw4: 8,
        bh4: 8,
        sb_h4: 16,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: 16,
        mi_cols: 16,
    };

    assert_eq!(
        extend_warp_base_position(&grid, &block, (0, -1), Some((-1, 0))),
        Some((-1, 0))
    );
    grid.record_block(
        8,
        0,
        8,
        8,
        true,
        0,
        None,
        false,
        Mv { row: 8, col: 16 },
        false,
        3,
        false,
        BlockPrecisionRecord::default(),
    );
    assert_eq!(
        extend_warp_base_position(&grid, &block, (0, -1), Some((-1, 0))),
        Some((0, -1))
    );
    Ok(())
}

#[test]
fn skip_mode_selects_the_upper_skip_txfm_context_bank() {
    assert_eq!(inter_skip_txfm_ctx(0, false), 0);
    assert_eq!(inter_skip_txfm_ctx(2, false), 2);
    assert_eq!(inter_skip_txfm_ctx(0, true), 3);
    assert_eq!(inter_skip_txfm_ctx(2, true), 5);
}

#[test]
fn partitioned_leaves_route_to_general_intra_before_inter_syntax() {
    assert!(leaf_uses_general_intra(false, true, false));
    assert!(leaf_uses_general_intra(false, false, true));
    assert!(!leaf_uses_general_intra(false, false, false));
}

#[test]
fn segment_id_validation_accepts_the_last_active_segment() {
    assert!(matches!(
        validate_segment_id(7, 7, ByteOffset::new(11)),
        Ok(7)
    ));
}

#[test]
fn segment_id_validation_rejects_values_above_the_active_range() {
    assert!(matches!(
        validate_segment_id(8, 7, ByteOffset::new(11)),
        Err(DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == "inter_segment_id_out_of_range"
                && unsupported.spec_section() == "5.20.5.8"
                && unsupported.byte_offset() == Some(ByteOffset::new(11))
    ));
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
fn chroma_smooth_tile_ranges_follow_chroma_sampling() {
    assert_eq!(
        chroma_smooth_tile_ranges(0..17, 0..19, ChromaFormatIdc::Yuv420),
        (0..9, 0..10)
    );
    assert_eq!(
        chroma_smooth_tile_ranges(0..17, 0..19, ChromaFormatIdc::Yuv422),
        (0..17, 0..10)
    );
    assert_eq!(
        chroma_smooth_tile_ranges(0..17, 0..19, ChromaFormatIdc::Yuv444),
        (0..17, 0..19)
    );
    assert_eq!(
        chroma_smooth_tile_ranges(5..17, 7..19, ChromaFormatIdc::Yuv420),
        (2..9, 3..10)
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

    let mut scratch = InterIntraScratch::default();
    predict_interintra_planes(
        &mut scratch,
        &workspace,
        &placed,
        &block_decoded,
        InterIntraMode::Smooth,
        false,
        BitDepth::Eight,
        ByteOffset::new(0),
    )?;
    let planes = scratch.planes().collect::<Vec<_>>();

    assert_eq!(planes.len(), 1);
    assert_eq!(planes[0].0.plane, PlaneId::Y);
    assert_eq!(planes[0].1.len(), 64);
    assert!(
        planes[0]
            .1
            .iter()
            .all(|sample| (127..=129).contains(sample))
    );
    Ok(())
}

#[test]
fn interintra_chroma_planes_follow_non420_subsampling() -> TestResult {
    for (format, chroma_samples, chroma_subsampling) in [
        (PixelFormat::Yuv422, 32, (1, 0)),
        (PixelFormat::Yuv444, 64, (0, 0)),
    ] {
        let workspace = CurrentFrameWorkspace::<u8>::new(frame_info(8, 8, format)?, 128)?;
        let block_decoded = TileBlockDecodedState::new(1, 1, 1, 16, 16, 16)?;
        let mut placed = placed_luma_block(0, 0, 8, 8, InterIntraMode::Smooth);
        placed.interintra_chroma = true;

        let mut scratch = InterIntraScratch::default();
        predict_interintra_planes(
            &mut scratch,
            &workspace,
            &placed,
            &block_decoded,
            InterIntraMode::Smooth,
            false,
            BitDepth::Eight,
            ByteOffset::new(0),
        )?;
        let planes = scratch.planes().collect::<Vec<_>>();

        assert_eq!(planes.len(), 3);
        for (plane, samples) in &planes[1..] {
            assert_eq!((plane.sub_x, plane.sub_y), chroma_subsampling);
            assert_eq!(samples.len(), chroma_samples);
        }
    }
    Ok(())
}

#[test]
fn interintra_chroma_fallback_edges_use_each_planes_neighbour() -> TestResult {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(frame_info(8, 16, PixelFormat::Yuv420)?, 128)?;
    for (plane, y, value, width) in [
        (PlaneId::Y, 7, 10, 8),
        (PlaneId::U, 3, 20, 4),
        (PlaneId::V, 3, 30, 4),
    ] {
        workspace.fill_rect(plane, PlaneRect::new(0, y, width, 1)?, value)?;
    }
    let block_decoded = TileBlockDecodedState::new(1, 1, 1, 16, 16, 16)?;
    let mut placed = placed_luma_block(0, 8, 8, 8, InterIntraMode::Horizontal);
    placed.interintra_chroma = true;
    let mut scratch = InterIntraScratch::default();

    predict_interintra_planes(
        &mut scratch,
        &workspace,
        &placed,
        &block_decoded,
        InterIntraMode::Horizontal,
        false,
        BitDepth::Eight,
        ByteOffset::new(0),
    )?;
    let planes = scratch.planes().collect::<Vec<_>>();

    assert_eq!(planes.len(), 3);
    for ((plane, samples), expected) in planes.iter().zip([10, 20, 30]) {
        assert!(
            samples.iter().all(|sample| *sample == expected),
            "{plane:?}"
        );
    }
    Ok(())
}

#[test]
fn interintra_scratch_reuses_pixel_and_fallback_edge_storage() -> TestResult {
    let workspace = CurrentFrameWorkspace::<u8>::new(monochrome_info(8, 8)?, 128)?;
    let block_decoded = TileBlockDecodedState::new(1, 1, 1, 16, 16, 16)?;
    let placed = placed_luma_block(0, 0, 8, 8, InterIntraMode::Vertical);
    let mut scratch = InterIntraScratch::default();

    for _ in 0..2 {
        predict_interintra_planes(
            &mut scratch,
            &workspace,
            &placed,
            &block_decoded,
            InterIntraMode::Vertical,
            false,
            BitDepth::Eight,
            ByteOffset::new(0),
        )?;
        assert!(
            scratch
                .storage_identity()
                .iter()
                .all(|(_, capacity)| *capacity > 0)
        );
    }
    let warmed = scratch.storage_identity();
    predict_interintra_planes(
        &mut scratch,
        &workspace,
        &placed,
        &block_decoded,
        InterIntraMode::Vertical,
        false,
        BitDepth::Eight,
        ByteOffset::new(0),
    )?;

    assert_eq!(scratch.storage_identity(), warmed);
    Ok(())
}

fn write_symbol(
    tile: &mut crate::bitstream::tile_payload::TileCdfSubset,
    encoder: &mut SymbolEncoder,
    selector: TileCdfSelector,
    value: u8,
) -> TestResult {
    tile.with_row_mut(selector, |row| {
        encoder.write_symbol_u16(row, Symbol::new(value))
    })??;
    Ok(())
}

fn monochrome_info(width: usize, height: usize) -> splot_recon::Result<DecodedFrameInfo> {
    frame_info(width, height, PixelFormat::Monochrome)
}

fn frame_info(
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
) -> splot_recon::Result<DecodedFrameInfo> {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
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
        sub8x8_chroma: false,
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
