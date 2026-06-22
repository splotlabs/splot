// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the AV2 § 7.11 / § 7.12 spatial MV-context + MV-stack kernel.
//!
//! The worked example is the verified multi-block inter fixture
//! `syn-2frame-inter-mvstack-64x64.ivf`: a 64x64 frame (16x16 MI) split into
//! four 32x32 (8x8 MI) inter blocks in § 5.20.3 partition (DFS) order:
//! block 0 @ MI(0,0) is NEWMV with `mv = (row 0, col 48)` (a +6 full-pel
//! horizontal MV, eighth-pel units, ref 0); blocks 1..3 are NEARMV/NEARESTMV
//! that must predict block 0's MV from the spatial stack. avmdec and dav2d agree
//! the decoded output is bit-exact (md5 `e5b581a55433785c0071b635d5642083`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

/// The fixture's block-0 NEWMV motion vector (eighth-pel, `Mv { row, col }`).
const BLOCK0_MV: Mv = Mv { row: 0, col: 48 };

/// 8x8 MI = a 32x32 block.
const N4_32: usize = 8;
/// 16 MI = a 64x64 superblock height (sb_size 64).
const SB_H4_64: usize = 16;
/// The fixture's 64x64 frame MI dimensions.
const MI_DIM: usize = 16;

fn block_at(mi_row: usize, mi_col: usize) -> MvBlockContext {
    MvBlockContext {
        mi_row,
        mi_col,
        bw4: N4_32,
        bh4: N4_32,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    }
}

/// Records a 32x32 inter block at MI `(r, c)` with the given mode/MV/skip.
fn record_inter(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    grid.record_block(r, c, N4_32, N4_32, true, 0, mode, mv, skip);
}

/// Records the fixture's block 0 (NEWMV @ MI(0,0), 32x32, skip=1) into a fresh grid.
fn grid_with_block0() -> NeighbourMvGrid {
    let mut grid = NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap();
    record_inter(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true);
    grid
}

#[test]
fn block0_has_no_inter_neighbours_so_context_is_zero() {
    // Block 0 @ MI(0,0) is the top-left block with no decoded neighbours, so
    // §7.11.2 nearestMatch == 0 and NewMvCount == 0 -> NewMvContext == 0, and
    // §7.12.2 finds only the zero global-MV fallback candidate. The §5.20.7.2
    // neighbour-buffer contexts (is_inter / skip) are also 0 (NNumBuf == 0).
    let grid = NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap();
    let block0 = block_at(0, 0);

    let ctx = find_mode_ctx(&grid, &block0);
    assert_eq!(ctx.new_mv_context, 0, "top-left NewMvContext");
    assert_eq!(ctx.new_mv_count, 0, "top-left NewMvCount");

    let nctx = block_neighbour_ctx(&grid, &block0);
    assert_eq!(nctx.is_inter_ctx, 0, "no-neighbour is_inter ctx");
    assert_eq!(nctx.skip_ctx, 0, "no-neighbour skip ctx");
    assert!(
        !nctx.has_neighbour,
        "top-left block has no decoded neighbour"
    );

    let stack = find_mv_stack(&grid, &block0, Mv::ZERO);
    assert_eq!(stack.num_mv_found(), 1, "only the zero global-MV fallback");
    assert_eq!(stack.candidate(0), Mv::ZERO, "fallback candidate is zero");
}

