// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::bitstream::tile_payload::{
    BlockSize, PartitionType, TilePartitionCall, child_calls, partition_test_frame, valid_subsize,
};

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

/// The tile-edge test of `block` (tile in 4x4 units) for `mv`.
fn in_tile(block: &IntrabcMvBlock, mv: [i32; 2]) -> bool {
    let (rows, cols) = &block.tile;
    let tile = [rows.start, cols.start, rows.end, cols.end].map(|mi| mi * 4);
    source_within_tile(
        block.mi,
        block.size,
        block.chroma.map(|(mi, _)| mi),
        tile,
        mv,
    )
}

#[test]
fn the_source_must_lie_in_the_tile_with_its_sub_sample_border() {
    let mut block = leaf((16, 16), 16, 32, 64);
    block.tile = (8..32, 8..64);
    assert!(in_tile(&block, bv(-32, -32)));
    assert!(!in_tile(&block, bv(-33, -32)), "above the tile");
    assert!(!in_tile(&block, bv(-32, -33)), "left of the tile");
    assert!(in_tile(&block, bv(48, 0)));
    assert!(!in_tile(&block, bv(49, 0)), "below the tile");
    assert!(
        !in_tile(&block, [48 * 8 + 1, 0]),
        "sub-sample bottom border"
    );
    assert!(in_tile(&block, bv(0, 176)));
    assert!(
        !in_tile(&block, [0, 176 * 8 + 4]),
        "sub-sample right border"
    );
    block.mi = (17, 17);
    block.size = (4, 4);
    block.chroma = Some(((16, 16), (8, 8)));
    assert!(in_tile(&block, bv(-32, -32)), "the chroma edges are 32");
    assert!(
        !in_tile(&block, bv(-32, -33)),
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
    is_mv_valid(block, Some(state), mv)
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
fn without_the_local_area_state_only_the_global_area_accepts() {
    let mut block = leaf((16, 32), 16, 32, 64);
    let mv = bv(-64, -64);
    assert!(global(&block, mv));
    block.allow_local_intrabc = true;
    block.allow_global_intrabc = false;
    assert!(!global(&block, mv));
}

#[test]
fn the_offset_vector_moves_the_source_by_whole_samples() {
    let block = local_leaf((4, 12), 16);
    let buffers = IbcBuffers::start_sb_row(0, true);
    let all = |_: usize, _: usize| true;
    let state = || {
        Some(LocalAreaState {
            sdp_intra: false,
            buffers: &buffers,
            coded: &all,
        })
    };
    let mv = [-128, -160];
    assert!(is_mv_valid(&block, state(), mv));
    assert!(!is_offset_mv_valid(&block, state(), mv, (-1, -1)));
    assert!(is_offset_mv_valid(&block, state(), [-120, -160], (-1, -1)));
}

#[test]
fn chunk_updates_follow_the_raster_and_the_residual_group_orders() {
    let raster: Vec<_> = chunk_origins([0, 0], [64, 32], ChunkOrder::Raster).collect();
    let grouped: Vec<_> = chunk_origins([0, 0], [64, 32], ChunkOrder::Grouped).collect();
    let at = |cells: &[(usize, usize)]| -> Vec<[usize; 2]> {
        cells.iter().map(|&(y, x)| [y * 16, x * 16]).collect()
    };
    assert_eq!(
        raster,
        at(&[
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3)
        ])
    );
    assert_eq!(
        grouped,
        at(&[
            (0, 0),
            (0, 1),
            (1, 0),
            (1, 1),
            (0, 2),
            (0, 3),
            (1, 2),
            (1, 3)
        ])
    );
    assert_eq!(
        chunk_origins([4, 8], [8, 4], ChunkOrder::Grouped).collect::<Vec<_>>(),
        [[4, 8]]
    );
}

/// The `[r, c, w4, h4]` of entry `index` of `blocks`.
fn entry(blocks: &[[usize; 4]]) -> impl Fn(usize) -> Option<[usize; 4]> + '_ {
    |index| blocks.get(index).copied()
}

#[test]
fn the_replay_interleaves_out_of_frame_calls_and_owes_the_leaf_chunks() {
    let blocks = [[0, 0, 16, 16], [16, 0, 32, 16]];
    let calls = [OutOfFrameCall {
        entry: 1,
        origin: [0, 16],
        size4: [8, 8],
    }];
    let mut pending = &calls[..];
    let mut replay = IbcLocalReplay::new(false);
    let superblock = ([0, 0], 32, 0);
    assert!(
        replay
            .replay_to_leaf(superblock, 1, entry(&blocks), &mut pending)
            .is_some()
    );
    assert!(pending.is_empty());
    assert_eq!(replay.buffers.cur, (1, 0));
    assert_eq!(replay.buffers.stored[0], Some((0, 0)));
    assert_eq!(
        replay.buffers.stored[1],
        Some((0, 1)),
        "the out-of-frame call"
    );
    assert!(replay.coded(0, 20), "out-of-frame cells are coded");
    assert!(!replay.coded(8, 16));
    assert!(replay.coded(31, 31), "the leaf's own cells");
    assert!(
        replay
            .finish_superblock(superblock, 2, entry(&blocks), &mut pending)
            .is_some()
    );
    assert_eq!(replay.buffers.cur, (1, 1), "the 128x64 leaf's second chunk");
    assert_eq!(replay.buffers.stored[2], Some((1, 0)));
}

