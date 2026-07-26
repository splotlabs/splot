// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;

const CHROMA_CORNER_CAP_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-chroma-corner-cap-256x256-q140.ivf"
);

fn ctx(row4: usize, col4: usize, width4: usize, height4: usize) -> BlockCtx {
    let rect = BlockRect::new(row4, col4, width4, height4);
    BlockCtx::new(
        rect,
        TxShape::from_luma_4x4(width4, height4).expect("valid test transform"),
        32,
        32,
        BitDepth::Eight,
        ChromaSampling::Yuv420,
    )
}

fn assert_plane_block(
    ctx: &BlockCtx,
    plane_id: PlaneId,
    xy: (usize, usize),
    size4: (usize, usize),
    tx_log2: (u32, u32),
) {
    let block = ctx.plane_block(plane_id);
    assert_eq!((block.x(), block.y()), xy);
    assert_eq!((block.width4(), block.height4()), size4);
    assert_eq!((block.tx().width_log2(), block.tx().height_log2()), tx_log2);
}

#[test]
fn chroma_corner_counts_follow_large_transform_dimension_caps() {
    assert_eq!(
        normalize_intra_corner_counts(PlaneId::U, 6, 6, 7, 9),
        (0, 0)
    );
    assert_eq!(
        normalize_intra_corner_counts(PlaneId::U, 6, 5, 7, 9),
        (0, 9)
    );
    assert_eq!(
        normalize_intra_corner_counts(PlaneId::V, 5, 6, 7, 9),
        (7, 0)
    );
    assert_eq!(
        normalize_intra_corner_counts(PlaneId::V, 5, 5, 7, 9),
        (7, 9)
    );
    assert_eq!(
        normalize_intra_corner_counts(PlaneId::Y, 6, 6, 7, 9),
        (7, 9)
    );
}

#[test]
fn large_chroma_smooth_repeats_top_edge_instead_of_decoded_above_right() {
    use splot_recon::{IntraRectBlockSize, IntraSmoothMode, PixelFormat};

    let block = BlockRect::new(16, 0, 16, 16);
    let tx = TxShape::from_luma_4x4(16, 16).expect("valid 64x64 transform");
    let block_ctx = BlockCtx::new(block, tx, 32, 32, BitDepth::Ten, ChromaSampling::Yuv444);
    let mut decoded =
        crate::bitstream::tile_payload::TileBlockDecodedState::new(3, 0, 0, 16, 32, 32)
            .expect("valid decoded-state grid");
    decoded.clear_superblock(16, 0);
    assert_eq!(decoded.count_top_right_avail(1, 0, 0, 16), 16);
    let neighbours = block_ctx.neighbours_from_block_decoded(PlaneId::U, &decoded);
    assert_eq!(neighbours.num_above_right(), 0);

    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u16>(
        128,
        128,
        BitDepth::Ten,
        PixelFormat::Yuv444,
    )
    .expect("valid 10-bit workspace");
    workspace
        .write_rect_block(
            PlaneId::U,
            0,
            60,
            IntraRectBlockSize::new(6, 2).expect("valid 64x4 edge block"),
            &[510; 64 * 4],
        )
        .expect("write top edge");
    workspace
        .write_rect_block(
            PlaneId::U,
            64,
            60,
            IntraRectBlockSize::new(2, 2).expect("valid sentinel block"),
            &[511; 4 * 4],
        )
        .expect("write decoded above-right sentinel");

    let mut prediction = vec![0; 64 * 64];
    crate::pipeline::reconstruct::predict_intra_smooth_over_available_edges_into(
        &workspace,
        crate::pipeline::reconstruct::SmoothIntraPredictionRequest {
            plane_id: PlaneId::U,
            x: 0,
            y: 64,
            block_size: IntraRectBlockSize::new(6, 6).expect("valid 64x64 block"),
            mode: IntraSmoothMode::SmoothHorizontal,
            available_left_samples: None,
            available_above_samples: Some(64),
            num4_above_right: neighbours.num_above_right(),
            num4_below_left: neighbours.num_below_left(),
            bit_depth: BitDepth::Ten,
        },
        &mut prediction,
    )
    .expect("smooth prediction");

    assert!(prediction.iter().all(|&sample| sample == 510));
}

