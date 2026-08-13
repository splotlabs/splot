// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// A weighted spatial candidate with the default `ADJACENT_SMVP_WEIGHT` (1): the
/// weight most modelled positions place.
fn adj(mv: Mv) -> WeightedBv {
    WeightedBv {
        mv,
        weight: ADJACENT_SMVP_WEIGHT,
    }
}

/// A weighted spatial candidate with an explicit weight.
const fn wbv(mv: Mv, weight: u16) -> WeightedBv {
    WeightedBv { mv, weight }
}

fn spatial_intrabc_scan(
    geometry: SpatialScanGeometry,
    lookup: impl Fn(usize, usize) -> Option<Mv>,
    is_coded: impl Fn(usize, usize) -> bool,
) -> SpatialIntrabcScan {
    spatial_intrabc_scan_with_base_col(geometry, lookup, is_coded, |_, _| None)
}

/// Mission-stream frame-0 SB row 0, mib_size 32 (128x128 SB). Walks the first three
/// reachable IntrABC blocks and checks the bank + stack against the AVM
/// `av2_find_mv_refs` dump.
fn frontier_geometry(mi_row: usize, mi_col: usize) -> IntrabcStackGeometry {
    IntrabcStackGeometry {
        mi_row,
        mi_col,
        n4w: 8,  // 32 px wide
        n4h: 16, // 64 px high
        sb_samples: 128,
        frame_w: 1920,
        frame_h: 1080,
    }
}

/// The DRL selection every frontier case decodes: `max_bvp_drl_bits_minus_1 = 2`
/// (four stack entries) at the given `ref_bv_idx`.
fn drl(index: usize) -> IntrabcRefSelection {
    IntrabcRefSelection::new(2, index).expect("frontier ref_bv_idx is within the DRL bound")
}

#[test]
fn frontier_mi_0_112_is_default_only() {
    let stack = build_intrabc_ref_mv_stack_from_candidates(
        &[Mv { row: -512, col: 0 }],
        frontier_geometry(0, 112),
        true,
        &[],
        0,
    );
    assert_eq!(
        stack,
        vec![
            Mv { row: -1024, col: 0 },
            Mv { row: 0, col: -3072 },
            Mv { row: -512, col: 0 },
            Mv { row: 0, col: -256 },
        ]
    );
}

#[test]
fn frontier_mi_0_232_is_reordered_by_bank() {
    let stack = build_intrabc_ref_mv_stack_from_candidates(
        &[Mv { row: 0, col: -256 }, Mv { row: -512, col: 0 }],
        frontier_geometry(0, 232),
        true,
        &[],
        0,
    );
    assert_eq!(
        stack,
        vec![
            Mv { row: 0, col: -256 },
            Mv { row: -1024, col: 0 },
            Mv { row: 0, col: -3072 },
            Mv { row: -512, col: 0 },
        ]
    );
}

fn bounds(mi_row: i32, mi_col: i32) -> RmbCandBounds {
    RmbCandBounds {
        mi_row,
        mi_col,
        block_w: 32,
        block_h: 64,
        frame_w: 1920,
        frame_h: 1080,
    }
}

#[test]
fn check_rmb_cand_rejects_frame_boundary_top_edge() {
    assert!(!check_rmb_cand(
        Mv { row: -512, col: 0 },
        &[],
        bounds(0, 112),
        &mut 0,
    ));
}

#[test]
fn check_rmb_cand_admits_in_bounds_candidate() {
    assert!(check_rmb_cand(
        Mv { row: 0, col: -256 },
        &[],
        bounds(0, 232),
        &mut 0,
    ));
}

#[test]
fn check_rmb_cand_rejects_duplicate() {
    let cand = Mv { row: 0, col: -256 };
    assert!(!check_rmb_cand(cand, &[cand], bounds(0, 232), &mut 0));
}

#[test]
fn check_rmb_cand_appends_duplicate_once_pruning_budget_is_spent() {
    let cand = Mv { row: 0, col: -256 };
    let mut spent = MAX_PR_NUM;
    assert!(check_rmb_cand(cand, &[cand], bounds(0, 232), &mut spent));
}

