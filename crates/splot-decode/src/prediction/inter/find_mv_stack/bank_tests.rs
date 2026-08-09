// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn warp_bank_hit_keeps_the_original_translation_and_evicts_oldest() {
    let mut bank = WarpParamBank::new();
    let base = |trans: i32, scale: i32| [trans, -trans, 65536 + scale, 0, 0, 65536 - scale];
    bank.update(0, base(1 << 16, 64));
    bank.update(0, base(2 << 16, 128));
    bank.update(0, base(9 << 16, 64));
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.slots[0],
        base(1 << 16, 64),
        "params_equal on [2..6) rotates the EXISTING entry to the tail; its translation is kept"
    );
    assert_eq!(stack.slots[1], base(2 << 16, 128));
    assert_eq!(stack.num_found, 2, "the hit did not grow the ring");

    for scale in [192, 256, 320] {
        bank.update(0, base(3 << 16, scale));
    }
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(stack.num_found, 4, "ring capacity");
    assert_eq!(
        stack.slots[3],
        base(1 << 16, 64),
        "oldest surviving entry after the size-4 ring evicted the front"
    );
}

#[test]
fn ref_mv_bank_same_mvs_refresh_preserves_original_cwp_weight() {
    let mut bank = RefMvBank::new();
    let mv0 = Mv { row: 0, col: 8 };
    let mv1 = Mv { row: 0, col: -8 };
    bank.update(0, Some(1), mv0, Some(mv1), 10, false);
    bank.update(0, Some(1), mv0, Some(mv1), CWP_EQUAL, false);

    let mut entries = FixedStack::new();
    let mut prune_count = 0;
    let block = MvBlockContext {
        mi_row: 0,
        mi_col: 0,
        bw4: 8,
        bh4: 8,
        sb_h4: 16,
        ref_frame0: 0,
        ref_frame1: Some(1),
        mi_rows: 16,
        mi_cols: 16,
    };
    bank.fill_compound(
        &block,
        &mut entries,
        MAX_REF_MV_STACK_SIZE,
        &mut prune_count,
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].candidate.mvs, [mv0, mv1]);
    assert_eq!(entries[0].candidate.cwp_weight, 10);
}

#[test]
fn shared_ref_mv_bank_ordinary_entries_evict_intrabc_candidate() {
    let mut bank = RefMvBank::new();
    let intrabc = Mv { row: 0, col: -256 };
    bank.update(INTRABC_REF_FRAME, None, intrabc, None, CWP_EQUAL, false);
    for (index, mv) in [
        Mv { row: -72, col: 176 },
        Mv { row: 98, col: 58 },
        Mv { row: 0, col: 56 },
        Mv { row: -2, col: 56 },
    ]
    .into_iter()
    .enumerate()
    {
        bank.update(8, None, mv, None, CWP_EQUAL, false);
        let expected = usize::from(index < 3);
        assert_eq!(bank.intrabc_candidates().len(), expected);
    }
}

#[test]
fn shared_ref_mv_bank_intrabc_candidates_are_newest_first() {
    let mut bank = RefMvBank::new();
    let older = Mv { row: -512, col: 0 };
    let newer = Mv { row: 0, col: -512 };
    bank.update(INTRABC_REF_FRAME, None, older, None, CWP_EQUAL, false);
    bank.update(INTRABC_REF_FRAME, None, newer, None, CWP_EQUAL, false);
    assert_eq!(bank.intrabc_candidates(), vec![newer, older]);

    bank.update(INTRABC_REF_FRAME, None, older, None, CWP_EQUAL, false);
    assert_eq!(bank.intrabc_candidates(), vec![older, newer]);
}

#[test]
fn shared_ref_mv_bank_seeds_intrabc_from_previous_superblock_row() {
    let mv = Mv { row: -512, col: 0 };
    let mut grid = NeighbourMvGrid::new(270, 480).unwrap();
    grid.record_block(
        240,
        128,
        8,
        16,
        true,
        INTRABC_REF_FRAME,
        None,
        false,
        mv,
        true,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
    let mut inter_bank = RefMvBank::new();
    inter_bank.reset_for_leaf(&grid, 256, 128, 32, true);
    assert_eq!(inter_bank.intrabc_candidates(), vec![mv]);

    let mut intra_bank = RefMvBank::new();
    intra_bank.reset_for_leaf(&grid, 256, 128, 32, false);
    assert!(intra_bank.intrabc_candidates().is_empty());
}

#[test]
fn shared_ref_mv_bank_late_leaf_uses_only_existing_unit_budget() {
    let grid = NeighbourMvGrid::new(64, 480).unwrap();
    let mv = Mv { row: 0, col: -512 };
    let mut ordered = RefMvBank::new();
    ordered.reset_for_leaf(&grid, 0, 288, 32, true);
    ordered.update_count_for_non_inter(0, 288, 16, 16, 32);
    ordered.reset_for_leaf(&grid, 22, 288, 32, true);
    ordered.update_for_block(
        INTRABC_REF_FRAME,
        None,
        mv,
        None,
        CWP_EQUAL,
        22,
        288,
        16,
        8,
        32,
    );
    ordered.reset_for_leaf(&grid, 28, 320, 32, true);
    assert_eq!(ordered.intrabc_candidates(), vec![mv]);

    let mut late = RefMvBank::new();
    late.reset_for_leaf(&grid, 22, 288, 32, true);
    late.update_for_block(
        INTRABC_REF_FRAME,
        None,
        mv,
        None,
        CWP_EQUAL,
        22,
        288,
        16,
        8,
        32,
    );
    assert!(late.intrabc_candidates().is_empty());
}

#[test]
fn warp_bank_clears_per_superblock_row_and_reseeds_per_superblock() {
    let mut grid = NeighbourMvGrid::new(64, 64).unwrap();
    let above = [4 << 16, -6 << 16, 65536 + 512, 64, -128, 65536];
    grid.record_warp_block(
        15,
        4,
        2,
        1,
        0,
        false,
        Mv { row: 4, col: -12 },
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::ExtendWarp,
        above,
        BlockPrecisionRecord::default(),
    );
    let mut bank = WarpParamBank::new();
    let carried = [7 << 16, 0, 65536 - 320, 0, 0, 65536 + 448];
    bank.reset_for_leaf(&grid, 0, 0, 16);
    bank.update(0, carried);

    bank.reset_for_leaf(&grid, 0, 16, 16);
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.num_found, 1,
        "a same-row superblock transition keeps the bank contents"
    );

    bank.reset_for_leaf(&grid, 16, 0, 16);
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.slots[0], above,
        "the new superblock row cleared contents, then re-seeded the warp neighbour from the row above"
    );
    assert_eq!(
        stack.num_found, 1,
        "carried entry was cleared on the row transition"
    );
}

#[test]
fn warp_param_bank_updates_key_each_list_by_its_reference() {
    let model0 = [320, -640, 65_536 + 256, -128, 192, 65_536 - 320];
    let model1 = [-960, 480, 65_536 - 512, 96, -64, 65_536 + 448];
    let mut bank = WarpParamBank::new();
    bank.update(0, model0);
    bank.update(1, model1);
    let mut list0 = WarpParamStack::new();
    bank.fill(0, &mut list0);
    let mut list1 = WarpParamStack::new();
    bank.fill(1, &mut list1);
    assert_eq!((list0.num_found, list0.slots[0]), (1, model0));
    assert_eq!((list1.num_found, list1.slots[0]), (1, model1));
}