#[test]
fn block1_predicts_block0_mv_via_left_neighbour() {
    // Block 1 @ MI(0,8) (the right top block of the SPLIT) has block 0 as its left
    // neighbour (an inter NEWMV block, ref 0, skip 1). §7.12.2 must place block 0's
    // MV at the head of the stack so NEARMV/NEARESTMV (RefMvIdx 0) reconstructs
    // (row 0, col 48).
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32); // MI(0, 8)

    let ctx = find_mode_ctx(&grid, &block1);
    // §7.11.2: leftA = scan_point_ctx(bh4 - 1, -1) = MI(7, 7) -> block 0 (NEWMV,
    // ref match), leftB = scan_point_ctx(0, -1) = MI(0, 7) -> block 0 too. Both
    // probes hit the NEWMV block, so §7.11.3 increments NewMvCount per probe (= 2).
    // nearestMatch = (above 0) + (left 1) = 1; NewMvContext = 1 + 2 = 3.
    assert_eq!(ctx.new_mv_count, 2, "both left probes hit the NEWMV block");
    assert_eq!(ctx.new_mv_context, 3, "left-NEWMV NewMvContext");

    // §5.20.7.2: NPosBuf probes (bottom-left (7,7), top-right (-1,15), left (0,7),
    // above (-1,8)) -> 2 inside (both block 0). Both inter -> is_inter ctx 0; both
    // skip=1 -> skip ctx 2.
    let nctx = block_neighbour_ctx(&grid, &block1);
    assert_eq!(
        nctx.is_inter_ctx, 0,
        "all-inter neighbours -> is_inter ctx 0"
    );
    assert_eq!(nctx.skip_ctx, 2, "two skip=1 neighbours -> skip ctx 2");
    assert!(nctx.has_neighbour, "block 1 has a decoded left neighbour");

    let stack = find_mv_stack(&grid, &block1, Mv::ZERO);
    assert!(stack.num_mv_found() >= 1, "at least one candidate");
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn block2_predicts_block0_mv_via_above_neighbour() {
    // Block 2 @ MI(8,0) (the bottom-left block of the SPLIT) has block 0 as its
    // ABOVE neighbour. §7.12.2 must again place block 0's MV at the head of the
    // stack for the NEARMV reconstruction.
    let grid = grid_with_block0();
    let block2 = block_at(N4_32, 0); // MI(8, 0)

    let ctx = find_mode_ctx(&grid, &block2);
    // §7.11.2: aboveA = scan_point_ctx(-1, bw4 - 1) = MI(7, 7) -> block 0,
    // aboveB = scan_point_ctx(-1, 0) = MI(7, 0) -> block 0. Both probes hit the
    // NEWMV block, so §7.11.3 increments NewMvCount per probe (= 2). nearestMatch =
    // (above 1) + (left 0) = 1; NewMvContext = 1 + 2 = 3.
    assert_eq!(ctx.new_mv_count, 2, "both above probes hit the NEWMV block");
    assert_eq!(ctx.new_mv_context, 3, "above-NEWMV NewMvContext");

    let stack = find_mv_stack(&grid, &block2, Mv::ZERO);
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn block3_predicts_block0_mv_via_above_and_left() {
    // Block 3 @ MI(8,8) (the bottom-right block) has blocks 1 (left, NEARMV) and
    // 2 (above, NEARMV) as neighbours, both carrying block 0's MV (row 0, col 48).
    // After blocks 1 and 2 decode, the stack for block 3 must still place that MV
    // at the head.
    let mut grid = grid_with_block0();
    record_inter(&mut grid, 0, N4_32, NeighbourYMode::Other, BLOCK0_MV, true); // block 1
    record_inter(&mut grid, N4_32, 0, NeighbourYMode::Other, BLOCK0_MV, true); // block 2
    let block3 = block_at(N4_32, N4_32); // MI(8, 8)

    let ctx = find_mode_ctx(&grid, &block3);
    // Neighbours are NEARMV (Other), not NEWMV, so NewMvCount stays 0; nearestMatch
    // = (above 1) + (left 1) = 2; NewMvContext = 2 + 0 = 2.
    assert_eq!(ctx.new_mv_count, 0, "NEARMV neighbours are not NEW MVs");
    assert_eq!(ctx.new_mv_context, 2, "above+left NEARMV NewMvContext");

    let nctx = block_neighbour_ctx(&grid, &block3);
    assert_eq!(nctx.skip_ctx, 2, "two skip=1 neighbours -> skip ctx 2");

    let stack = find_mv_stack(&grid, &block3, Mv::ZERO);
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn intra_neighbour_does_not_contribute() {
    // An intra (is_inter == false) left neighbour must not be added to the stack
    // or counted for the context (the §7.11.3 / §7.12.2.10 IsInters guard), and it
    // makes the §8.3.2 is_inter ctx non-zero.
    let mut grid = NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap();
    grid.record_block(
        0,
        0,
        N4_32,
        N4_32,
        false,
        -1,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
    );
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 0, "intra neighbour gives mode ctx 0");

    let nctx = block_neighbour_ctx(&grid, &block1);
    // Two intra neighbours (NNumBuf == 2, both NIntra) -> is_inter ctx 3.
    assert_eq!(
        nctx.is_inter_ctx, 3,
        "two intra neighbours -> is_inter ctx 3"
    );

    let stack = find_mv_stack(&grid, &block1, Mv::ZERO);
    assert_eq!(stack.num_mv_found(), 1, "intra neighbour not a candidate");
    assert_eq!(
        stack.candidate(0),
        Mv::ZERO,
        "only the zero global fallback"
    );
}

#[test]
fn mismatched_reference_neighbour_does_not_contribute() {
    // A neighbour with a different reference frame must not match the MV stack
    // (§7.12.2.10 / §7.11.3 RefFrames == RefFrame[0] guard), though it is still an
    // inter neighbour for the §8.3.2 is_inter / skip contexts.
    let mut grid = NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap();
    grid.record_block(
        0,
        0,
        N4_32,
        N4_32,
        true,
        1,
        NeighbourYMode::NewMv,
        BLOCK0_MV,
        true,
    );
    let block1 = block_at(0, N4_32); // ref_frame0 == 0, neighbour ref 1

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 0, "ref-mismatch gives mode ctx 0");

    let stack = find_mv_stack(&grid, &block1, Mv::ZERO);
    assert_eq!(
        stack.num_mv_found(),
        1,
        "ref-mismatch neighbour not a candidate"
    );
}

