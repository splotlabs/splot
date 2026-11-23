// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A `size`-sample square leaf at 4x4 position `mi` of a tile of
/// `rows x cols` 4x4 units that starts at (0, 0).
fn leaf(mi: (usize, usize), size: usize, rows: usize, cols: usize) -> IntrabcMvBlock {
    IntrabcMvBlock {
        mi,
        size: (size, size),
        chroma: None,
        tile: (0..rows, 0..cols),
        sb_size: 64,
        allow_local_intrabc: false,
        allow_global_intrabc: true,
        allow_extra_ibc_range: false,
    }
}

/// A block vector of whole samples `(rows, cols)` in 1/8 samples.
const fn bv(rows: i32, cols: i32) -> [i32; 2] {
    [rows * 8, cols * 8]
}

fn global(block: &IntrabcMvBlock, mv: [i32; 2]) -> bool {
    is_mv_valid(block, None, mv)
}

#[test]
fn the_source_must_lie_in_the_tile_with_its_sub_sample_border() {
    let mut block = leaf((16, 16), 16, 32, 64);
    block.tile = (8..32, 8..64);
    let edges = [8 * 4, 8 * 4, 32 * 4, 64 * 4];
    assert!(source_within_tile(
        block.mi,
        block.size,
        None,
        edges,
        bv(-32, -32)
    ));
    assert!(
        !source_within_tile(block.mi, block.size, None, edges, bv(-33, -32)),
        "above the tile"
    );
    assert!(
        !source_within_tile(block.mi, block.size, None, edges, bv(-32, -33)),
        "left of the tile"
    );
    assert!(source_within_tile(
        block.mi,
        block.size,
        None,
        edges,
        bv(48, 0)
    ));
    assert!(
        !source_within_tile(block.mi, block.size, None, edges, bv(49, 0)),
        "below the tile"
    );
    assert!(
        !source_within_tile(block.mi, block.size, None, edges, [48 * 8 + 1, 0]),
        "sub-sample bottom border"
    );
    assert!(source_within_tile(
        block.mi,
        block.size,
        None,
        edges,
        bv(0, 176)
    ));
    assert!(
        !source_within_tile(block.mi, block.size, None, edges, [0, 176 * 8 + 4]),
        "sub-sample right border"
    );
    block.mi = (17, 17);
    block.size = (4, 4);
    block.chroma = Some(((16, 16), (8, 8)));
    block.allow_global_intrabc = false;
    block.allow_local_intrabc = true;
    assert!(
        !is_mv_valid(&block, None, bv(-32, -32)),
        "tile-edge check alone still fails closed without local or global area"
    );
    let chroma_mi = block.chroma.map(|(mi, _)| mi);
    assert!(
        source_within_tile(block.mi, block.size, chroma_mi, edges, bv(-32, -32)),
        "the chroma edges are 32"
    );
    assert!(
        !source_within_tile(block.mi, block.size, chroma_mi, edges, bv(-32, -33)),
        "luma left 35, chroma left 31"
    );
}

#[test]
fn without_local_or_global_area_no_vector_is_valid() {
    let mut block = leaf((16, 32), 16, 32, 64);
    assert!(global(&block, bv(-64, -64)));
    block.allow_global_intrabc = false;
    assert!(!global(&block, bv(-64, -64)));
}

#[test]
fn the_global_area_keeps_the_superblock_delay() {
    let block = leaf((16, 32), 16, 32, 64);
    assert!(global(&block, bv(-64, -16)), "source in 64x64 column 1");
    assert!(
        !global(&block, bv(-64, -15)),
        "column 2: srcSb64 >= activeSb64 - INTRABC_DELAY_SB64"
    );
    assert!(!global(&block, bv(0, -128)), "the current superblock row");
}

#[test]
fn the_global_area_keeps_the_wavefront() {
    let block = leaf((16, 0), 16, 32, 256);
    assert!(global(&block, bv(-64, 48)));
    assert!(
        !global(&block, bv(-64, 49)),
        "srcSb64Col >= activeSb64Col - INTRABC_DELAY_SB64 + wfOffset"
    );
}