#[test]
fn check_rmb_cand_bounds_rejection_still_spends_the_budget() {
    let out_of_frame = Mv { row: -512, col: 0 };
    let in_stack = Mv { row: 0, col: -256 };
    let mut spent = MAX_PR_NUM - 1;
    assert!(!check_rmb_cand(
        out_of_frame,
        &[in_stack],
        bounds(0, 112),
        &mut spent,
    ));
    assert_eq!(spent, MAX_PR_NUM);
    assert!(check_rmb_cand(
        in_stack,
        &[in_stack],
        bounds(0, 232),
        &mut spent
    ));
}

/// No spatial candidate.
fn no_spatial() -> SpatialIntrabcScan {
    SpatialIntrabcScan {
        candidates: FixedStack::new(),
        nearest_len: 0,
        comparisons: 0,
    }
}

#[test]
fn admission_admits_frontier_mi_0_112_default_only() {
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[Mv { row: -512, col: 0 }],
            frontier_geometry(0, 112),
            &no_spatial(),
            true,
            DrlReorderMode::Always,
            drl(3),
        ),
        Mv { row: 0, col: -256 }
    );
}

#[test]
fn admission_selects_frontier_mi_0_232_bank_reordered_bv() {
    let decide = |enable_refmvbank| {
        select_intrabc_ref_mv_from_candidates(
            &[Mv { row: 0, col: -256 }, Mv { row: -512, col: 0 }],
            frontier_geometry(0, 232),
            &no_spatial(),
            enable_refmvbank,
            DrlReorderMode::Always,
            drl(2),
        )
    };
    assert_eq!(decide(true), Mv { row: 0, col: -3072 });
    assert_eq!(decide(false), Mv { row: -512, col: 0 });
}

#[test]
fn admission_selects_frontier_mi_0_240_spatial_bv() {
    let bank_candidates = [
        Mv { row: 0, col: -3072 },
        Mv { row: 0, col: -256 },
        Mv { row: -512, col: 0 },
    ];
    let spatial = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([adj(Mv { row: 0, col: -3072 })]),
        nearest_len: 1,
        comparisons: 0,
    };
    let stack = build_intrabc_ref_mv_stack_from_candidates(
        &bank_candidates,
        frontier_geometry(0, 240),
        true,
        &[Mv { row: 0, col: -3072 }],
        0,
    );
    assert_eq!(
        stack,
        vec![
            Mv { row: 0, col: -3072 },
            Mv { row: 0, col: -256 },
            Mv { row: -1024, col: 0 },
            Mv { row: 0, col: -3072 },
        ]
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &bank_candidates,
            frontier_geometry(0, 240),
            &spatial,
            true,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: 0, col: -3072 }
    );
}

#[test]
fn admission_forced_swap_places_max_weight_at_slot0() {
    let unsorted = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([
            wbv(Mv { row: 0, col: -64 }, 0),
            wbv(Mv { row: -512, col: 0 }, 1),
        ]),
        nearest_len: 2,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &unsorted,
            false, // bank fill OFF: the stack is spatial-prefix + defaults only.
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -512, col: 0 },
        "the §7.12.2.19 sort must move the weight-1 candidate to slot 0 (not a passthrough)"
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &unsorted,
            false,
            DrlReorderMode::Always,
            drl(1),
        ),
        Mv { row: 0, col: -64 }
    );
}

#[test]
fn admission_no_op_swap_when_slot0_already_max() {
    let tie = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([
            wbv(Mv { row: -1024, col: 0 }, 1),
            wbv(Mv { row: -512, col: 0 }, 1),
        ]),
        nearest_len: 2,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &tie,
            false,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -1024, col: 0 },
        "equal weights must keep the lowest index in slot 0 (strict `>` tie-break)"
    );
    let frontier = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([
            wbv(Mv { row: -1024, col: 0 }, 3),
            wbv(Mv { row: -512, col: 0 }, 1),
        ]),
        nearest_len: 2,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &frontier,
            false,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -1024, col: 0 },
        "the max-weight slot-0 entry stays put (no swap)"
    );
}