#[test]
fn duplicate_mv_neighbours_merge_to_one_stack_entry() {
    // Two neighbours carrying the same MV merge to a single stack entry
    // (§7.12.2.12 search-stack dedupe), with the global fallback appended only if
    // distinct.
    let mut grid = NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap();
    record_inter(&mut grid, 0, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    record_inter(&mut grid, N4_32, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    let block3 = block_at(N4_32, N4_32);

    let stack = find_mv_stack(&grid, &block3, Mv::ZERO);
    // The single distinct neighbour MV (col 48) + the zero global fallback = 2.
    assert_eq!(
        stack.num_mv_found(),
        2,
        "deduped neighbour MV + zero fallback"
    );
    assert_eq!(stack.candidate(0), BLOCK0_MV);
    assert_eq!(stack.candidate(1), Mv::ZERO);
}

#[test]
fn clamp_keeps_small_mvs_unchanged() {
    // The fixture's MV (col 48 = +6 full pels) is far inside the §5.20.9.4 /
    // §5.20.9.5 clamp bounds, so clamping is a no-op.
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);
    let stack = find_mv_stack(&grid, &block1, Mv::ZERO);
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "clamp leaves the small MV intact"
    );
}

#[test]
fn record_block_marks_every_covered_cell() {
    // The grid records the MV into every MI cell the block covers, so a probe at
    // any cell of the neighbour returns the block.
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);
    // leftA probes (bh4 - 1, -1) = MI(7, 7), leftB probes (0, -1) = MI(0, 7); both
    // must see block 0.
    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 3, "both left probes see block 0");
}

// --- DECODE-INTER-GRID-SPATIAL: 2-D superblock-grid availability ---------------

/// A full 64x64 superblock is 16 MI units wide / high.
const N4_64: usize = 16;
/// The 128x128 (2x2 superblock) grid MI dimensions.
const GRID_MI_DIM: usize = 32;

/// One full-64x64-superblock block context in the 128x128 grid (`bw4 = bh4 = 16`,
/// `sb_h4 = 16`).
fn sb_block_at(mi_row: usize, mi_col: usize) -> MvBlockContext {
    MvBlockContext {
        mi_row,
        mi_col,
        bw4: N4_64,
        bh4: N4_64,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        mi_rows: GRID_MI_DIM,
        mi_cols: GRID_MI_DIM,
    }
}

/// Records a full-64x64-superblock inter block into the grid.
fn record_sb(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    grid.record_block(r, c, N4_64, N4_64, true, 0, mode, mv, skip);
}

