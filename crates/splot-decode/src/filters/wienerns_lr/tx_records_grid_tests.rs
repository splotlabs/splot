// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::ReconError;

use super::super::derive_wienerns_lr_tx_skip_grid_retention;
use super::*;

const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
const TX_64X64: usize = 4;
const TX_8X32: usize = 15;
const TX_4X32: usize = 19;

/// The AV2 §7.13.2.1 PER-TRANSFORM far-edge availability (`num4AboveRight`) for the
/// LEFT `TX_16X8` of a `BLOCK_32X8` `V_PRED` coding block at MI(224,30) must be the
/// transform width in 4x4 units (`tx_size_wide_unit == 4`), NOT the partition count
/// `0` the old block-granularity helper produced. The transform's above-right lies
/// inside the coding block's own above span (AVM `has_top_right`,
/// `reconintra.c:113`: `col_off + tx_size_wide_unit < plane_bw_unit` ⇒ `0 + 4 < 8`),
/// so it reads the already-decoded row ABOVE the whole 32x8 block — never the next
/// undecoded partition block to the right. A block-width `count_top_right_avail`
/// scan (`w4 == n4w == 8`) would instead scan past the block and return `0` (the
/// #566 partition-granularity bug). Verifies both the per-transform count (4) and
/// the partition-granularity scan (0) over the same live `BlockDecoded` state.
#[test]
fn selectable_tx_grid_records_middle_and_scan_order_flags() {
    let mut grid = SelectableLumaTxGrid::new(8, 8).unwrap();
    apply_tx_partition(&mut grid, 0, 0, TX_32X32, TX_PARTITION_VERT5).unwrap();

    let records = grid.records_for_region(0, 0, 8, 8).unwrap();
    assert_eq!(records.len(), 5);
    assert_eq!(
        records
            .iter()
            .map(|record| (record.row, record.col, record.rows, record.cols))
            .collect::<Vec<_>>(),
        vec![
            (0, 0, 4, 2),
            (4, 0, 4, 2),
            (0, 2, 8, 4),
            (0, 6, 4, 2),
            (4, 6, 4, 2)
        ]
    );
    assert!(!records[0].middle);
    assert!(records[1..].iter().all(|record| record.middle));
    assert!(records.iter().all(|record| record.scan_order));
}

#[test]
fn selectable_luma_record_fill_context_tracks_full_block_extent() {
    let full_record = SelectableLumaTxRecord {
        row: 0,
        col: 0,
        rows: 16,
        cols: 16,
        tx_size: TX_64X64,
        middle: false,
        scan_order: false,
    };
    assert!(selectable_luma_tx_record_fills_block(full_record, 16, 16));
    assert!(
        !selectable_luma_tx_record_fills_block(full_record, 32, 32),
        "a 64x64 transform record inside a 128x128 luma block must not take the §8.3.2 ctx=0 full-block branch"
    );

    let mut grid = SelectableLumaTxGrid::new(16, 16).unwrap();
    apply_tx_partition(&mut grid, 0, 0, TX_64X64, TX_PARTITION_HORZ5).unwrap();

    let records = grid.records_for_region(0, 0, 16, 16).unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| (record.row, record.col, record.rows, record.cols))
            .collect::<Vec<_>>(),
        vec![
            (0, 0, 4, 8),
            (0, 8, 4, 8),
            (4, 0, 8, 16),
            (12, 0, 4, 8),
            (12, 8, 4, 8),
        ]
    );
    assert!(
        records
            .iter()
            .copied()
            .all(|record| !selectable_luma_tx_record_fills_block(record, 16, 16))
    );
}

#[test]
fn selectable_leaf_shape_admits_luma_only_block_4x32() {
    assert!(selectable_transform_leaf_shape_supported(true, false, 1, 8));
    assert!(selectable_transform_leaf_shape_supported(true, false, 8, 1));
    assert!(selectable_transform_leaf_shape_supported(true, false, 1, 1));
}

#[test]
fn selectable_leaf_shape_preserves_chroma_bearing_narrow_guard() {
    assert!(!selectable_transform_leaf_shape_supported(
        false, true, 1, 8
    ));
    assert!(!selectable_transform_leaf_shape_supported(true, true, 1, 8));
    assert!(!selectable_transform_leaf_shape_supported(
        false, false, 1, 8
    ));
    assert!(!selectable_transform_leaf_shape_supported(
        true, false, 0, 8
    ));
    assert!(!selectable_transform_leaf_shape_supported(
        true, false, 1, 0
    ));
    assert!(selectable_transform_leaf_shape_supported(false, true, 2, 2));
}

#[test]
fn selectable_chroma_offset_leaf_support_is_luma_only() {
    assert!(selectable_chroma_offset_leaf_supported(true, false));
    assert!(!selectable_chroma_offset_leaf_supported(true, true));
    assert!(!selectable_chroma_offset_leaf_supported(false, false));
    assert!(!selectable_chroma_offset_leaf_supported(false, true));
}