#[test]
fn allow_extra_ibc_range_widens_the_bottom_left_64x64_of_a_128x128_superblock() {
    let mut block = leaf((16, 64), 16, 64, 256);
    block.sb_size = 128;
    let mv = bv(-64, -256);
    assert!(!global(&block, mv));
    block.allow_extra_ibc_range = true;
    assert!(global(&block, mv));
    assert!(!global(&block, bv(-64, -207)), "source in 64x64 column 1");
    block.mi = (0, 64);
    assert!(!global(&block, bv(0, -256)), "not the bottom-left 64x64");
}

/// The 4x4 cells of `cells` (`(rows, cols)` ranges) are coded.
fn coded(cells: &[(Range<usize>, Range<usize>)]) -> impl Fn(usize, usize) -> bool + '_ {
    move |row, col| {
        cells
            .iter()
            .any(|(rows, cols)| rows.contains(&row) && cols.contains(&col))
    }
}

/// The buffers after leaves in the 64x64 regions of columns `0..=last` of
/// the first superblock row, with 64x64 superblocks.
fn buffers_after(last: usize) -> IbcBuffers {
    let mut buffers = IbcBuffers::start_sb_row(0, true);
    for col in 0..=last {
        buffers.update(0, col * 16, |_, _| false);
    }
    buffers
}

fn local(
    block: &IntrabcMvBlock,
    sdp_intra: bool,
    buffers: &IbcBuffers,
    coded: &dyn Fn(usize, usize) -> bool,
    mv: [i32; 2],
) -> bool {
    let state = LocalAreaState {
        sdp_intra,
        buffers,
        coded,
    };
    is_mv_valid(block, Some(&state), mv)
}

fn local_leaf(mi: (usize, usize), size: usize) -> IntrabcMvBlock {
    let mut block = leaf(mi, size, 32, 128);
    block.allow_local_intrabc = true;
    block.allow_global_intrabc = false;
    block
}

#[test]
fn the_local_area_needs_coded_corners_before_the_block() {
    let block = local_leaf((4, 16), 8);
    let buffers = buffers_after(1);
    let cells = [(0..8, 0..16), (0..4, 16..32), (4..6, 18..20)];
    let coded = coded(&cells);
    assert!(local(&block, false, &buffers, &coded, bv(0, -32)));
    assert!(local(&block, false, &buffers, &coded, bv(-16, 0)), "above");
    assert!(
        !local(&block, false, &buffers, &coded, bv(16, -32)),
        "uncoded"
    );
    assert!(
        !local(&block, false, &buffers, &coded, bv(0, 8)),
        "a corner at or right of and below the block's top-left sample"
    );
}

#[test]
fn the_local_area_reads_only_the_buffers_of_the_superblock_row() {
    let block = local_leaf((4, 80), 8);
    let buffers = buffers_after(5);
    let cells = [(0..16, 0..80), (0..4, 80..96)];
    let coded = coded(&cells);
    assert!(
        !local(&block, false, &buffers, &coded, bv(0, -300)),
        "region 0 was replaced by region 4"
    );
    assert!(
        local(&block, false, &buffers, &coded, bv(24, -128)),
        "region 3"
    );
    assert!(
        !local(&block, false, &buffers, &coded, bv(-16, -256)),
        "region 1 at a coded colocated cell of the current region"
    );
    assert!(
        local(&block, false, &buffers, &coded, bv(24, -256)),
        "region 1"
    );
    let fresh = IbcBuffers::start_sb_row(0, true);
    assert!(
        !local(&block, false, &fresh, &coded, bv(24, -128)),
        "no buffer"
    );
}