#[test]
fn admission_sort_respects_drl_reorder_mode() {
    let candidates = FixedStack::from_entries([
        wbv(Mv { row: 0, col: -64 }, 0),
        wbv(Mv { row: -512, col: 0 }, 1),
    ]);
    let scan = SpatialIntrabcScan {
        candidates,
        nearest_len: 2,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &scan,
            false,
            DrlReorderMode::Disabled,
            drl(0),
        ),
        Mv { row: 0, col: -64 },
        "DRL_REORDER_DISABLED must NOT sort (slot 0 stays scan-order-first)"
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &scan,
            false,
            DrlReorderMode::Constraint,
            drl(0),
        ),
        Mv { row: 0, col: -64 },
        "DRL_REORDER_CONSTRAINT with nearest < 4 must NOT sort"
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &scan,
            false,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -512, col: 0 },
        "DRL_REORDER_ALWAYS must sort the weight-1 candidate into slot 0"
    );
}

#[test]
fn admission_sort_leaves_scan_col_tail_outside_nearest_prefix() {
    let scan = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([
            wbv(Mv { row: 0, col: -64 }, 0),
            wbv(Mv { row: -512, col: 0 }, 1),
        ]),
        nearest_len: 1,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &scan,
            false,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: 0, col: -64 },
        "§7.12.2.19 sorts only the step-15 nearest prefix, not scan-col tail entries"
    );
}

#[test]
fn admission_admits_single_spatial_candidate() {
    let one = SpatialIntrabcScan {
        candidates: FixedStack::from_entries([adj(Mv { row: 0, col: -64 })]),
        nearest_len: 1,
        comparisons: 0,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            frontier_geometry(0, 240),
            &one,
            true,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: 0, col: -64 }
    );
}

#[test]
fn sort_nearest_moves_max_weight_to_slot0_strict() {
    let mut swap = vec![wbv(Mv { row: 1, col: 1 }, 0), wbv(Mv { row: 2, col: 2 }, 1)];
    sort_nearest_max_weight_to_slot0(&mut swap);
    assert_eq!(swap[0], wbv(Mv { row: 2, col: 2 }, 1));
    assert_eq!(swap[1], wbv(Mv { row: 1, col: 1 }, 0));
    let mut tie = vec![wbv(Mv { row: 1, col: 1 }, 2), wbv(Mv { row: 2, col: 2 }, 2)];
    sort_nearest_max_weight_to_slot0(&mut tie);
    assert_eq!(tie[0], wbv(Mv { row: 1, col: 1 }, 2));
    let mut already = vec![wbv(Mv { row: 1, col: 1 }, 3), wbv(Mv { row: 2, col: 2 }, 1)];
    sort_nearest_max_weight_to_slot0(&mut already);
    assert_eq!(already[0], wbv(Mv { row: 1, col: 1 }, 3));
    let mut empty: Vec<WeightedBv> = Vec::new();
    sort_nearest_max_weight_to_slot0(&mut empty);
    assert!(empty.is_empty());
}

#[test]
fn spatial_scan_adds_left_neighbour_and_admits_modelled_above_neighbour() {
    let geom = SpatialScanGeometry {
        mi_row: 4,
        mi_col: 8,
        n4w: 4,
        n4h: 4,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    };
    let left_only = spatial_intrabc_scan(
        geom,
        |row, col| (row == 7 && col == 7).then_some(Mv { row: 0, col: -64 }),
        |_, _| false,
    );
    assert_eq!(left_only.candidates[..], [adj(Mv { row: 0, col: -64 })]);
    let above = spatial_intrabc_scan(
        geom,
        |row, col| (row == 3 && col == 8).then_some(Mv { row: -8, col: 0 }),
        |_, _| false,
    );
    assert_eq!(above.candidates[..], [adj(Mv { row: -8, col: 0 })]);
    let deep_left = spatial_intrabc_scan(
        geom,
        |row, col| (row == 4 && col == 5).then_some(Mv { row: 0, col: -512 }),
        |_, _| false,
    );
    assert!(deep_left.candidates.is_empty());
    let scan_col = spatial_intrabc_scan_with_base_col(
        geom,
        |row, col| (row == 4 && col == 5).then_some(Mv { row: 0, col: -512 }),
        |_, _| false,
        |row, col| {
            if row == 4 && col == 5 {
                Some(4)
            } else if row == 4 && col == 7 {
                Some(6)
            } else {
                None
            }
        },
    );
    assert_eq!(scan_col.candidates[..], [wbv(Mv { row: 0, col: -512 }, 0)]);
    assert_eq!(scan_col.nearest_len, 0);
}