#[test]
fn selectable_tx_grid_records_observed_luma_only_block_8x32_region() {
    let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();
    grid.set_tx_size(8, 24, 8, 2, false, false).unwrap();

    let records = grid.records_for_region(8, 24, 8, 2).unwrap();
    assert_eq!(
        records,
        vec![SelectableLumaTxRecord {
            row: 8,
            col: 24,
            rows: 8,
            cols: 2,
            tx_size: TX_8X32,
            middle: false,
            scan_order: false,
        }]
    );
}

#[test]
fn selectable_tx_grid_records_luma_only_block_4x32_region() {
    let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();
    grid.set_tx_size(8, 24, 8, 1, false, false).unwrap();

    let records = grid.records_for_region(8, 24, 8, 1).unwrap();
    assert_eq!(
        records,
        vec![SelectableLumaTxRecord {
            row: 8,
            col: 24,
            rows: 8,
            cols: 1,
            tx_size: TX_4X32,
            middle: false,
            scan_order: false,
        }]
    );
    assert_eq!(
        grid.records_for_region(8, 24, 8, 2).unwrap_err(),
        SelectableTransformRecordError::Incomplete {
            expected: 16,
            actual: 8,
        }
    );
}

#[test]
fn selectable_tx_grid_rejects_empty_transform_dimensions() {
    let mut grid = SelectableLumaTxGrid::new(4, 4).unwrap();

    assert_eq!(
        grid.set_tx_size(0, 0, 4, 0, false, false).unwrap_err(),
        SelectableTransformRecordError::EmptyTransform { h4: 4, w4: 0 }
    );
    assert_eq!(
        grid.set_tx_size(0, 0, 0, 4, false, false).unwrap_err(),
        SelectableTransformRecordError::EmptyTransform { h4: 0, w4: 4 }
    );
}

#[test]
fn selectable_tx_grid_reset_matches_fresh_grid_and_leaks_nothing() {
    let mut fresh = SelectableLumaTxGrid::new(16, 16).unwrap();
    apply_tx_partition(&mut fresh, 8, 4, TX_16X16, TX_PARTITION_HORZ).unwrap();
    let expected = fresh.records_for_region(8, 4, 4, 4).unwrap();

    let mut reused = SelectableLumaTxGrid::new(16, 16).unwrap();
    apply_tx_partition(&mut reused, 0, 0, TX_32X32, TX_PARTITION_VERT5).unwrap();
    reused.records_for_region(0, 0, 8, 8).unwrap();
    reused.reset();

    assert_eq!(
        reused.records_for_region(0, 0, 8, 8).unwrap_err(),
        SelectableTransformRecordError::Incomplete {
            expected: 64,
            actual: 0,
        },
        "reset must clear every previously set cell"
    );
    apply_tx_partition(&mut reused, 8, 4, TX_16X16, TX_PARTITION_HORZ).unwrap();
    assert_eq!(reused.records_for_region(8, 4, 4, 4).unwrap(), expected);
    assert_eq!(reused, fresh);
}

#[test]
fn selectable_tx_grid_reset_clips_edge_overhanging_records() {
    let mut grid = SelectableLumaTxGrid::new(6, 6).unwrap();
    grid.set_tx_size(4, 4, 4, 4, false, false).unwrap();
    grid.reset();

    let fresh = SelectableLumaTxGrid::new(6, 6).unwrap();
    assert_eq!(grid, fresh);
}

#[test]
fn with_selectable_tx_grid_reuses_scratch_without_cross_block_state() {
    let expected = with_selectable_tx_grid(16, 16, |grid| {
        apply_tx_partition(grid, 8, 4, TX_16X16, TX_PARTITION_HORZ).unwrap();
        grid.records_for_region(8, 4, 4, 4).unwrap()
    })
    .unwrap();

    with_selectable_tx_grid(16, 16, |grid| {
        apply_tx_partition(grid, 0, 0, TX_32X32, TX_PARTITION_VERT5).unwrap();
        grid.records_for_region(0, 0, 8, 8).unwrap();
    })
    .unwrap();
    let records = with_selectable_tx_grid(16, 16, |grid| {
        assert!(grid.cells.iter().all(Option::is_none));
        assert!(grid.records.is_empty());
        apply_tx_partition(grid, 8, 4, TX_16X16, TX_PARTITION_HORZ).unwrap();
        grid.records_for_region(8, 4, 4, 4).unwrap()
    })
    .unwrap();
    assert_eq!(records, expected);

    let resized = with_selectable_tx_grid(4, 4, |grid| (grid.rows, grid.cols, grid.cells.len()));
    assert_eq!(resized.unwrap(), (4, 4, 16));
}

