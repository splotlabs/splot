// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::test_support::{yuv420_workspace, yuv420_workspace_with};

static EMPTY_CHROMA_RECORDS: ChromaDeblockRecords = ChromaDeblockRecords::new();

const fn prediction(r: usize, c: usize, tx: usize) -> DeblockPredictionUnit {
    DeblockPredictionUnit {
        base_r: r,
        base_c: c,
        default_sub_pu_tx: tx,
    }
}

fn with_plane_ctx<T: ReconSample, R>(
    ws: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    f: impl FnOnce(&mut PlaneCtx<'_, '_, T>) -> R,
) -> R {
    let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
    let mut frame = ws.as_frame_mut();
    let view = frame.plane_mut(plane).unwrap();
    let stride = view.stride_samples();
    let mut band = PlaneBand::plane(view.samples_mut(), stride, width, height);
    let mut ctx = PlaneCtx::new(&mut band).unwrap();
    f(&mut ctx)
}

fn deblock_blocks(mi_rows: usize, mi_cols: usize) -> Vec<DeblockBlock> {
    let mut blocks = Vec::new();
    for r in (0..mi_rows).step_by(8) {
        for c in (0..mi_cols).step_by(8) {
            blocks.push(DeblockBlock {
                r,
                c,
                luma_prediction: prediction(r, c, 3),
                chroma_prediction: prediction(r, c, 2),
                chroma_base_r: r,
                chroma_base_c: c,
                n4w: 8,
                n4h: 8,
                luma_tx: 3,
                chroma_tx: Some(2),
                sub_pu_size: None,
                chroma_transform_only: false,
                qindex: 100,
                skip: false,
                lossless: false,
            });
        }
    }
    blocks
}

const fn filter(apply_deblocking_filter: [bool; 4]) -> DeblockingFilterParams {
    DeblockingFilterParams::new(apply_deblocking_filter, [false; 4], [0; 4])
}

fn fill_rect(
    ws: &mut CurrentFrameWorkspace<u8>,
    plane: PlaneId,
    x_range: core::ops::Range<usize>,
    y_range: core::ops::Range<usize>,
    sample: u8,
) {
    for y in y_range {
        for x in x_range.clone() {
            ws.set_reconstructed_sample(plane, x, y, sample).unwrap();
        }
    }
}

fn run_deblock(
    ws: &mut CurrentFrameWorkspace<u8>,
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
    apply_deblocking_filter: [bool; 4],
) {
    let mut filter = filter(apply_deblocking_filter);
    filter.allow_df_sub_pu = blocks.iter().any(|block| block.sub_pu_size.is_some());
    deblock_general_intra_frame(
        ws,
        blocks,
        mi_rows,
        mi_cols,
        filter,
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
}

#[test]
fn one_row_advance_matches_the_whole_frame_deblock() {
    let mi_rows = 8;
    let mi_cols = 16;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let make_workspace = || {
        let mut workspace = yuv420_workspace(64, 32, 0);
        fill_rect(&mut workspace, PlaneId::Y, 0..32, 0..32, 100);
        fill_rect(&mut workspace, PlaneId::Y, 32..64, 0..32, 108);
        workspace
    };
    let mut combined = make_workspace();
    let mut staged = make_workspace();
    let params = filter([true; 4]);

    deblock_general_intra_frame(
        &mut combined,
        &blocks,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
    let mut plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    plan.advance(&mut staged, mi_rows, BitDepth::Eight).unwrap();
    assert!(plan.finish().is_none());

    let combined =
        splot_recon::DecodedFrameHashInput::new(&combined.freeze().unwrap()).compute_hash();
    let staged = splot_recon::DecodedFrameHashInput::new(&staged.freeze().unwrap()).compute_hash();
    assert_eq!(staged, combined);
}

fn patterned_yuv420_workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u8> {
    let mut workspace = yuv420_workspace(width, height, 0);
    fill_pattern(&mut workspace);
    workspace
}

fn patterned_yuv420_workspace_with_visible_height(
    width: usize,
    height: usize,
    visible_height: usize,
) -> CurrentFrameWorkspace<u8> {
    let info = splot_recon::DecodedFrameInfo::new(
        splot_recon::OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        splot_recon::PlaneSize::new(width, height).unwrap(),
        splot_recon::PlaneRect::new(0, 0, width, visible_height).unwrap(),
    )
    .unwrap();
    let mut workspace = CurrentFrameWorkspace::new(info, 0).unwrap();
    fill_pattern(&mut workspace);
    workspace
}

fn fill_pattern(workspace: &mut CurrentFrameWorkspace<u8>) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let (plane_width, plane_height) = coded_plane_dimensions(workspace, plane).unwrap();
        let shift = usize::from(plane != PlaneId::Y);
        for y in 0..plane_height {
            for x in 0..plane_width {
                let band = ((x / (32 >> shift)) + (y / (32 >> shift))) & 1;
                workspace
                    .set_reconstructed_sample(plane, x, y, 100 + 8 * band as u8)
                    .unwrap();
            }
        }
    }
}

fn assert_workspace_samples_eq(
    actual: &CurrentFrameWorkspace<u8>,
    expected: &CurrentFrameWorkspace<u8>,
) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert_eq!(
            actual.plane(plane).unwrap().samples(),
            expected.plane(plane).unwrap().samples(),
            "{plane:?} samples differ"
        );
    }
}

fn two_by_two_tile_info() -> TileInfo {
    use splot_core::bitio::BitReader;
    use splot_core::headers::frame::{CoreSeqTileView, FrameSize, parse_tile_info};
    use splot_core::headers::sequence::{LevelIdx, SuperblockSize, Tier};
    use splot_core::span::ByteOffset;
    use splot_core::tile::TileParams;

    let view = CoreSeqTileView {
        seq_tile_info_present_flag: true,
        allow_tile_info_change: false,
        seq_tile_params: Some(TileParams {
            tile_cols: 2,
            tile_rows: 2,
            tile_cols_log2: 1,
            tile_rows_log2: 1,
            sb_cols: 2,
            sb_rows: 2,
            uniform_spacing: true,
            covers_cols: true,
            covers_rows: true,
        }),
        seq_sb_col_starts: Vec::new(),
        seq_sb_row_starts: Vec::new(),
        seq_sb_size: SuperblockSize::Block64x64,
        use_256x256_superblock: false,
        use_128x128_superblock: false,
        enable_avg_cdf: true,
        avg_cdf_type: 1,
        seq_tier: Tier::Main,
        seq_level_idx: LevelIdx::from_bits(0),
    };
    let data = [0_u8];
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    parse_tile_info(
        &mut reader,
        &view,
        FrameSize::new(128, 128),
        true,
        false,
        false,
    )
    .unwrap()
}