/// Mission-stream frame-0 MI(32,56) geometry for the § 7.12.2.1 step-8 SB-border probe.
/// mib_size 32, so MiRow 32 sits on a horizontal SB border (`32 % 32 == 0`).
fn frontier_mi_32_56_scan_geometry() -> SpatialScanGeometry {
    SpatialScanGeometry {
        mi_row: 32,
        mi_col: 56,
        n4w: 8,
        n4h: 16,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    }
}

#[test]
fn spatial_scan_admits_frontier_mi_32_56_step8_above_neighbour() {
    let scan = spatial_intrabc_scan(
        frontier_mi_32_56_scan_geometry(),
        |row, col| (row == 31 && col == 62).then_some(Mv { row: -512, col: 0 }),
        |_, _| false,
    );
    assert_eq!(scan.candidates[..], [adj(Mv { row: -512, col: 0 })]);
}

#[test]
fn admission_selects_frontier_mi_32_56_step8_bv() {
    let spatial = spatial_intrabc_scan(
        frontier_mi_32_56_scan_geometry(),
        |row, col| (row == 31 && col == 62).then_some(Mv { row: -512, col: 0 }),
        |_, _| false,
    );
    let geometry = IntrabcStackGeometry {
        mi_row: 32,
        mi_col: 56,
        n4w: 8,
        n4h: 16,
        sb_samples: 128,
        frame_w: 1920,
        frame_h: 1080,
    };
    let stack = build_intrabc_ref_mv_stack_from_candidates(
        &[],
        geometry,
        true,
        &[Mv { row: -512, col: 0 }],
        0,
    );
    assert_eq!(
        stack,
        vec![
            Mv { row: -512, col: 0 },
            Mv { row: -1024, col: 0 },
            Mv { row: 0, col: -3072 },
            Mv { row: -512, col: 0 },
        ]
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            geometry,
            &spatial,
            true,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -512, col: 0 }
    );
}

#[test]
fn spatial_scan_ignores_non_table_above_row_column() {
    let scan = spatial_intrabc_scan(
        frontier_mi_32_56_scan_geometry(),
        |row, col| (row == 31 && col == 60).then_some(Mv { row: 7, col: -99 }),
        |_, _| false,
    );
    assert!(scan.candidates.is_empty());
    let scan = spatial_intrabc_scan(
        frontier_mi_32_56_scan_geometry(),
        |row, col| {
            if row == 31 && col == 62 {
                Some(Mv { row: -512, col: 0 })
            } else if row == 31 && col == 60 {
                Some(Mv { row: 7, col: -99 })
            } else {
                None
            }
        },
        |_, _| false,
    );
    assert_eq!(scan.candidates[..], [adj(Mv { row: -512, col: 0 })]);
}

fn assert_sb_border_step8_aligns_odd_mi_col(
    mi_row: usize,
    mi_col: usize,
    above_neighbour_col: usize,
) {
    let geom = SpatialScanGeometry {
        mi_row,
        mi_col,
        n4w: 8,
        n4h: 16,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    };
    assert_eq!(step8_above_row_column(&geom), Some(above_neighbour_col));
    let above = mi_row - 1;
    let scan = spatial_intrabc_scan(
        geom,
        |row, col| (row == above && col == above_neighbour_col).then_some(Mv { row: -512, col: 0 }),
        |_, _| false,
    );
    assert_eq!(scan.candidates[..], [adj(Mv { row: -512, col: 0 })]);
}

#[test]
fn spatial_scan_aligns_odd_mi_col_sb_border() {
    assert_sb_border_step8_aligns_odd_mi_col(32, 57, 62);
}

/// Mission-stream frame-0 MI(48,56) geometry: mib_size 32, MiRow 48, `48 % 32 == 16 != 0`
/// -> NOT an SB border, so the within-SB 4x4-resolution above-row scan applies.
/// BLOCK_32X64 (bw4 = 8, bh4 = 16), the new frontier-class block.
fn frontier_mi_48_56_scan_geometry() -> SpatialScanGeometry {
    SpatialScanGeometry {
        mi_row: 48,
        mi_col: 56,
        n4w: 8,
        n4h: 16,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    }
}