#[test]
fn the_local_area_of_128x128_superblocks_indexes_by_row_and_column() {
    let mut block = local_leaf((20, 20), 8);
    block.sb_size = 128;
    let mut buffers = IbcBuffers::start_sb_row(0, false);
    for (row, col) in [(0, 0), (0, 16), (16, 0), (16, 16)] {
        buffers.update(row, col, |_, _| false);
    }
    let cells = [(0..32, 0..16), (0..16, 16..32), (16..20, 16..32)];
    let coded = coded(&cells);
    assert!(
        local(&block, false, &buffers, &coded, bv(-16, -32)),
        "(1, 0)"
    );
    assert!(local(&block, false, &buffers, &coded, bv(-48, 0)), "(0, 1)");
}

#[test]
fn sdp_intra_checks_the_luma_leaf_in_place_of_its_chroma_block() {
    let mut block = local_leaf((5, 17), 4);
    block.chroma = Some(((4, 16), (8, 8)));
    let buffers = buffers_after(1);
    let cells = [(0..16, 0..14), (5..8, 14..16), (0..4, 16..32)];
    let coded = coded(&cells);
    let mv = bv(0, -8);
    assert!(!local(&block, false, &buffers, &coded, mv), "cell (4, 14)");
    assert!(local(&block, true, &buffers, &coded, mv), "cell (5, 15)");
}

#[test]
fn the_global_area_still_accepts_what_the_local_area_refuses() {
    let mut block = local_leaf((16, 32), 16);
    let buffers = IbcBuffers::start_sb_row(16, true);
    let coded = coded(&[]);
    assert!(!local(&block, false, &buffers, &coded, bv(-64, -64)));
    block.allow_global_intrabc = true;
    assert!(local(&block, false, &buffers, &coded, bv(-64, -64)));
}

#[test]
fn without_the_local_area_state_the_local_area_fails_closed() {
    let mut block = leaf((16, 32), 16, 32, 64);
    let mv = bv(0, -128);
    assert!(!global(&block, mv));
    block.allow_local_intrabc = true;
    assert!(!global(&block, mv));
}

#[test]
fn the_offset_vector_moves_the_source_by_whole_samples() {
    let mut block = leaf((4, 12), 16, 16, 64);
    block.allow_local_intrabc = true;
    block.allow_global_intrabc = false;
    let buffers = IbcBuffers::start_sb_row(0, true);
    let cells = [(0..4, 0..16), (4..16, 0..12)];
    let coded_fn = coded(&cells);
    let state = LocalAreaState {
        sdp_intra: false,
        buffers: &buffers,
        coded: &coded_fn,
    };
    let mv = [-128, -160];
    assert!(is_mv_valid(&block, Some(&state), mv));
    assert!(!is_offset_mv_valid(&block, Some(&state), mv, (-1, -1)));
    assert!(is_offset_mv_valid(
        &block,
        Some(&state),
        [-120, -160],
        (-1, -1)
    ));
}

#[test]
fn ibc_buffers_sb64_bru_inactive_reset_distinguishes_modes_and_sb_sizes() {
    let bru_active = 2u8;
    let bru_support = 1u8;
    let bru_inactive = 0u8;

    for (mode, expected_slot0) in [
        (bru_active, Some((0, 0))),
        (bru_support, Some((0, 0))),
        (bru_inactive, None),
    ] {
        let mut buffers = IbcBuffers::start_sb_row(0, true);
        assert_eq!(buffers.cur(), (0, 0));
        assert_eq!(buffers.stored(), [None; 4]);
        let changed = buffers.update(0, 16, |r, c| {
            assert_eq!((r, c), (0, 0));
            mode == bru_inactive
        });
        assert!(changed);
        assert_eq!(buffers.cur(), (0, 1));
        assert_eq!(
            buffers.stored(),
            [expected_slot0, None, None, None],
            "mode={mode}"
        );
    }

    for leaving_mode in [bru_active, bru_support, bru_inactive] {
        let mut buffers = IbcBuffers::start_sb_row(0, true);
        buffers.update(0, 16, |_, _| false);
        buffers.update(0, 32, |_, _| false);
        buffers.update(0, 48, |_, _| false);
        assert_eq!(
            buffers.stored(),
            [Some((0, 0)), Some((0, 1)), Some((0, 2)), None]
        );
        buffers.update(0, 64, |r, c| {
            assert_eq!((r, c), (0, 48));
            leaving_mode == bru_inactive
        });
        assert_eq!(buffers.cur(), (0, 4));
        if leaving_mode == bru_inactive {
            assert_eq!(buffers.stored(), [None; 4]);
        } else {
            assert_eq!(
                buffers.stored(),
                [Some((0, 0)), Some((0, 1)), Some((0, 2)), Some((0, 3))]
            );
        }
    }

    for chunk in [(0, 16), (16, 0), (16, 16)] {
        let mut sb128_or_256 = IbcBuffers::start_sb_row(0, false);
        let changed = sb128_or_256.update(chunk.0, chunk.1, |_, _| true);
        assert!(changed);
        assert_eq!(
            sb128_or_256.stored(),
            [Some((0, 0)), None, None, None],
            "SB128/SB256 must not apply the SB64-only BRU_INACTIVE reset"
        );
    }
}