#[test]
fn incremental_deblock_matches_whole_frame_across_superblock_rows_and_chroma() {
    let mi_rows = 32;
    let mi_cols = 32;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let mut expected = patterned_yuv420_workspace(128, 128);
    let mut actual = patterned_yuv420_workspace(128, 128);

    deblock_general_intra_frame(
        &mut expected,
        &blocks,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
    let mut plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    for mi_row_end in [16, 32] {
        plan.advance(&mut actual, mi_row_end, BitDepth::Eight)
            .unwrap();
    }

    assert_eq!(plan.final_luma_rows(1), 128);
    assert!(plan.finish().is_none());
    assert_workspace_samples_eq(&actual, &expected);
}

#[test]
fn contiguous_source_deblock_matches_workspace_while_an_older_lease_is_live() {
    let mi_rows = 32;
    let mi_cols = 32;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let mut expected = patterned_yuv420_workspace(128, 128);
    let mut source =
        crate::filters::source::DeblockedSource::new(patterned_yuv420_workspace(128, 128));

    deblock_general_intra_frame(
        &mut expected,
        &blocks,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
    let mut plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    plan.advance_source(&mut source, 16, BitDepth::Eight)
        .unwrap();
    let earlier = source.lease(0, 32, 8).unwrap();
    plan.advance_source(&mut source, 32, BitDepth::Eight)
        .unwrap();

    assert!(earlier.planes().is_some());
    let actual = source.lease(0, 128, 0).unwrap();
    let actual = actual.planes().unwrap();
    for (plane, actual) in [
        (PlaneId::Y, Some(actual.y)),
        (PlaneId::U, actual.u),
        (PlaneId::V, actual.v),
    ] {
        let actual = actual.unwrap();
        let expected = expected.plane(plane).unwrap();
        let width = expected.storage_size().width();
        let stride = expected.stride_samples();
        for y in 0..expected.storage_size().height() {
            assert_eq!(
                actual.row(y).unwrap(),
                &expected.samples()[y * stride..y * stride + width],
                "{plane:?} row {y} differs"
            );
        }
    }
    assert!(plan.finish().is_none());
}

#[test]
fn owned_deblock_records_match_borrowed_plan_and_return_on_finish() {
    let mi_rows = 32;
    let mi_cols = 32;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let mut borrowed = patterned_yuv420_workspace(128, 128);
    let mut owned = patterned_yuv420_workspace(128, 128);
    let mut borrowed_plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    let owned_blocks = blocks.clone();
    let owned_pointer = owned_blocks.as_ptr();
    let owned_capacity = owned_blocks.capacity();
    let fixture = include_bytes!(
        "../../../../tests/conformance/vectors/valid/\
         syn-2frame-multirow-inter-64x256-10bit-q100.ivf"
    );
    let (_, core) = crate::prediction::inter::test_support::fixture_sequence_and_key_core(fixture);
    let mut owned_plan = FrameDeblock::prepare_owned(
        OwnedDeblockRecords {
            blocks: owned_blocks,
            chroma: ChromaDeblockRecords::default(),
        },
        mi_rows,
        mi_cols,
        params,
        Arc::new(core),
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();

    for mi_row_end in [16, 32] {
        borrowed_plan
            .advance(&mut borrowed, mi_row_end, BitDepth::Eight)
            .unwrap();
        owned_plan
            .advance(&mut owned, mi_row_end, BitDepth::Eight)
            .unwrap();
    }

    assert_workspace_samples_eq(&owned, &borrowed);
    assert!(borrowed_plan.finish().is_none());
    let records = owned_plan.finish().unwrap();
    assert_eq!(records.blocks.len(), blocks.len());
    assert_eq!(records.blocks.as_ptr(), owned_pointer);
    assert_eq!(records.blocks.capacity(), owned_capacity);
    assert!(records.chroma.is_empty());
}

#[test]
fn incremental_deblock_enforces_frontiers_and_extracts_exact_owned_window() {
    let mi_rows = 32;
    let mi_cols = 32;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let mut workspace = patterned_yuv420_workspace(128, 128);
    let mut plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();

    assert_eq!(plan.final_luma_rows(1), 0);
    plan.advance(&mut workspace, 16, BitDepth::Eight).unwrap();
    assert_eq!(plan.final_luma_rows(0), 56);
    assert_eq!(plan.final_luma_rows(1), 48);

    let window = plan.extract_window(&workspace, 0, 32, 8).unwrap();
    let planes = window.planes().unwrap();
    assert_eq!((planes.y.origin_y(), planes.y.end_y()), (0, 40));
    assert_eq!(
        (planes.u.unwrap().origin_y(), planes.u.unwrap().end_y()),
        (0, 24)
    );
    assert!(matches!(
        plan.extract_window(&workspace, 0, 33, 8),
        Err(DeblockError::Workspace)
    ));
    assert!(plan.finish().is_none());
}

#[test]
fn incremental_deblock_clamps_completed_window_to_clipped_frame_height() {
    let mi_rows = 18;
    let mi_cols = 16;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let mut expected = patterned_yuv420_workspace_with_visible_height(64, 72, 70);
    let mut actual = patterned_yuv420_workspace_with_visible_height(64, 72, 70);

    deblock_general_intra_frame(
        &mut expected,
        &blocks,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
    let mut plan = FrameDeblock::prepare(
        &blocks,
        &EMPTY_CHROMA_RECORDS,
        mi_rows,
        mi_cols,
        params,
        None,
        false,
        DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    plan.advance(&mut actual, mi_rows, BitDepth::Eight).unwrap();

    assert_eq!(plan.final_luma_rows(1), 72);
    let window = plan.extract_window(&actual, 0, 70, 16).unwrap();
    assert_eq!(window.planes().unwrap().y.end_y(), 72);
    assert!(matches!(
        plan.extract_window(&actual, 0, 73, 16),
        Err(DeblockError::Workspace)
    ));
    assert!(plan.finish().is_none());
    assert_workspace_samples_eq(&actual, &expected);
}

#[test]
fn incremental_deblock_matches_tile_boundary_rules() {
    let mi_rows = 32;
    let mi_cols = 32;
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let params = filter([true; 4]);
    let tile_info = two_by_two_tile_info();
    assert_eq!(tile_info.mi_col_starts, [0, 16, 32]);
    assert_eq!(tile_info.mi_row_starts, [0, 16, 32]);

    for disable_loopfilters_across_tiles in [false, true] {
        let make_workspace = || {
            let mut workspace = yuv420_workspace(128, 128, 100);
            fill_rect(&mut workspace, PlaneId::Y, 64..128, 0..128, 108);
            workspace
        };
        let mut expected = make_workspace();
        let mut actual = make_workspace();
        deblock_general_intra_frame(
            &mut expected,
            &blocks,
            mi_rows,
            mi_cols,
            params,
            Some(&tile_info),
            disable_loopfilters_across_tiles,
            DeblockQuantDeltas::ZERO,
            BitDepth::Eight,
        )
        .unwrap();
        let mut plan = FrameDeblock::prepare(
            &blocks,
            &EMPTY_CHROMA_RECORDS,
            mi_rows,
            mi_cols,
            params,
            Some(&tile_info),
            disable_loopfilters_across_tiles,
            DeblockQuantDeltas::ZERO,
        )
        .unwrap()
        .unwrap();
        plan.advance(&mut actual, mi_rows, BitDepth::Eight).unwrap();
        assert!(plan.finish().is_none());
        assert_workspace_samples_eq(&actual, &expected);

        let p0 = actual.reconstructed_sample(PlaneId::Y, 63, 16).unwrap();
        let q0 = actual.reconstructed_sample(PlaneId::Y, 64, 16).unwrap();
        if disable_loopfilters_across_tiles {
            assert_eq!((p0, q0), (100, 108));
        } else {
            assert_smoothed_step(p0, q0, "cross-tile edge must filter");
        }
    }
}

#[test]
fn tip_filter_widths_follow_unit_and_chroma_superblock_edges() {
    assert_eq!(deblock_filter_max_width(8, false, false), (3, 3));
    assert_eq!(deblock_filter_max_width(16, false, true), (6, 6));
    assert_eq!(deblock_filter_max_width(4, true, true), (1, 1));
    assert_eq!(deblock_filter_max_width(8, true, true), (2, 3));
    assert_eq!(deblock_filter_max_width(8, true, false), (3, 3));
    assert_eq!(deblock_filter_max_width(32, false, false), (8, 8));
    assert_eq!(deblock_filter_max_width(32, true, true), (2, 4));
}

#[test]
fn sub_pu_filter_dimensions_follow_filt_max_size() {
    assert_eq!(sub_pu_filter_dimension(64, 8, true), (8, true));
    assert_eq!(sub_pu_filter_dimension(8, 8, false), (4, true));
    assert_eq!(sub_pu_filter_dimension(16, 16, false), (8, true));
    assert_eq!(sub_pu_filter_dimension(4, 8, false), (4, false));
}

#[test]
fn explicit_sub_pu_dimensions_follow_pass_and_chroma_subsampling() {
    let mut blocks = deblock_blocks(1, 1);
    blocks[0].sub_pu_size = Some(DeblockSubPuSize::square(8));
    let info = EdgeBlock {
        block: &blocks[0],
        chroma_transform: None,
    };
    assert_eq!(sub_pu_dimension(info, 1, 0, 1, 1), 4);
    assert_eq!(sub_pu_dimension(info, 1, 1, 1, 1), 4);

    blocks[0].sub_pu_size = Some(DeblockSubPuSize::new(16, 8));
    let info = EdgeBlock {
        block: &blocks[0],
        chroma_transform: None,
    };
    assert_eq!(sub_pu_dimension(info, 1, 0, 1, 1), 8);
    assert_eq!(sub_pu_dimension(info, 1, 1, 1, 1), 4);
}

#[test]
fn tip_deblocking_smooths_prediction_unit_boundaries() {
    let mut ws = yuv420_workspace(32, 32, 0);
    fill_rect(&mut ws, PlaneId::Y, 0..16, 0..32, 100);
    fill_rect(&mut ws, PlaneId::Y, 16..32, 0..32, 108);

    deblock_tip_frame(
        &mut ws,
        16,
        QuantizationParams::inferred_tip(100, 0, 0),
        0,
        None,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    assert_smoothed_step(
        ws.reconstructed_sample(PlaneId::Y, 15, 8).unwrap(),
        ws.reconstructed_sample(PlaneId::Y, 16, 8).unwrap(),
        "TIP prediction-unit boundary must be filtered",
    );
}

#[test]
fn tip_tile_edges_map_subsampled_plane_coordinates_to_luma_mi() {
    let col_starts = [0, 16, 32];
    let row_starts = [0, 8, 16];
    assert!(tip_tile_edge(Some(&col_starts), 64, 0));
    assert!(tip_tile_edge(Some(&col_starts), 32, 1));
    assert!(tip_tile_edge(Some(&row_starts), 32, 0));
    assert!(tip_tile_edge(Some(&row_starts), 16, 1));
    assert!(!tip_tile_edge(Some(&col_starts), 16, 0));
    assert!(!tip_tile_edge(None, 64, 0));
}

#[test]
fn tip_deblocking_obeys_cross_tile_filtering_flag() {
    let col_starts = [0, 16, 32];
    let row_starts = [0, 8, 16];
    let run = |disable_loopfilters_across_tiles| {
        let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
            128,
            64,
            BitDepth::Eight,
            PixelFormat::Yuv420,
        )
        .unwrap();
        fill_rect(&mut workspace, PlaneId::Y, 0..64, 0..64, 100);
        fill_rect(&mut workspace, PlaneId::Y, 64..128, 0..64, 108);
        deblock_tip_frame(
            &mut workspace,
            16,
            QuantizationParams::inferred_tip(100, 0, 0),
            0,
            Some((&col_starts, &row_starts)),
            disable_loopfilters_across_tiles,
            BitDepth::Eight,
        )
        .unwrap();
        workspace
    };

    let disabled = run(true);
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 63, 8), Ok(100));
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 64, 8), Ok(108));

    let enabled = run(false);
    assert_smoothed_step(
        enabled.reconstructed_sample(PlaneId::Y, 63, 8).unwrap(),
        enabled.reconstructed_sample(PlaneId::Y, 64, 8).unwrap(),
        "TIP tile edge filters when cross-tile loop filtering is enabled",
    );
}

#[test]
fn tip_deblocking_handles_yuv422_and_yuv444_chroma_geometry() {
    for (pixel_format, chroma_width, boundary) in
        [(PixelFormat::Yuv422, 16, 8), (PixelFormat::Yuv444, 32, 16)]
    {
        let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
            32,
            32,
            BitDepth::Eight,
            pixel_format,
        )
        .unwrap();
        fill_rect(&mut workspace, PlaneId::U, 0..boundary, 0..32, 100);
        fill_rect(
            &mut workspace,
            PlaneId::U,
            boundary..chroma_width,
            0..32,
            108,
        );
        deblock_tip_frame(
            &mut workspace,
            16,
            QuantizationParams::inferred_tip(100, 0, 0),
            0,
            None,
            false,
            BitDepth::Eight,
        )
        .unwrap();
        assert_smoothed_step(
            workspace
                .reconstructed_sample(PlaneId::U, boundary - 1, 8)
                .unwrap(),
            workspace
                .reconstructed_sample(PlaneId::U, boundary, 8)
                .unwrap(),
            "TIP deblocking must use the coded chroma plane geometry",
        );
    }
}