#[test]
fn spatial_scan_admits_frontier_mi_48_56_within_sb_step8() {
    let scan = spatial_intrabc_scan(
        frontier_mi_48_56_scan_geometry(),
        |row, col| (row == 47 && (56..=63).contains(&col)).then_some(Mv { row: -512, col: 0 }),
        |_, _| true,
    );
    assert_eq!(scan.candidates[..], [wbv(Mv { row: -512, col: 0 }, 2)]);
}

#[test]
fn spatial_scan_step12_top_right_respects_has_top_right() {
    let geom = SpatialScanGeometry {
        mi_row: 20,
        mi_col: 8,
        n4w: 4,
        n4h: 4,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    };
    let bv = Mv { row: -8, col: -8 };
    let not_coded = spatial_intrabc_scan(geom, |_, _| None, |_, _| false);
    assert!(not_coded.candidates.is_empty());
    let coded = spatial_intrabc_scan(
        geom,
        |row, col| (row == 19 && col == 12).then_some(bv),
        |row, col| row == 19 && col == 12,
    );
    assert_eq!(coded.candidates[..], [adj(bv)]);
}

#[test]
fn spatial_scan_disables_step10_for_block_width_4_within_sb() {
    let narrow = SpatialScanGeometry {
        mi_row: 20,
        mi_col: 8,
        n4w: 1,
        n4h: 4,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    };
    let bv = Mv { row: -8, col: 0 };
    let scan = spatial_intrabc_scan(
        narrow,
        |row, col| (row == 19 && col == 8).then_some(bv),
        |_, _| false,
    );
    assert_eq!(
        scan.candidates[..],
        [wbv(bv, 1)],
        "step 10 disabled for bw4 == 1: the above candidate keeps step-8 weight 1"
    );
    let wide = SpatialScanGeometry { n4w: 2, ..narrow };
    let wide_scan = spatial_intrabc_scan(
        wide,
        |row, col| (row == 19 && (8..=9).contains(&col)).then_some(bv),
        |_, _| false,
    );
    assert_eq!(
        wide_scan.candidates[..],
        [wbv(bv, 2)],
        "bw4 >= 2 enables step 10: step 8 + step 10 accumulate weight 2"
    );
}

#[test]
fn spatial_scan_disables_step10_for_block_width_4_sb_border() {
    let narrow = SpatialScanGeometry {
        mi_row: 32,
        mi_col: 8,
        n4w: 1,
        n4h: 4,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    };
    let bv = Mv { row: -8, col: 0 };
    let scan = spatial_intrabc_scan(
        narrow,
        |row, col| (row == 31 && col == 8).then_some(bv),
        |_, _| false,
    );
    assert_eq!(
        scan.candidates[..],
        [wbv(bv, 1)],
        "step 10 disabled for bw4 == 1 on the SB border: the above candidate keeps step-8 weight 1"
    );
    let wide = SpatialScanGeometry { n4w: 8, ..narrow };
    let wide_scan = spatial_intrabc_scan(
        wide,
        |row, col| (row == 31 && (col == 8 || col == 14)).then_some(bv),
        |_, _| false,
    );
    assert_eq!(
        wide_scan.candidates[..],
        [wbv(bv, 2)],
        "bw4 >= 4 enables step 10 on the SB border: step 8 + step 10 accumulate weight 2"
    );
}

#[test]
fn spatial_scan_dedups_same_left_neighbour() {
    let geom = SpatialScanGeometry {
        mi_row: 0,
        mi_col: 8,
        n4w: 8,
        n4h: 16,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    };
    let scan = spatial_intrabc_scan(
        geom,
        |row, col| (col == 7 && row < 16).then_some(Mv { row: 0, col: -3072 }),
        |_, _| false,
    );
    assert_eq!(scan.candidates[..], [wbv(Mv { row: 0, col: -3072 }, 2)]);
}