#[test]
fn an_extended_sdp_chroma_block_moves_the_current_buffer_back() {
    let blocks = [[8, 0, 16, 8], [16, 0, 16, 8], [8, 0, 16, 16]];
    let mut replay = IbcLocalReplay::new(false);
    assert!(
        replay
            .replay_to_leaf(([0, 0], 32, 0), 2, entry(&blocks), &mut &[][..])
            .is_some()
    );
    assert_eq!(replay.buffers.cur, (0, 0));
    assert_eq!(replay.buffers.stored[2], Some((1, 0)));
}

#[test]
fn ibc_coded_outside_the_superblock_is_one_only_to_its_left_in_its_row() {
    let mut replay = IbcLocalReplay::new(false);
    assert!(
        replay
            .replay_to_leaf(([16, 32], 16, 0), 0, entry(&[[16, 32, 4, 4]]), &mut &[][..])
            .is_some()
    );
    assert!(
        replay.coded(16, 0) && replay.coded(31, 31),
        "left, same row"
    );
    assert!(replay.coded(19, 35) && !replay.coded(20, 32));
    assert!(!replay.coded(16, 48), "right of the superblock");
    assert!(!replay.coded(15, 0) && !replay.coded(32, 0), "other rows");
    let blocks = [[0; 4], [32, 0, 4, 4]];
    assert!(
        replay
            .finish_superblock(([32, 0], 16, 1), 2, entry(&blocks), &mut &[][..])
            .is_some()
    );
    assert_eq!(
        replay.buffers,
        IbcBuffers::start_sb_row(32, true),
        "a new row resets"
    );
}

#[test]
fn the_buffer_test_refuses_sources_right_of_the_superblock_or_in_earlier_rows() {
    let all = |_: usize, _: usize| true;
    let mut buffers = IbcBuffers::start_sb_row(0, true);
    buffers.update(0, 0);
    buffers.update(0, 16);
    let block = local_leaf((4, 16), 8);
    assert!(
        local(&block, false, &buffers, &all, bv(-16, -64)),
        "region 0"
    );
    assert!(
        !local(&block, false, &buffers, &all, bv(-16, 80)),
        "region 2, right of the current superblock"
    );
    let mut lower = IbcBuffers::start_sb_row(16, true);
    lower.update(16, 16);
    let block = local_leaf((20, 16), 8);
    assert!(
        !local(&block, false, &lower, &all, bv(-32, 0)),
        "the superblock row above"
    );
}

/// A step of a xorshift generator.
fn next(seed: &mut u64) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed as usize
}

/// A random partition tree under `call` in a frame of `mi` 4x4 rows and
/// columns: its leaves in `blocks`, its calls outside the frame in `calls`.
fn random_tree(
    seed: &mut u64,
    call: TilePartitionCall,
    mi: [usize; 2],
    (blocks, calls): (&mut Vec<[usize; 4]>, &mut Vec<OutOfFrameCall>),
) {
    let [h4, w4] = call.b_size.num_4x4();
    if call.r() >= mi[0] || call.c() >= mi[1] {
        calls.push(OutOfFrameCall {
            entry: blocks.len(),
            origin: [call.r(), call.c()],
            size4: [w4, h4],
        });
        return;
    }
    let splits: Vec<_> = PartitionType::ALL
        .into_iter()
        .filter_map(|p| Some((p, valid_subsize(p, call.b_size).ok()?)))
        .filter(|&(p, _)| p != PartitionType::None)
        .collect();
    let frame = partition_test_frame(15);
    let children = (next(seed) % 3 != 0 && !splits.is_empty())
        .then(|| splits[next(seed) % splits.len()])
        .and_then(|(p, sub)| child_calls(call, call.own_chroma_ref(), p, sub, &frame, false).ok());
    let Some(children) = children else {
        blocks.push([call.r(), call.c(), w4, h4]);
        return;
    };
    for &child in children.as_slice() {
        random_tree(seed, child, mi, (&mut *blocks, &mut *calls));
    }
}

#[test]
fn the_superblock_summary_matches_the_leaf_replay() {
    let mut seed = 0x2545_f491_4f6c_dd1d;
    for (sb4, size) in [(16, 12), (32, 15), (64, 18)] {
        for _ in 0..400 {
            let mi = [
                1 + next(&mut seed) % (2 * sb4),
                sb4 + 1 + next(&mut seed) % (2 * sb4),
            ];
            let mut blocks = vec![[0, 0, sb4, sb4]];
            let mut calls = Vec::new();
            let root = TilePartitionCall::root(0, sb4, BlockSize::new(size).unwrap(), true);
            random_tree(&mut seed, root, mi, (&mut blocks, &mut calls));
            let superblock = ([0, sb4], sb4, 1);
            let mut replay = IbcLocalReplay::new(false);
            let mut pending = &[][..];
            assert!(
                replay
                    .replay_before(([0, 0], sb4, 0), 1, &entry(&blocks), &mut pending)
                    .is_some()
            );
            let mut summary = replay.clone();
            let mut pending = &calls[..];
            assert!(
                replay
                    .replay_before(superblock, blocks.len(), &entry(&blocks), &mut pending)
                    .is_some()
            );
            assert!(pending.is_empty());
            let mut pending = &calls[..];
            let leaf = 1 + next(&mut seed) % blocks.len();
            if leaf < blocks.len() {
                assert!(
                    summary
                        .replay_to_leaf(superblock, leaf, entry(&blocks), &mut pending)
                        .is_some()
                );
            }
            assert!(
                summary
                    .finish_superblock(superblock, blocks.len(), entry(&blocks), &mut pending)
                    .is_some()
            );
            assert!(pending.is_empty());
            for state in [&mut replay, &mut summary] {
                state.buffers.update(0, 2 * sb4);
            }
            assert_eq!(replay.buffers, summary.buffers, "{blocks:?} {calls:?}");
        }
    }
}