fn edge_test_grid(curr_skip: bool) -> MiGrid<'static> {
    edge_test_grid_with_metadata(curr_skip, false)
}

fn edge_test_grid_with_metadata(curr_skip: bool, prediction_boundary: bool) -> MiGrid<'static> {
    let qindex = if prediction_boundary { 215 } else { 100 };
    let blocks = Box::leak(Box::new([
        DeblockBlock {
            r: 0,
            c: if prediction_boundary { 0 } else { 2 },
            luma_prediction: prediction(0, 2, 3),
            chroma_prediction: prediction(0, 2, 3),
            chroma_base_r: 0,
            chroma_base_c: 2,
            n4w: 1,
            n4h: 1,
            luma_tx: 3,
            chroma_tx: None,
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex,
            skip: false,
            lossless: false,
        },
        DeblockBlock {
            r: 0,
            c: 0,
            luma_prediction: prediction(0, 0, 3),
            chroma_prediction: prediction(0, 0, 3),
            chroma_base_r: 0,
            chroma_base_c: 0,
            n4w: 1,
            n4h: 1,
            luma_tx: 3,
            chroma_tx: None,
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex,
            skip: curr_skip,
            lossless: false,
        },
    ]));
    let mut cells = vec![MiCell::default(); 4 * 16];
    cells[4].base = 0;
    cells[5].base = 1;
    let storage = Box::leak(Box::new(MiGridStorage {
        mi_cols: 16,
        fully_covered: false,
        cells,
        candidates: vec![0; 4 * 16],
    }));
    MiGrid::new(storage, None, blocks, &EMPTY_CHROMA_RECORDS)
}