/// Mission-stream frame-0 MI(192,112) geometry: MiRow 192, `192 % 32 == 0` -> SB border;
/// MiCol 112 even; BLOCK_64X32 (bw4 = 16, bh4 = 8) — the §7.12.2.19 weight-sort
/// frontier.
fn frontier_mi_192_112_scan_geometry() -> SpatialScanGeometry {
    SpatialScanGeometry {
        mi_row: 192,
        mi_col: 112,
        n4w: 16,
        n4h: 8,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    }
}

#[test]
fn admission_admits_frontier_mi_192_112_no_op_weight_sort() {
    let scan = spatial_intrabc_scan(
        frontier_mi_192_112_scan_geometry(),
        |row, col| {
            if (192..=199).contains(&row) && col == 111 {
                Some(Mv { row: -1024, col: 0 })
            } else if row == 191 && col == 126 {
                Some(Mv { row: -512, col: 0 })
            } else {
                None
            }
        },
        |_, _| false,
    );
    assert_eq!(
        scan.candidates[..],
        [
            wbv(Mv { row: -1024, col: 0 }, 2),
            wbv(Mv { row: -512, col: 0 }, 1),
        ],
        "(-1024,0) accumulates step 7 + step 9 weight (2); (-512,0) step 8 weight (1)"
    );
    let geometry = IntrabcStackGeometry {
        mi_row: 192,
        mi_col: 112,
        n4w: 16,
        n4h: 8,
        sb_samples: 128,
        frame_w: 1920,
        frame_h: 1080,
    };
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            geometry,
            &scan,
            true,
            DrlReorderMode::Always,
            drl(1),
        ),
        Mv { row: -512, col: 0 },
    );
    assert_eq!(
        select_intrabc_ref_mv_from_candidates(
            &[],
            geometry,
            &scan,
            true,
            DrlReorderMode::Always,
            drl(0),
        ),
        Mv { row: -1024, col: 0 },
    );
}

#[test]
fn spatial_scan_admits_sb_border_even_mi_col_step14() {
    let geom = SpatialScanGeometry {
        mi_row: 32,
        mi_col: 320,
        n4w: 8,
        n4h: 16,
        mi_rows: 270,
        mi_cols: 480,
        sb_size4: 32,
    };
    let scan = spatial_intrabc_scan(
        geom,
        |row, col| (row == 31 && col == 318).then_some(Mv { row: 0, col: -256 }),
        |_, _| false,
    );
    assert_eq!(scan.candidates[..], [wbv(Mv { row: 0, col: -256 }, 0)]);
}

#[test]
fn spatial_scan_uses_aligned_sb_border_offset_for_weight() {
    let geom = SpatialScanGeometry {
        mi_row: 16,
        mi_col: 9,
        n4w: 1,
        n4h: 2,
        mi_rows: 32,
        mi_cols: 64,
        sb_size4: 16,
    };
    let bv = Mv { row: -64, col: 64 };
    let scan = spatial_intrabc_scan(
        geom,
        |row, col| (row == 15 && (col == 8 || col == 10)).then_some(bv),
        |row, col| row == 15 && col == 10,
    );
    assert_eq!(
        scan.candidates[..],
        [wbv(bv, 1)],
        "the aligned step-8 (-1,-1) probe has weight 0; top-right contributes weight 1"
    );
}

/// A synthetic geometry at an even MiCol (so the SB-border 8x8 alignment is a
/// no-op), large enough that every probe column stays inside the tile.
fn generic_scan_geom(mi_row: usize, mi_col: usize, bw4: usize) -> SpatialScanGeometry {
    SpatialScanGeometry {
        mi_row,
        mi_col,
        n4w: bw4,
        n4h: 4,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    }
}

fn scan_single_intrabc_probe(
    geom: SpatialScanGeometry,
    probe_row: usize,
    probe_col: usize,
    mv: Mv,
) -> SpatialIntrabcScan {
    spatial_intrabc_scan(
        geom,
        move |row, col| (row == probe_row && col == probe_col).then_some(mv),
        |_, _| false,
    )
}