#[test]
fn tile_local_ibc_state_enforces_bru_reset_support_future_and_tile_bounds() {
    let mut tile = TileLocalIbcState::new(0..96, 16, false).unwrap();
    tile.begin_superblock(0, true);
    tile.enter_leaf(0, 0, 16, 16, true);
    assert!(tile.is_coded(0, 0));

    tile.begin_superblock(0, false);
    tile.enter_leaf(0, 16, 4, 4, false);
    assert_eq!(tile.buffers().stored(), [None; 4]);

    let block_sb1 = IntrabcMvBlock {
        mi: (4, 16),
        size: (8, 8),
        chroma: None,
        tile: (0..32, 0..96),
        sb_size: 64,
        allow_local_intrabc: true,
        allow_global_intrabc: false,
        allow_extra_ibc_range: false,
    };
    let check_sb1 = |mv| {
        let coded_fn = |r, c| tile.is_coded(r, c);
        let local_area = LocalAreaState {
            sdp_intra: false,
            buffers: tile.buffers(),
            coded: &coded_fn,
        };
        is_mv_valid(&block_sb1, Some(&local_area), mv)
    };
    assert!(
        !check_sb1(bv(0, -16)),
        "source in BRU_INACTIVE SB (0, 0) must fail after SB64 reset"
    );
    assert!(
        !check_sb1(bv(0, 4)),
        "uncoded right/future block in current SB must fail"
    );
    assert!(
        check_sb1(bv(-16, 0)),
        "coded block above inside current SB must pass"
    );

    tile.enter_leaf(4, 16, 12, 12, false);
    tile.begin_superblock(0, false);
    tile.enter_leaf(0, 32, 16, 16, false);
    tile.begin_superblock(0, false);
    tile.enter_leaf(0, 48, 4, 4, false);
    assert_eq!(
        tile.buffers().stored(),
        [None, Some((0, 1)), Some((0, 2)), None],
        "leaving BRU_SUPPORT SB (0, 32) preserves buffer 1 and stores buffer 2"
    );

    let block_sb3 = IntrabcMvBlock {
        mi: (4, 48),
        ..block_sb1.clone()
    };
    let coded_fn = |r, c| tile.is_coded(r, c);
    let local_area = LocalAreaState {
        sdp_intra: false,
        buffers: tile.buffers(),
        coded: &coded_fn,
    };
    assert!(
        is_mv_valid(&block_sb3, Some(&local_area), bv(0, -64)),
        "BRU_SUPPORT SB (0, 32) must remain a valid local IBC source"
    );
    assert!(
        is_mv_valid(&block_sb3, Some(&local_area), bv(0, -128)),
        "earlier active SB (0, 16) must stay valid across BRU_SUPPORT SB"
    );
    assert!(
        !is_mv_valid(&block_sb3, Some(&local_area), bv(-16, -192)),
        "BRU_INACTIVE SB (0, 0) remains invalidated"
    );

    let mut tile2 = TileLocalIbcState::new(16..64, 16, false).unwrap();
    tile2.begin_superblock(0, false);
    tile2.enter_leaf(0, 16, 16, 16, false);
    tile2.begin_superblock(0, false);
    tile2.enter_leaf(0, 32, 4, 4, false);
    assert_eq!(tile2.buffers().cur(), (0, 2));
    assert_eq!(tile2.buffers().stored()[1], Some((0, 1)));
    let tile2_block = IntrabcMvBlock {
        mi: (0, 32),
        size: (8, 8),
        chroma: None,
        tile: (0..16, 16..64),
        sb_size: 64,
        allow_local_intrabc: true,
        allow_global_intrabc: false,
        allow_extra_ibc_range: false,
    };
    let coded_fn2 = |r, c| tile2.is_coded(r, c);
    let local_area2 = LocalAreaState {
        sdp_intra: false,
        buffers: tile2.buffers(),
        coded: &coded_fn2,
    };
    assert!(is_mv_valid(&tile2_block, Some(&local_area2), bv(0, -16)));
    assert!(
        !is_mv_valid(&tile2_block, Some(&local_area2), bv(0, -68)),
        "cross-tile source left of MiColStart=16 must fail"
    );
}

