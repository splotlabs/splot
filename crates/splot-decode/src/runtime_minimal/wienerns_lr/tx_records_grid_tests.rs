// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::super::derive_wienerns_lr_tx_skip_grid_retention;
use super::*;

const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
const TX_64X64: usize = 4;
const TX_8X32: usize = 15;
const TX_4X32: usize = 19;

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

// §5.20.6.1 frame-edge cell drop: the spec `LumaTxSizes` fill (set_tx_size,
// 05-syntax-structures.md:12061-12071) has no MiRows/MiCols clamp; out-of-frame
// tx samples are dropped downstream via §5.20.3.2 `block_coded`
// (05-syntax-structures.md:9621). The ac0ej3 trigger is a `BLOCK_128X64` at
// MI(256,0) in a 480x270 MI grid whose chunked path writes two 16x16 MI tiles at
// rows 256..272, overshooting MiRows=270 by 2 rows (rows 270,271) across the
// full 32-MI width.

/// The bottom-edge block's in-frame tx records match the geometric placement:
/// each 16x16 chunk tile is `TX_64X64`, only the in-frame rows are filled, and
/// the record origins stay in-frame (rows 256..272 drops the past-edge rows
/// 270,271 without erroring `OutOfBounds`).
#[test]
fn set_tx_size_drops_bottom_edge_cells_past_frame_extent() {
    // Minimal grid reproducing the ac0ej3 partial-SB bottom row: 272 cols, 270
    // rows, origin MI(256,0), two 16x16 chunk tiles side by side.
    let mut grid = SelectableLumaTxGrid::new(270, 272).unwrap();
    assert_eq!(
        grid.set_tx_size(256, 0, 16, 16, false, false).unwrap(),
        TX_64X64
    );
    assert_eq!(
        grid.set_tx_size(256, 16, 16, 16, false, false).unwrap(),
        TX_64X64
    );

    // The records keep the full geometric extent (16x16) at their in-frame
    // origins, so tx kind / middle / scan_order stay AVM-exact.
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
    // The two out-of-frame rows (270,271) are never filled, and `index()` still
    // errors `OutOfBounds` for the genuinely-out-of-frame coord — the drop happens
    // in `set_tx_size`, not by weakening the grid-index invariant.
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
        // Identical geometry, but the grid ends 2 rows short of the block.
        let mut grid = SelectableLumaTxGrid::new(14, 64).unwrap();
        grid.set_tx_size(0, 0, 16, 16, false, false).unwrap();
        grid.set_tx_size(0, 16, 16, 16, false, false).unwrap();
        grid.records_for_region(0, 0, 16, 32).unwrap()
    };
    // Same record count, tx_size, and order — the in-frame record set is identical.
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
/// past-edge columns the same way the bottom edge drops past-edge rows. ac0ej3
/// frame 0 exercises only the bottom edge, so this is tested, not assumed.
#[test]
fn set_tx_size_drops_right_edge_cells_past_frame_extent() {
    // 32 rows, 30 cols, two stacked 16x16 chunk tiles whose 32-col extent
    // overshoots MiCols=30 by 2 columns (cols 30,31).
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
    // Columns 30,31 past MiCols are never filled; `index()` still errors
    // `OutOfBounds` for the out-of-frame coord (the drop is in `set_tx_size`).
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
    // The block's geometric region is 16 rows x 32 cols at MI(256,0), but rows
    // 270,271 are out of frame; the clamped region is still Complete.
    let records = grid.records_for_region(256, 0, 16, 32).unwrap();
    assert_eq!(records.len(), 2);

    // A single-tile interior `TX_16X16` region stays Complete without the clamp,
    // confirming the clamp does not loosen the in-frame completeness check.
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