#[test]
fn large_chroma_smooth_corner_cap_fixture_decodes_to_oracle() {
    use splot_parallel::ThreadCount;
    use splot_recon::DecodedFrameHashInput;

    let options = crate::DecodeOptions::default();
    let context =
        crate::DecodeContext::new(crate::DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("decode context");
    let plan = context
        .plan_bytes(CHROMA_CORNER_CAP_FIXTURE, options)
        .expect("decode plan");
    let decoded = context
        .pool()
        .install(|| {
            crate::pipeline::decode_frame_from_plan(CHROMA_CORNER_CAP_FIXTURE, &options, &plan)
        })
        .expect("decode fixture");
    let ready = decoded.ready_frame().expect("ready frame");
    assert!(
        matches!(&ready, crate::pipeline::PipelineDecodedFrame::Eight(_)),
        "fixture decoded at the wrong bit depth"
    );
    let crate::pipeline::PipelineDecodedFrame::Eight(frame) = ready else {
        return;
    };

    assert_eq!(
        DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
        "9d1331b7f113bcd0de59adfd04681009dd8a8c03b18d7ddbd651058fb406435c",
    );
}

#[test]
fn classifies_frame_edges() {
    let cases = [
        (ctx(0, 0, 16, 16), false, false, 16),
        (ctx(0, 16, 16, 16), false, true, 0),
        (ctx(16, 0, 16, 16), true, false, 16),
        (ctx(16, 16, 16, 16), true, true, 0),
        (ctx(8, 8, 8, 8), true, true, 8),
        (ctx(24, 8, 8, 8), true, true, 8),
    ];

    for (ctx, has_above, has_left, above_right) in cases {
        let neighbours = ctx.neighbours(PlaneId::Y);
        assert_eq!(neighbours.has_above(), has_above);
        assert_eq!(neighbours.has_left(), has_left);
        assert_eq!(neighbours.num_above_right(), above_right);
        assert_eq!(neighbours.num_below_left(), 0);
    }
}

#[test]
fn plane_blocks_scale_and_clamp_420_chroma() {
    let scaled = ctx(8, 16, 16, 8);

    assert_plane_block(&scaled, PlaneId::Y, (64, 32), (16, 8), (6, 5));
    assert_plane_block(&scaled, PlaneId::U, (32, 16), (8, 4), (5, 4));

    let minimum = ctx(0, 0, 1, 1);

    assert_plane_block(&minimum, PlaneId::U, (0, 0), (1, 1), (2, 2));
}

#[test]
fn plane_blocks_use_chroma_ref_geometry_for_420_chroma() {
    let chroma_ref = BlockRect::new(24, 206, 2, 4);
    let chroma_tx = TxShape::from_luma_4x4(2, 4).expect("valid chroma reference transform");
    let ctx = ctx(24, 207, 1, 4).with_chroma_ref(chroma_ref, chroma_tx);

    assert_plane_block(&ctx, PlaneId::Y, (828, 96), (1, 4), (2, 4));
    assert_plane_block(&ctx, PlaneId::U, (412, 48), (1, 2), (2, 3));
}

#[test]
fn block_decoded_neighbours_cover_subpartition_above_right() {
    let mut block_decoded =
        crate::bitstream::tile_payload::TileBlockDecodedState::new(3, 1, 1, 16, 32, 32)
            .expect("valid block decoded state");
    block_decoded.clear_superblock(0, 0);
    block_decoded.set_block(0, 0, 8, 8, 8);

    let bottom_left = ctx(8, 0, 8, 8);
    let neighbours = bottom_left.neighbours_from_block_decoded(PlaneId::Y, &block_decoded);

    assert!(neighbours.has_above());
    assert!(!neighbours.has_left());
    assert_eq!(neighbours.num_above_right(), 8);
    assert_eq!(neighbours.num_below_left(), 0);
}

#[test]
fn tile_bounds_make_tile_start_edges_unavailable() {
    let tile_start = ctx(0, 16, 16, 16).with_tile_bounds(0, 16, 16, 32);
    let neighbours = tile_start.neighbours(PlaneId::Y);

    assert!(!neighbours.has_above());
    assert!(!neighbours.has_left());
    assert_eq!(neighbours.num_above_right(), 0);
}

#[test]
fn tile_bounds_clip_above_right_to_tile_end() {
    let within_tile = ctx(8, 16, 8, 8).with_tile_bounds(0, 16, 16, 24);
    let neighbours = within_tile.neighbours(PlaneId::Y);

    assert!(neighbours.has_above());
    assert!(!neighbours.has_left());
    assert_eq!(neighbours.num_above_right(), 0);
}