fn assert_candidate_mask_superset(
    grid: &MiGrid,
    mi_rows: usize,
    mi_cols: usize,
    plane: usize,
    pass: usize,
    sub_x: usize,
    sub_y: usize,
) {
    let (dx, dy) = if pass == 0 { (1, 0) } else { (0, 1) };
    let row_step = 1 << sub_y;
    let col_step = 1 << sub_x;
    for row in (0..mi_rows).step_by(row_step) {
        for col in (0..mi_cols).step_by(col_step) {
            if pass == 0 && col == 0 || pass == 1 && row == 0 {
                continue;
            }
            let prev_row = row - (dy << sub_y);
            let prev_col = col - (dx << sub_x);
            let curr = grid.get_edge(row, col).unwrap();
            let prev = grid.get_edge(prev_row, prev_col).unwrap();
            let curr_tx_base = curr.tx_base(plane);
            let prev_tx_base = prev.tx_base(plane);
            let x_p = (col * MI_SIZE) >> sub_x;
            let y_p = (row * MI_SIZE) >> sub_y;
            let curr_sub_pu = sub_pu_base(curr, plane, x_p, y_p, sub_x, sub_y);
            let prev_sub_pu = sub_pu_base(
                prev,
                plane,
                x_p.saturating_sub(dx),
                y_p.saturating_sub(dy),
                sub_x,
                sub_y,
            );
            if curr_tx_base != prev_tx_base || curr_sub_pu != prev_sub_pu {
                assert!(
                    grid.is_candidate(row, col, pass, true, sub_x, sub_y),
                    "candidate mask missed plane={plane} pass={pass} row={row} col={col}"
                );
            }
        }
    }
}

#[test]
fn candidate_mask_is_a_superset_for_mixed_transform_and_sub_pu_edges() {
    let block = |r, c, n4w, n4h, sub_pu_size| DeblockBlock {
        r,
        c,
        luma_prediction: prediction(r, c, 3),
        chroma_prediction: prediction(r, c, 2),
        chroma_base_r: r,
        chroma_base_c: c,
        n4w,
        n4h,
        luma_tx: 3,
        chroma_tx: Some(2),
        sub_pu_size,
        chroma_transform_only: false,
        qindex: 100,
        skip: false,
        lossless: false,
    };
    let blocks = [
        block(0, 0, 1, 1, None),
        block(0, 1, 5, 1, None),
        block(1, 0, 1, 3, None),
        block(1, 1, 2, 3, Some(DeblockSubPuSize::square(8))),
        block(1, 3, 3, 3, None),
    ];
    let storage = build_mi_grid(&blocks, 4, 6).unwrap();
    let grid = MiGrid::new(&storage, None, &blocks, &EMPTY_CHROMA_RECORDS);

    for (plane, sub_x, sub_y) in [(0, 0, 0), (1, 1, 1)] {
        for pass in 0..2 {
            assert_candidate_mask_superset(&grid, 4, 6, plane, pass, sub_x, sub_y);
        }
    }
    assert!(!grid.is_candidate(3, 5, 0, false, 0, 0));
    assert!(!grid.is_candidate(3, 5, 1, false, 0, 0));
}

fn assert_smoothed_step(p0: u8, q0: u8, reason: &str) {
    assert!(
        (100..=108).contains(&p0) && (100..=108).contains(&q0),
        "smoothing stays within the step band: p0={p0} q0={q0}"
    );
    assert!(p0 > 100 || q0 < 108, "{reason}: p0={p0} q0={q0}");
}

#[test]
fn skipped_block_filters_internal_prediction_unit_edges() {
    let mut ws = yuv420_workspace(32, 8, 100);
    fill_rect(&mut ws, PlaneId::Y, 8..32, 0..8, 108);
    let block = DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction(0, 0, 3),
        chroma_prediction: prediction(0, 0, 2),
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 8,
        n4h: 2,
        luma_tx: 3,
        chroma_tx: Some(2),
        sub_pu_size: Some(DeblockSubPuSize::square(8)),
        chroma_transform_only: false,
        qindex: 215,
        skip: true,
        lossless: false,
    };

    run_deblock(&mut ws, &[block], 2, 8, [true, false, false, false]);

    assert_smoothed_step(
        ws.reconstructed_sample(PlaneId::Y, 7, 0).unwrap(),
        ws.reconstructed_sample(PlaneId::Y, 8, 0).unwrap(),
        "internal prediction-unit boundary must be filtered",
    );
}

