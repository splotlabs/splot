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
    is_mv_valid(block, LocalArea::StateNotAvailable, mv)
}

#[test]
fn the_source_must_lie_in_the_tile_with_its_sub_sample_border() {
    let mut block = leaf((16, 16), 16, 32, 64);
    block.tile = (8..32, 8..64);
    block.allow_global_intrabc = false;
    block.allow_local_intrabc = true;
    assert!(global(&block, bv(-32, -32)));
    assert!(!global(&block, bv(-33, -32)), "above the tile");
    assert!(!global(&block, bv(-32, -33)), "left of the tile");
    assert!(global(&block, bv(48, 0)));
    assert!(!global(&block, bv(49, 0)), "below the tile");
    assert!(!global(&block, [48 * 8 + 1, 0]), "sub-sample bottom border");
    assert!(global(&block, bv(0, 176)));
    assert!(!global(&block, [0, 176 * 8 + 4]), "sub-sample right border");
    block.mi = (17, 17);
    block.size = (4, 4);
    block.chroma = Some(((16, 16), (8, 8)));
    assert!(global(&block, bv(-32, -32)), "the chroma edges are 32");
    assert!(
        !global(&block, bv(-32, -33)),
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
        buffers.update(0, col * 16);
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
    is_mv_valid(block, LocalArea::State(state), mv)
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
        buffers.update(row, col);
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
fn without_the_local_area_state_the_local_area_accepts() {
    let mut block = leaf((16, 32), 16, 32, 64);
    let mv = bv(0, -128);
    assert!(!global(&block, mv));
    block.allow_local_intrabc = true;
    assert!(global(&block, mv));
    assert!(
        !global(&block, bv(-65, -128)),
        "the tile-edge test still runs"
    );
}

#[test]
fn the_offset_vector_moves_the_source_by_whole_samples() {
    let mut block = leaf((4, 12), 16, 16, 64);
    block.allow_local_intrabc = true;
    let mv = [-128, -160];
    assert!(is_mv_valid(&block, LocalArea::StateNotAvailable, mv));
    assert!(!is_offset_mv_valid(
        &block,
        LocalArea::StateNotAvailable,
        mv,
        (-1, -1)
    ));
    assert!(is_offset_mv_valid(
        &block,
        LocalArea::StateNotAvailable,
        [-120, -160],
        (-1, -1)
    ));
}