#[test]
fn selectable_tx_grid_rejects_incomplete_region() {
    let mut grid = SelectableLumaTxGrid::new(4, 4).unwrap();
    grid.set_tx_size(0, 0, 2, 2, false, false).unwrap();

    assert_eq!(
        grid.records_for_region(0, 0, 4, 4).unwrap_err(),
        SelectableTransformRecordError::Incomplete {
            expected: 16,
            actual: 4,
        }
    );
}

#[test]
fn tx_skip_grid_retention_preserves_skip_flag_for_nonzero_eob_record() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 1,
            skip_flag: true,
            eob: 3,
            intra_ist: None,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 1,
            rows: 1,
            cols: 1,
            skip_flag: false,
            eob: 3,
            intra_ist: None,
        },
    ];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

    assert_eq!(
        tx_skip
            .lookup(super::super::WienerNsLrTxSkipLookup {
                x: 0,
                y: 0,
                row: 0,
                col: 0
            })
            .unwrap(),
        1
    );
    assert_eq!(
        tx_skip
            .lookup(super::super::WienerNsLrTxSkipLookup {
                x: 0,
                y: 0,
                row: 0,
                col: 1
            })
            .unwrap(),
        0
    );
}

/// §5.20.6.1 PC-Wiener `LrTxSkip` FilterClass grid retention drops the off-frame MI
/// cells of a bottom-edge skipped block instead of erroring on the overhang. Models
/// the frontier frontier: a skipped 16x16-MI block at MI(256,0) overhangs the 270-row
/// grid by 2; its in-frame rows 256..270 fill, the off-frame rows 270,271 are dropped
/// (they carry no FilterClass), mirroring AVM `av2_set_entropy_contexts` and the
/// §5.20.3.2 `block_coded` clamp. The full grid stays populated by the in-frame cells.
#[test]
fn tx_skip_grid_retention_clamps_bottom_edge_overhang() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 2,
            cols: 1,
            skip_flag: true,
            eob: 0,
            intra_ist: None,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 2,
            col: 0,
            rows: 4, // nominal extent overhangs the 4-row grid by 2 rows
            cols: 1,
            skip_flag: false,
            eob: 5,
            intra_ist: None,
        },
    ];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(4, 1, &records).unwrap();

    let column: Vec<i32> = (0..4)
        .map(|row| {
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup {
                    x: 0,
                    y: 0,
                    row,
                    col: 0,
                })
                .unwrap()
        })
        .collect();
    assert_eq!(column, vec![1, 1, 0, 0]);
}

/// A right-edge analogue: a record whose nominal width overhangs MiCols fills only its
/// in-frame columns; the past-edge columns are dropped, not errored.
#[test]
fn tx_skip_grid_retention_clamps_right_edge_overhang() {
    let records = [WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 1,
        cols: 4, // nominal width overhangs the 2-col grid by 2 cols
        skip_flag: true,
        eob: 0,
        intra_ist: None,
    }];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

    for col in 0..2 {
        assert_eq!(
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup {
                    x: 0,
                    y: 0,
                    row: 0,
                    col,
                })
                .unwrap(),
            1
        );
    }
}

/// A genuine out-of-frame ORIGIN (`row >= rows` or `col >= cols`) is STILL a hard
/// error, matching the §5.20.3.2 `block_coded` model: AVM never emits such a record.
#[test]
fn tx_skip_grid_retention_rejects_out_of_frame_origin() {
    let records = [WienerNsLrTxSkipTransformRecord {
        row: 4, // origin at/beyond the 4-row grid
        col: 0,
        rows: 1,
        cols: 1,
        skip_flag: true,
        eob: 0,
        intra_ist: None,
    }];

    assert!(matches!(
        derive_wienerns_lr_tx_skip_grid_retention(4, 1, &records).unwrap_err(),
        ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record bounds"
        }
    ));
}

#[test]
fn selectable_tx_records_populate_live_tx_skip_grid() {
    let mut grid = SelectableLumaTxGrid::new(8, 8).unwrap();
    apply_tx_partition(&mut grid, 0, 0, TX_32X32, TX_PARTITION_SPLIT).unwrap();
    let records = grid.records_for_region(0, 0, 8, 8).unwrap();
    let tx_skip_records = records
        .iter()
        .enumerate()
        .map(|(index, record)| WienerNsLrTxSkipTransformRecord {
            row: record.row,
            col: record.col,
            rows: record.rows,
            cols: record.cols,
            skip_flag: false,
            eob: usize::from(index == 0),
            intra_ist: None,
        })
        .collect::<Vec<_>>();

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(8, 8, &tx_skip_records).unwrap();
    assert_eq!(
        (0..8)
            .map(|row| tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup {
                    x: 0,
                    y: 0,
                    row,
                    col: 0
                })
                .unwrap())
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 1, 1, 1]
    );
}

