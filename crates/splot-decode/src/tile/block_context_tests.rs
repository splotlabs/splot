// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;

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
fn plane_blocks_scale_luma_and_420_chroma() {
    let ctx = ctx(8, 16, 16, 8);

    let y = ctx.plane_block(PlaneId::Y);
    assert_eq!((y.x(), y.y()), (64, 32));
    assert_eq!((y.width4(), y.height4()), (16, 8));
    assert_eq!((y.tx().width_log2(), y.tx().height_log2()), (6, 5));

    let u = ctx.plane_block(PlaneId::U);
    assert_eq!((u.x(), u.y()), (32, 16));
    assert_eq!((u.width4(), u.height4()), (8, 4));
    assert_eq!((u.tx().width_log2(), u.tx().height_log2()), (5, 4));
}

#[test]
fn plane_blocks_use_chroma_ref_geometry_for_420_chroma() {
    let chroma_ref = BlockRect::new(24, 206, 2, 4);
    let chroma_tx = TxShape::from_luma_4x4(2, 4).expect("valid chroma reference transform");
    let ctx = ctx(24, 207, 1, 4).with_chroma_ref(chroma_ref, chroma_tx);

    let y = ctx.plane_block(PlaneId::Y);
    assert_eq!((y.x(), y.y()), (828, 96));
    assert_eq!((y.width4(), y.height4()), (1, 4));

    let u = ctx.plane_block(PlaneId::U);
    assert_eq!((u.x(), u.y()), (412, 48));
    assert_eq!((u.width4(), u.height4()), (1, 2));
    assert_eq!((u.tx().width_log2(), u.tx().height_log2()), (2, 3));
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