#[test]
fn prediction_unit_geometry_caps_filter_width_at_block_edges() {
    let mut ws = yuv420_workspace(128, 8, 100);
    fill_rect(&mut ws, PlaneId::Y, 64..128, 0..8, 108);
    let block = |c| DeblockBlock {
        r: 0,
        c,
        luma_prediction: prediction(0, c, 4),
        chroma_prediction: prediction(0, c, 2),
        chroma_base_r: 0,
        chroma_base_c: c,
        n4w: 16,
        n4h: 2,
        luma_tx: 4,
        chroma_tx: Some(2),
        sub_pu_size: Some(DeblockSubPuSize::square(8)),
        chroma_transform_only: false,
        qindex: 215,
        skip: true,
        lossless: false,
    };

    run_deblock(
        &mut ws,
        &[block(0), block(16)],
        2,
        32,
        [true, false, false, false],
    );

    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 60, 0), Ok(100));
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 67, 0), Ok(108));
    assert_smoothed_step(
        ws.reconstructed_sample(PlaneId::Y, 63, 0).unwrap(),
        ws.reconstructed_sample(PlaneId::Y, 64, 0).unwrap(),
        "prediction-unit geometry must cap block-edge filtering",
    );
}

fn yuv420_workspace_10bit(width: usize, height: usize, fill: u16) -> CurrentFrameWorkspace<u16> {
    yuv420_workspace_with(BitDepth::Ten, width, height, fill)
}

fn splat_asymmetric<T: ReconSample>(
    ws: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    max_sample: u16,
) {
    let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let coords = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    for (x, y) in coords {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let value = ((state >> 33) as u16) % (max_sample + 1);
        ws.set_reconstructed_sample(plane, x, y, T::try_from_u16(value).unwrap())
            .unwrap();
    }
}

fn reference_gather<T: ReconSample>(
    ws: &CurrentFrameWorkspace<T>,
    plane: PlaneId,
    perp: PerpLine,
) -> Vec<T> {
    let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
    let max_x = width.saturating_sub(1) as isize;
    let max_y = height.saturating_sub(1) as isize;
    (0..2 * GATHER_HALF)
        .map(|idx| {
            let offset = idx as isize - GATHER_HALF as isize;
            let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
            let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
            ws.reconstructed_sample(plane, sx, sy).unwrap()
        })
        .collect()
}

fn reference_apply<T: ReconSample>(
    ws: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    perp: PerpLine,
    params: DeblockSampleFilter,
) {
    let before = reference_gather(ws, plane, perp);
    let mut line = before.clone();
    deblock_sample_filter(&mut line, &params).unwrap();
    let changed: Vec<(usize, T)> = line
        .iter()
        .zip(before.iter())
        .enumerate()
        .filter(|(_, (new, old))| new.to_u16() != old.to_u16())
        .map(|(idx, (&new, _))| (idx, new))
        .collect();
    for (idx, new) in changed {
        let (fx, fy) = perp
            .offset(idx as isize - params.boundary as isize)
            .unwrap();
        ws.set_reconstructed_sample(plane, fx, fy, new).unwrap();
    }
}

fn edge_and_corner_perps(width: usize, height: usize) -> Vec<PerpLine> {
    let mut perps = Vec::new();
    for &(x, y) in &[
        (GATHER_HALF, GATHER_HALF),
        (4, 0),
        (width - 4, height - 1),
        (width / 2, height / 2),
        (0, height - 4),
        (width - 1, 4),
    ] {
        perps.push(PerpLine::new(x, y, 1, 0));
        perps.push(PerpLine::new(x, y, 0, 1));
    }
    perps
}

fn assert_gather_and_apply_match_accessor_reference<T: ReconSample + core::fmt::Debug + Eq>(
    mut direct: CurrentFrameWorkspace<T>,
    mut reference: CurrentFrameWorkspace<T>,
    bit_depth: BitDepth,
) {
    let plane = PlaneId::Y;
    let max_sample = bit_depth.max_sample();
    splat_asymmetric(&mut direct, plane, max_sample);
    splat_asymmetric(&mut reference, plane, max_sample);
    let (width, height) = coded_plane_dimensions(&direct, plane).unwrap();

    for perp in edge_and_corner_perps(width, height) {
        let got = with_plane_ctx(&mut direct, plane, |ctx| gather_line(ctx, perp));
        assert_eq!(
            got.to_vec(),
            reference_gather(&reference, plane, perp),
            "gather at ({}, {}) d=({}, {})",
            perp.x,
            perp.y,
            perp.dx,
            perp.dy
        );
    }

    let params = DeblockSampleFilter {
        boundary: GATHER_HALF,
        q_thr: 60,
        max_width_neg: 4,
        max_width_pos: 4,
        q_thresh_mult: 25,
        w_mult_neg: 28,
        w_mult_pos: 28,
        prev_lossless: false,
        curr_lossless: false,
        bit_depth,
    };
    for &(x, y, dx, dy) in &[
        (GATHER_HALF, 2usize, 1usize, 0usize),
        (width / 2, height / 2, 1, 0),
        (width / 2, height / 2, 0, 1),
        (width - GATHER_HALF, height - 3, 1, 0),
        (4, GATHER_HALF, 0, 1),
    ] {
        let perp = PerpLine::new(x, y, dx, dy);
        with_plane_ctx(&mut direct, plane, |ctx| {
            apply_sample_filter(ctx, perp, params).unwrap();
        });
        reference_apply(&mut reference, plane, perp, params);
    }

    let batched_params = [
        params,
        DeblockSampleFilter {
            max_width_neg: 2,
            max_width_pos: 4,
            prev_lossless: true,
            ..params
        },
        DeblockSampleFilter {
            max_width_neg: 4,
            max_width_pos: 2,
            curr_lossless: true,
            ..params
        },
        DeblockSampleFilter {
            prev_lossless: true,
            curr_lossless: true,
            ..params
        },
    ];
    for params in batched_params {
        for &(x, y, dx, dy, lanes) in &[
            (16usize, 4usize, 1usize, 0usize, MI_SIZE),
            (4, 11, 0, 1, MI_SIZE),
            (16, 12, 1, 0, 2),
            (20, 11, 0, 1, 3),
        ] {
            let perp = PerpLine::new(x, y, dx, dy);
            with_plane_ctx(&mut direct, plane, |ctx| {
                apply_edge_samples(ctx, perp, lanes, params).unwrap();
            });
            for lane in 0..lanes {
                reference_apply(
                    &mut reference,
                    plane,
                    PerpLine::new(x + dy * lane, y + dx * lane, dx, dy),
                    params,
                );
            }
        }
    }
    assert_eq!(
        direct.samples(plane).unwrap(),
        reference.samples(plane).unwrap(),
        "direct-slice apply must match the accessor-based reference"
    );
}

#[test]
fn gather_and_apply_match_accessor_reference_8bit() {
    assert_gather_and_apply_match_accessor_reference(
        yuv420_workspace(34, 22, 0),
        yuv420_workspace(34, 22, 0),
        BitDepth::Eight,
    );
}