#[test]
fn second_sb_row_block_predicts_above_sb_mv_across_sb_row_boundary() {
    // The 2-D-grid gap: a superblock in SB ROW 1 must predict an SB-ROW-0
    // superblock's MV across the superblock-row boundary via the frame-wide
    // §7.12.2 spatial stack. The §5.20.2.1 raster loop decodes SB row 0 fully
    // before SB row 1, so SB0 @ MI(0,0) (NEWMV, col 48) is already recorded when
    // SB2 @ MI(16,0) decodes; SB2's §7.12.2 above probes (deltaRow == -1) land in
    // SB0's decoded bottom edge -> RefMvIdx 0 reconstructs SB0's MV. This is the
    // exact case the single-SB-row brick (DECODE-INTER-MULTI-SB-SPATIAL) deferred.
    let mut grid = NeighbourMvGrid::new(GRID_MI_DIM, GRID_MI_DIM).unwrap();
    record_sb(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true);
    let sb2 = sb_block_at(N4_64, 0); // MI(16, 0), SB row 1, col 0

    let ctx = find_mode_ctx(&grid, &sb2);
    // §7.11.2: aboveA = scan_point_ctx(-1, bw4 - 1) = MI(15, 15) -> SB0,
    // aboveB = scan_point_ctx(-1, 0) = MI(15, 0) -> SB0. Both above probes hit the
    // NEWMV SB across the SB-row boundary -> NewMvCount 2, nearestMatch (above 1) =
    // 1, NewMvContext = 1 + 2 = 3.
    assert_eq!(
        ctx.new_mv_count, 2,
        "both above probes hit SB0 across SB row"
    );
    assert_eq!(ctx.new_mv_context, 3, "above-SB NewMvContext");

    let nctx = block_neighbour_ctx(&grid, &sb2);
    assert!(
        nctx.has_neighbour,
        "SB2 in SB row 1 has a decoded above neighbour (SB0)"
    );

    let stack = find_mv_stack(&grid, &sb2, Mv::ZERO);
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts SB0's MV across the SB-row boundary"
    );
}

#[test]
fn undecoded_later_sb_column_yields_no_candidate() {
    // §7.12.2.6 availability: the scan invokes the add-reference-MV step only when
    // `is_inside(mvRow, mvCol)` AND `RefFrames[mvRow][mvCol][0] has been written for
    // this frame`. A grid cell for a not-yet-decoded superblock returns `None`
    // (unwritten) and must NOT contribute a candidate, so a non-top SB whose only
    // would-be neighbour is a LATER (undecoded) SB column finds only the zero
    // global-MV fallback. Here SB1 @ MI(0,16) (SB row 0, col 1) has its LEFT
    // neighbour SB0 undecoded (empty grid) and the SB to its right (MI(0,16)+) also
    // undecoded: the stack must be the zero fallback alone.
    let grid = NeighbourMvGrid::new(GRID_MI_DIM, GRID_MI_DIM).unwrap();
    let sb1 = sb_block_at(0, N4_64); // MI(0, 16), no decoded neighbour at all

    let nctx = block_neighbour_ctx(&grid, &sb1);
    assert!(
        !nctx.has_neighbour,
        "an SB whose neighbours are all undecoded has no neighbour"
    );

    let stack = find_mv_stack(&grid, &sb1, Mv::ZERO);
    assert_eq!(
        stack.num_mv_found(),
        1,
        "undecoded neighbours contribute no candidate"
    );
    assert_eq!(
        stack.candidate(0),
        Mv::ZERO,
        "only the zero global-MV fallback"
    );
}

#[test]
fn bottom_right_sb_predicts_from_decoded_above_and_left() {
    // The last superblock in the 2x2 grid, SB3 @ MI(16,16) (SB row 1, col 1),
    // decodes after SB0/SB1/SB2 in raster order. Its left neighbour (SB2 @
    // MI(16,0)) and above neighbour (SB1 @ MI(0,16)) are both decoded and carry
    // SB0's MV (col 48), so the §7.12.2 stack heads with that MV — the full 2-D
    // grid reconstruction has every later SB predicting from its already-decoded
    // raster-order neighbours.
    let mut grid = NeighbourMvGrid::new(GRID_MI_DIM, GRID_MI_DIM).unwrap();
    record_sb(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true); // SB0
    record_sb(&mut grid, 0, N4_64, NeighbourYMode::Other, BLOCK0_MV, true); // SB1
    record_sb(&mut grid, N4_64, 0, NeighbourYMode::Other, BLOCK0_MV, true); // SB2
    let sb3 = sb_block_at(N4_64, N4_64); // MI(16, 16)

    let nctx = block_neighbour_ctx(&grid, &sb3);
    assert!(
        nctx.has_neighbour,
        "SB3 has decoded above + left neighbours"
    );

    let stack = find_mv_stack(&grid, &sb3, Mv::ZERO);
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts the propagated MV in the bottom-right SB"
    );
}