/// The bottom-edge block's in-frame tx records match the geometric placement:
/// each 16x16 chunk tile is `TX_64X64`, only the in-frame rows are filled, and
/// the record origins stay in-frame (rows 256..272 drops the past-edge rows
/// 270,271 without erroring `OutOfBounds`).
#[test]
fn set_tx_size_drops_bottom_edge_cells_past_frame_extent() {
    let mut grid = SelectableLumaTxGrid::new(270, 272).unwrap();
    assert_eq!(
        grid.set_tx_size(256, 0, 16, 16, false, false).unwrap(),
        TX_64X64
    );
    assert_eq!(
        grid.set_tx_size(256, 16, 16, 16, false, false).unwrap(),
        TX_64X64
    );

    assert_eq!(
        grid.records_for_region(256, 0, 16, 32)
            .unwrap()
            .iter()
            .map(|record| (
                record.row,
                record.col,
                record.rows,
                record.cols,
                record.tx_size
            ))
            .collect::<Vec<_>>(),
        vec![(256, 0, 16, 16, TX_64X64), (256, 16, 16, 16, TX_64X64)]
    );
    assert_eq!(
        grid.cell(270, 0).unwrap_err(),
        SelectableTransformRecordError::OutOfBounds {
            row: 270,
            col: 0,
            rows: 270,
            cols: 272,
        }
    );
}

/// An interior block of the same MiSize, fully in-frame, produces records
/// identical (count / tx_size / order) to the edge block's records — the edge
/// block differs only by the dropped out-of-frame cells, not the record set.
#[test]
fn interior_block_records_match_edge_block_minus_dropped_cells() {
    let interior = {
        let mut grid = SelectableLumaTxGrid::new(64, 64).unwrap();
        grid.set_tx_size(0, 0, 16, 16, false, false).unwrap();
        grid.set_tx_size(0, 16, 16, 16, false, false).unwrap();
        grid.records_for_region(0, 0, 16, 32).unwrap()
    };
    let edge = {
        let mut grid = SelectableLumaTxGrid::new(14, 64).unwrap();
        grid.set_tx_size(0, 0, 16, 16, false, false).unwrap();
        grid.set_tx_size(0, 16, 16, 16, false, false).unwrap();
        grid.records_for_region(0, 0, 16, 32).unwrap()
    };
    let shape = |records: &[SelectableLumaTxRecord]| {
        records
            .iter()
            .map(|record| {
                (
                    record.row,
                    record.col,
                    record.rows,
                    record.cols,
                    record.tx_size,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&interior), shape(&edge));
}

/// Right-edge symmetry: a block whose chunk tiles overshoot MiCols drops the
/// past-edge columns the same way the bottom edge drops past-edge rows. frontier
/// frame 0 exercises only the bottom edge, so this is tested, not assumed.
#[test]
fn set_tx_size_drops_right_edge_cells_past_frame_extent() {
    let mut grid = SelectableLumaTxGrid::new(32, 30).unwrap();
    grid.set_tx_size(0, 16, 16, 16, false, false).unwrap();
    grid.set_tx_size(16, 16, 16, 16, false, false).unwrap();
    assert_eq!(
        grid.records_for_region(0, 16, 32, 16)
            .unwrap()
            .iter()
            .map(|record| (record.row, record.col, record.tx_size))
            .collect::<Vec<_>>(),
        vec![(0, 16, TX_64X64), (16, 16, TX_64X64)]
    );
    assert_eq!(
        grid.cell(0, 30).unwrap_err(),
        SelectableTransformRecordError::OutOfBounds {
            row: 0,
            col: 30,
            rows: 32,
            cols: 30,
        }
    );
}

/// `records_for_region` returns Complete (not a false `Incomplete`) for an edge
/// region after the clamp: the completeness count uses the same frame-edge drop
/// as `set_tx_size`, so the dropped out-of-frame cells do not flip the region
/// into `Incomplete`.
#[test]
fn records_for_region_is_complete_for_clamped_edge_region() {
    let mut grid = SelectableLumaTxGrid::new(270, 272).unwrap();
    grid.set_tx_size(256, 0, 16, 16, false, false).unwrap();
    grid.set_tx_size(256, 16, 16, 16, false, false).unwrap();
    let records = grid.records_for_region(256, 0, 16, 32).unwrap();
    assert_eq!(records.len(), 2);

    let mut interior = SelectableLumaTxGrid::new(8, 8).unwrap();
    interior.set_tx_size(0, 0, 4, 4, false, false).unwrap();
    assert_eq!(
        interior
            .records_for_region(0, 0, 4, 4)
            .unwrap()
            .first()
            .map(|record| record.tx_size),
        Some(TX_16X16)
    );
}