#[test]
fn gather_and_apply_match_accessor_reference_10bit() {
    assert_gather_and_apply_match_accessor_reference(
        yuv420_workspace_10bit(34, 22, 0),
        yuv420_workspace_10bit(34, 22, 0),
        BitDepth::Ten,
    );
}

#[test]
fn strength_cache_matches_direct_computation() {
    for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
        for &(quant_delta, df_delta_q) in &[(0i32, 0i32), (-6, 3), (12, -2)] {
            let cache = StrengthCache::new(quant_delta, df_delta_q, bit_depth);
            for qindex in (0u32..=300).chain([301, 302, 303, 304, 1000, u32::MAX]) {
                let direct = adaptive_strength(
                    deblock_level(qindex, quant_delta, df_delta_q, bit_depth),
                    bit_depth,
                );
                assert_eq!(cache.get(qindex), direct, "first lookup qindex={qindex}");
                assert_eq!(cache.get(qindex), direct, "cached lookup qindex={qindex}");
            }
        }
    }
}

#[test]
fn q_clamped_zero_delta_matches_spec() {
    for q in [0u32, 1, 100, 255] {
        assert_eq!(q_clamped(q, 0, BitDepth::Eight), q, "q_clamped({q}, 0)");
    }
}

#[test]
fn adaptive_strength_for_lvl_100_8bit() {
    let (q_thr, side) = adaptive_strength(100, BitDepth::Eight);
    assert_eq!(side, 1, "side threshold for lvl 100 (8-bit)");
    assert!(q_thr > 0, "qThr must be positive for a nonzero level");
}

#[test]
fn combine_strengths_averages_then_maxes() {
    assert_eq!(
        combine_strengths(3, 5, 2, 4),
        ((3 + 5 + 1) >> 1, (2 + 4 + 1) >> 1)
    );
    assert_eq!(combine_strengths(0, 5, 0, 4), (5, 4));
    assert_eq!(combine_strengths(3, 0, 2, 0), (3, 2));
}

#[test]
fn chroma_plane_pass_uses_yuv422_subsampling() {
    let pass = PlanePass::active(
        1,
        0,
        filter([false, false, true, false]),
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
        PixelFormat::Yuv422,
        &(0..1),
    )
    .unwrap();
    assert_eq!(pass.plane_sub_x, 1);
    assert_eq!(pass.plane_sub_y, 0);
    assert_eq!(pass.row_step, 1);
    assert_eq!(pass.col_step, 2);
}

#[test]
fn empty_apply_pattern_is_a_no_op() {
    let mut workspace = yuv420_workspace(64, 64, 100);
    deblock_general_intra_frame(
        &mut workspace,
        &[],
        16,
        16,
        filter([false; 4]),
        None,
        false,
        DeblockQuantDeltas::ZERO,
        BitDepth::Eight,
    )
    .unwrap();
    assert!(
        workspace
            .samples(PlaneId::Y)
            .unwrap()
            .iter()
            .all(|&s| s == 100),
        "no-op deblock leaves the workspace untouched"
    );
}

#[test]
fn unchanged_border_taps_do_not_require_in_frame_write_coordinates() {
    let mut workspace = yuv420_workspace(16, 16, 100);
    with_plane_ctx(&mut workspace, PlaneId::Y, |ctx| {
        apply_sample_filter(
            ctx,
            PerpLine::new(4, 0, 1, 0),
            DeblockSampleFilter {
                boundary: GATHER_HALF,
                q_thr: 1,
                max_width_neg: GATHER_HALF,
                max_width_pos: GATHER_HALF,
                q_thresh_mult: 1,
                w_mult_neg: 1,
                w_mult_pos: 1,
                prev_lossless: true,
                curr_lossless: true,
                bit_depth: BitDepth::Eight,
            },
        )
        .unwrap();
    });
}

#[test]
fn deblock_bounds_use_coded_plane_storage_for_partial_edge_frame() {
    let workspace = yuv420_workspace(18, 14, 100);
    assert_eq!(
        coded_plane_dimensions(&workspace, PlaneId::Y).unwrap(),
        (18, 14)
    );
    assert_eq!(
        coded_plane_dimensions(&workspace, PlaneId::U).unwrap(),
        (9, 7)
    );
    assert_eq!(
        coded_plane_dimensions(&workspace, PlaneId::V).unwrap(),
        (9, 7)
    );
}

#[test]
fn mi_grid_covers_decoded_blocks() {
    let blocks = [DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction(0, 0, 3),
        chroma_prediction: prediction(0, 0, 2),
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 8,
        n4h: 8,
        luma_tx: 3,
        chroma_tx: Some(2),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 100,
        skip: false,
        lossless: true,
    }];
    let storage = build_mi_grid(&blocks, 16, 16).unwrap();
    let grid = MiGrid::new(&storage, None, &blocks, &EMPTY_CHROMA_RECORDS);
    assert!(grid.get_edge(0, 0).is_some(), "top-left MI is covered");
    assert!(
        grid.get_edge(0, 0).is_some_and(|info| info.block.lossless),
        "MI records preserve per-segment lossless state"
    );
    assert!(
        grid.get_edge(7, 7).is_some(),
        "bottom-right of the 8x8 footprint is covered"
    );
    assert!(
        grid.get_edge(8, 8).is_none(),
        "an MI outside the block is uncovered"
    );
    let info = grid.get_edge(0, 0).unwrap();
    assert_eq!(info.tx_base(0), (0, 0));
    assert_eq!(info.tx(0), 3, "luma tx index");
    assert_eq!(info.tx(1), 2, "chroma tx index");
}

#[test]
fn inherited_chroma_residual_transform_retains_prediction_metadata() {
    let luma = [DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction(0, 0, 0),
        chroma_prediction: prediction(0, 0, 0),
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 8,
        n4h: 8,
        luma_tx: 0,
        chroma_tx: None,
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 1,
        skip: false,
        lossless: false,
    }];
    let metadata = DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction(0, 3, 1),
        chroma_prediction: prediction(0, 0, 3),
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 4,
        n4h: 2,
        luma_tx: 1,
        chroma_tx: Some(2),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 90,
        skip: false,
        lossless: false,
    };
    let transform = DeblockBlock {
        r: 0,
        c: 2,
        luma_prediction: prediction(7, 7, 0),
        chroma_prediction: prediction(7, 7, 0),
        chroma_base_r: 0,
        chroma_base_c: 2,
        n4w: 2,
        n4h: 2,
        luma_tx: 0,
        chroma_tx: Some(5),
        sub_pu_size: None,
        chroma_transform_only: true,
        qindex: 90,
        skip: false,
        lossless: false,
    };
    let base = build_mi_grid(&luma, 8, 8).unwrap();
    let mut chroma = ChromaDeblockRecords::default();
    chroma.push(0, metadata);
    chroma.push(0, transform);
    let storage = overlay_mi_grid(&base, &chroma, 0, 8, 8).unwrap();
    let grid = MiGrid::new(&base, Some(&storage), &luma, &chroma);

    let inherited = grid.get_edge(1, 3).unwrap();
    let chroma_prediction = inherited.prediction(1);
    assert_eq!((chroma_prediction.base_r, chroma_prediction.base_c), (0, 0));
    assert_eq!(chroma_prediction.default_sub_pu_tx, 3);
    assert_eq!(inherited.tx_base(1), (0, 2));
    assert_eq!(inherited.tx(1), 5);
    assert_eq!(inherited.block.qindex, 90);
    assert!(!inherited.block.skip);
    assert!(!inherited.block.lossless);
    assert_eq!(inherited.block.sub_pu_size, None);
}