/// Asserts `AboveRowScan::resolve`'s four above-row probe columns match the AVM
/// `row_smvp_all_states[is_sb_boundary][block_width_type]` table entry. `tr_col`
/// is the above-row top-right 4x4 the step-12 `has_top_right` gate consults (`None`
/// when step 12 is disabled/unavailable), so a single helper drives every
/// `[is_sb_boundary][block_width_type]` case (`label` cites the AVM row). The
/// `expected` columns are `[step8, step10, step12, step14]` (`None` = disabled).
fn assert_row_smvp_table(
    label: &str,
    mi_row: usize,
    bw4: usize,
    tr_col: Option<usize>,
    expected: [Option<usize>; 4],
) {
    let geom = generic_scan_geom(mi_row, 8, bw4);
    let scan = AboveRowScan::resolve(&geom, &|r, c| {
        Some((r, c)) == tr_col.map(|t| (mi_row - 1, t))
    });
    assert_eq!(scan.step8, expected[0], "{label} step8 column");
    assert_eq!(
        scan.step10, expected[1],
        "{label} step10 column (is_available)"
    );
    assert_eq!(
        scan.step12, expected[2],
        "{label} step12 column (Max(2,bw4) on border)"
    );
    assert_eq!(scan.step14, expected[3], "{label} step14 column");
}

#[test]
fn within_sb_above_row_columns_match_avm_table_all_widths() {
    assert_row_smvp_table(
        "within-SB BLOCK_WIDTH_4 row_smvp_all_states[0][0]",
        20,
        1,
        Some(9),
        [Some(8), None, Some(9), Some(7)],
    );
    assert_row_smvp_table(
        "within-SB BLOCK_WIDTH_8 row_smvp_all_states[0][1]",
        20,
        2,
        Some(10),
        [Some(9), Some(8), Some(10), Some(7)],
    );
    assert_row_smvp_table(
        "within-SB BLOCK_WIDTH_OTHERS row_smvp_all_states[0][2]",
        20,
        4,
        Some(12),
        [Some(11), Some(8), Some(12), Some(7)],
    );
}

#[test]
fn sb_border_above_row_columns_match_avm_table_all_widths() {
    assert_row_smvp_table(
        "SB-border BLOCK_WIDTH_4 row_smvp_all_states[1][0]",
        32,
        1,
        None,
        [Some(8), None, Some(10), Some(6)],
    );
    assert_row_smvp_table(
        "SB-border BLOCK_WIDTH_8 row_smvp_all_states[1][1]",
        32,
        2,
        None,
        [Some(8), None, Some(10), Some(6)],
    );
    assert_row_smvp_table(
        "SB-border BLOCK_WIDTH_OTHERS row_smvp_all_states[1][2]",
        32,
        4,
        None,
        [Some(10), Some(8), Some(12), Some(6)],
    );
}

#[test]
fn sb_border_narrow_disabled_step10_column_is_ignored() {
    let scan =
        scan_single_intrabc_probe(generic_scan_geom(32, 8, 2), 31, 7, Mv { row: 9, col: -9 });
    assert!(
        scan.candidates.is_empty(),
        "no SB-border state reaches col 7"
    );
}

#[test]
fn sb_border_above_row_columns_align_odd_mi_col() {
    let geom = SpatialScanGeometry {
        mi_row: 32,
        mi_col: 9,
        n4w: 4,
        n4h: 4,
        mi_rows: 64,
        mi_cols: 64,
        sb_size4: 32,
    };
    let scan = AboveRowScan::resolve(&geom, &|_, _| false);
    assert_eq!(scan.step8, Some(10));
    assert_eq!(scan.step10, Some(8));
    assert_eq!(scan.step12, Some(12));
    assert_eq!(scan.step14, Some(6));
}

#[test]
fn sb_border_block_width_4_step12_reads_max2_column() {
    let geom = generic_scan_geom(32, 8, 1);
    let at_10 = scan_single_intrabc_probe(geom, 31, 10, Mv { row: -8, col: -8 });
    assert_eq!(at_10.candidates[..], [adj(Mv { row: -8, col: -8 })]);
    let at_9 = scan_single_intrabc_probe(geom, 31, 9, Mv { row: -8, col: -8 });
    assert!(
        at_9.candidates.is_empty(),
        "no state reaches MiCol+1 for bw4==1"
    );
}