#[test]
fn tile_local_ibc_state_handles_large_leaves_out_of_frame_padding_and_colocated_collisions() {
    let mut sb128 = TileLocalIbcState::new(0..56, 32, false).unwrap();
    sb128.begin_superblock(0, true);
    sb128.enter_leaf(0, 0, 32, 32, true);
    sb128.finish_large_leaf(0, 0, 32, 32, true);
    assert_eq!(sb128.buffers().cur(), (1, 1));
    assert_eq!(
        sb128.buffers().stored(),
        [Some((0, 0)), Some((0, 1)), Some((1, 0)), None,],
        "128x128 leaf updates buffers in 2x2 chunk order and SB128 ignores BRU_INACTIVE"
    );

    let mut clipped = TileLocalIbcState::new(0..40, 32, false).unwrap();
    clipped.begin_superblock(0, false);
    clipped.enter_leaf(0, 0, 16, 16, false);
    clipped.enter_leaf(0, 16, 16, 16, false);
    clipped.record_out_of_frame(16, 16, 16, 16, false);
    assert!(clipped.is_coded(31, 31));
    assert_eq!(clipped.buffers().cur(), (1, 1));

    let mut row = TileLocalIbcState::new(0..128, 16, false).unwrap();
    row.begin_superblock(0, false);
    for col in [0, 16, 32, 48] {
        row.begin_superblock(0, false);
        row.enter_leaf(0, col, 16, 16, false);
    }
    row.begin_superblock(0, false);
    row.enter_leaf(0, 64, 4, 4, false);
    let block = IntrabcMvBlock {
        mi: (4, 64),
        size: (8, 8),
        chroma: None,
        tile: (0..16, 0..128),
        sb_size: 64,
        allow_local_intrabc: true,
        allow_global_intrabc: false,
        allow_extra_ibc_range: false,
    };
    let coded_fn = |r, c| row.is_coded(r, c);
    let area = LocalAreaState {
        sdp_intra: false,
        buffers: row.buffers(),
        coded: &coded_fn,
    };
    assert!(
        !is_mv_valid(&block, Some(&area), bv(-16, -256)),
        "buffer (0, 0) replaced by current buffer (0, 4) and colocated (0, 64) is coded"
    );
    assert!(
        !is_mv_valid(&block, Some(&area), [0, -8 * 8 + 4]),
        "fractional right border extending into uncoded/future cell fails"
    );
    assert!(
        is_mv_valid(&block, Some(&area), bv(-16, -64)),
        "active previous buffer (0, 3) stays valid"
    );
}