#[test]
fn ordinary_chroma_overlay_replaces_full_block_metadata() {
    let luma = [DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction(0, 0, 0),
        chroma_prediction: prediction(0, 0, 0),
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 4,
        n4h: 2,
        luma_tx: 0,
        chroma_tx: None,
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 1,
        skip: true,
        lossless: false,
    }];
    let ordinary = DeblockBlock {
        r: 0,
        c: 2,
        luma_prediction: prediction(7, 7, 0),
        chroma_prediction: prediction(7, 7, 0),
        chroma_base_r: 0,
        chroma_base_c: 2,
        n4w: 2,
        n4h: 2,
        luma_tx: 0,
        chroma_tx: Some(5),
        sub_pu_size: Some(DeblockSubPuSize::new(4, 8)),
        chroma_transform_only: false,
        qindex: 255,
        skip: false,
        lossless: true,
    };
    let base = build_mi_grid(&luma, 8, 8).unwrap();
    let mut chroma = ChromaDeblockRecords::default();
    chroma.push(0, ordinary);
    let storage = overlay_mi_grid(&base, &chroma, 0, 8, 8).unwrap();
    let grid = MiGrid::new(&base, Some(&storage), &luma, &chroma);

    let info = grid.get_edge(1, 3).unwrap();
    let chroma_prediction = info.prediction(1);
    assert_eq!((chroma_prediction.base_r, chroma_prediction.base_c), (7, 7));
    assert_eq!(info.tx_base(1), (0, 2));
    assert_eq!(info.tx(1), 5);
    assert_eq!(info.block.sub_pu_size, Some(DeblockSubPuSize::new(4, 8)));
    assert_eq!(info.block.qindex, 255);
    assert!(!info.block.skip);
    assert!(info.block.lossless);
}

#[test]
fn ordinary_chroma_transform_record_keeps_scaled_prediction_origin() {
    let (plane, record) = crate::filters::wienerns_lr::chroma_transform_deblock_block(
        PlaneId::U,
        8,
        12,
        3,
        None,
        (1, 1),
        77,
        false,
    )
    .unwrap();
    assert_eq!(plane, 0);
    assert_eq!((record.r, record.c), (6, 4));
    assert_eq!(
        (record.luma_prediction.base_r, record.luma_prediction.base_c),
        (6, 4)
    );
    assert_eq!(
        (
            record.chroma_prediction.base_r,
            record.chroma_prediction.base_c
        ),
        (6, 4)
    );
    assert_eq!((record.chroma_base_r, record.chroma_base_c), (6, 4));
    assert!(!record.chroma_transform_only);

    let luma = deblock_blocks(16, 16);
    let base = build_mi_grid(&luma, 16, 16).unwrap();
    let mut chroma = ChromaDeblockRecords::default();
    chroma.push(0, record);
    let storage = overlay_mi_grid(&base, &chroma, 0, 16, 16).unwrap();
    let grid = MiGrid::new(&base, Some(&storage), &luma, &chroma);
    let info = grid.get_edge(6, 4).unwrap();
    let chroma_prediction = info.prediction(1);
    assert_eq!((chroma_prediction.base_r, chroma_prediction.base_c), (6, 4));
    assert_eq!(info.tx_base(1), (6, 4));
    assert_eq!(info.block.qindex, 77);
}

#[test]
fn skip_suppresses_internal_tx_edge_filtering() {
    let mut skipped = yuv420_workspace(64, 16, 100);
    fill_rect(&mut skipped, PlaneId::Y, 20..64, 0..16, 108);
    with_plane_ctx(&mut skipped, PlaneId::Y, |ctx| {
        deblock_filter_edge(
            ctx,
            &edge_test_grid(true),
            EdgeContext {
                row: 0,
                col: 5,
                plane_sub_x: 0,
                plane_sub_y: 0,
                bit_depth: BitDepth::Eight,
                allow_df_sub_pu: false,
                tile_edge: false,
            },
            false,
            &StrengthCache::new(0, 0, BitDepth::Eight),
        )
        .unwrap();
    });
    assert_eq!(
        skipped.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
        100,
        "skipped internal edge leaves the previous tap unchanged"
    );
    assert_eq!(
        skipped.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
        108,
        "skipped internal edge leaves the current tap unchanged"
    );

    let mut coded = yuv420_workspace(64, 16, 100);
    fill_rect(&mut coded, PlaneId::Y, 20..64, 0..16, 108);
    with_plane_ctx(&mut coded, PlaneId::Y, |ctx| {
        deblock_filter_edge(
            ctx,
            &edge_test_grid(false),
            EdgeContext {
                row: 0,
                col: 5,
                plane_sub_x: 0,
                plane_sub_y: 0,
                bit_depth: BitDepth::Eight,
                allow_df_sub_pu: false,
                tile_edge: false,
            },
            false,
            &StrengthCache::new(0, 0, BitDepth::Eight),
        )
        .unwrap();
    });
    assert_smoothed_step(
        coded.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
        coded.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
        "coded internal edge still filters",
    );
}

#[test]
fn tile_boundary_filtering_obeys_sequence_flag() {
    let run = |disable_loopfilters_across_tiles| {
        let mut workspace = yuv420_workspace(64, 16, 100);
        fill_rect(&mut workspace, PlaneId::Y, 20..64, 0..16, 108);
        with_plane_ctx(&mut workspace, PlaneId::Y, |ctx| {
            deblock_filter_edge(
                ctx,
                &edge_test_grid(false),
                EdgeContext {
                    row: 0,
                    col: 5,
                    plane_sub_x: 0,
                    plane_sub_y: 0,
                    bit_depth: BitDepth::Eight,
                    allow_df_sub_pu: false,
                    tile_edge: true,
                },
                disable_loopfilters_across_tiles,
                &StrengthCache::new(0, 0, BitDepth::Eight),
            )
            .unwrap();
        });
        workspace
    };

    let disabled = run(true);
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 19, 0), Ok(100));
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 20, 0), Ok(108));

    let enabled = run(false);
    assert_smoothed_step(
        enabled.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
        enabled.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
        "tile edge filters when cross-tile loop filtering is enabled",
    );
}

#[test]
fn allow_df_sub_pu_gates_prediction_boundary_filtering() {
    let grid = edge_test_grid_with_metadata(true, true);
    let run = |allow_df_sub_pu| {
        let mut workspace = yuv420_workspace(64, 16, 100);
        fill_rect(&mut workspace, PlaneId::Y, 20..64, 0..16, 108);
        with_plane_ctx(&mut workspace, PlaneId::Y, |ctx| {
            deblock_filter_edge(
                ctx,
                &grid,
                EdgeContext {
                    row: 0,
                    col: 5,
                    plane_sub_x: 0,
                    plane_sub_y: 0,
                    bit_depth: BitDepth::Eight,
                    allow_df_sub_pu,
                    tile_edge: false,
                },
                false,
                &StrengthCache::new(0, 0, BitDepth::Eight),
            )
            .unwrap();
        });
        workspace
    };

    let disabled = run(false);
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 19, 0), Ok(100));
    assert_eq!(disabled.reconstructed_sample(PlaneId::Y, 20, 0), Ok(108));

    let enabled = run(true);
    assert_smoothed_step(
        enabled.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
        enabled.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
        "allow_df_sub_pu enables the prediction boundary",
    );
}

#[test]
fn luma_vertical_pass_filters_the_x64_block_edge() {
    let mut ws = yuv420_workspace(128, 64, 100);
    fill_rect(&mut ws, PlaneId::Y, 64..128, 0..64, 108);
    let blocks = deblock_blocks(16, 32);
    run_deblock(&mut ws, &blocks, 16, 32, [true, false, false, false]);
    let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
    assert_eq!(at(10, 32), 100, "left interior untouched");
    assert_eq!(at(120, 32), 108, "right interior untouched");
    assert_eq!(at(31, 32), 100, "x=32 within-region edge untouched");
    assert_eq!(at(32, 32), 100, "x=32 within-region edge untouched");
    assert_eq!(at(95, 32), 108, "x=96 within-region edge untouched");
    assert_eq!(at(96, 32), 108, "x=96 within-region edge untouched");
    assert_smoothed_step(
        at(63, 32),
        at(64, 32),
        "luma-vertical pass must change the x=64 edge",
    );
}

#[test]
fn luma_horizontal_pass_filters_the_y64_superblock_edge() {
    let mut ws = yuv420_workspace(128, 128, 100);
    fill_rect(&mut ws, PlaneId::Y, 0..128, 64..128, 108);
    let blocks = deblock_blocks(32, 32);
    run_deblock(&mut ws, &blocks, 32, 32, [false, true, false, false]);
    let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
    assert_eq!(at(64, 10), 100, "top interior untouched");
    assert_eq!(at(64, 120), 108, "bottom interior untouched");
    assert_eq!(at(64, 31), 100, "y=32 within-region edge untouched");
    assert_eq!(at(64, 96), 108, "y=96 within-region edge untouched");
    assert_smoothed_step(
        at(64, 63),
        at(64, 64),
        "luma-horizontal sbEdge pass must change the y=64 edge",
    );
    assert_eq!(
        at(64, 59),
        100,
        "sbEdge caps the upward extent (row 59 unchanged)"
    );
}

#[test]
fn chroma_pass_filters_the_chroma_block_edge() {
    let mut ws = yuv420_workspace(128, 64, 100);
    fill_rect(&mut ws, PlaneId::U, 32..64, 0..32, 108);
    let blocks = deblock_blocks(16, 32);
    run_deblock(&mut ws, &blocks, 16, 32, [false, false, true, false]);
    let u = |x, y| ws.reconstructed_sample(PlaneId::U, x, y).unwrap();
    assert_eq!(u(8, 16), 100, "left chroma interior untouched");
    assert_eq!(u(60, 16), 108, "right chroma interior untouched");
    assert_smoothed_step(
        u(31, 16),
        u(32, 16),
        "chroma pass must change the chroma x=32 edge",
    );
    assert_eq!(
        ws.reconstructed_sample(PlaneId::V, 31, 16).unwrap(),
        100,
        "V plane untouched (apply[3] == false)"
    );
}

#[test]
fn chroma_pass_uses_4x4_tx_for_sub8_luma_records() {
    let mut ws = yuv420_workspace(8, 16, 100);
    fill_rect(&mut ws, PlaneId::U, 0..4, 4..8, 108);
    let block = |r| DeblockBlock {
        r,
        c: 0,
        luma_prediction: prediction(r, 0, 0),
        chroma_prediction: prediction(r, 0, 0),
        chroma_base_r: r,
        chroma_base_c: 0,
        n4w: 2,
        n4h: 2,
        luma_tx: 0,
        chroma_tx: None,
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 100,
        skip: false,
        lossless: false,
    };
    run_deblock(
        &mut ws,
        &[block(0), block(2)],
        4,
        2,
        [false, false, true, false],
    );
    assert_smoothed_step(
        ws.reconstructed_sample(PlaneId::U, 1, 3).unwrap(),
        ws.reconstructed_sample(PlaneId::U, 1, 4).unwrap(),
        "sub-8x8 luma records still have a 4x4 chroma transform",
    );
}

#[test]
fn banded_parallel_pass_matches_serial_output() {
    use splot_parallel::{ThreadCount, WorkerPool};

    let (mi_rows, mi_cols) = (16usize, 32usize);
    let blocks = deblock_blocks(mi_rows, mi_cols);
    let mut serial = yuv420_workspace_10bit(128, 64, 512);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        splat_asymmetric(&mut serial, plane, 1023);
    }
    let run = |ws: &mut CurrentFrameWorkspace<u16>| {
        deblock_general_intra_frame(
            ws,
            &blocks,
            mi_rows,
            mi_cols,
            filter([true, true, true, true]),
            None,
            false,
            DeblockQuantDeltas::ZERO,
            BitDepth::Ten,
        )
        .unwrap();
    };
    run(&mut serial);
    for threads in [1, 4] {
        let mut parallel = yuv420_workspace_10bit(128, 64, 512);
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            splat_asymmetric(&mut parallel, plane, 1023);
        }
        let pool = WorkerPool::new(ThreadCount::Fixed(threads.try_into().unwrap())).unwrap();
        assert!(pool.install(|| {
            let active = splot_parallel::on_worker_pool();
            run(&mut parallel);
            active
        }));

        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            assert_eq!(
                serial.samples(plane).unwrap(),
                parallel.samples(plane).unwrap(),
                "banded pass with {threads} worker(s) must match serial for {plane:?}"
            );
        }
    }
}
