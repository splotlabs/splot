// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const TX_8X32: usize = 15;
const TX_8X64: usize = 21;

#[test]
fn selectable_luma_leaf_actual_extent_is_only_observed_luma_narrow() {
    for (n4w, n4h) in [(1, 8), (2, 8), (1, 16), (2, 16)] {
        assert!(selectable_luma_leaf_uses_actual_extent(
            true, false, n4w, n4h
        ));
    }

    for (is_luma, has_chroma, n4w, n4h) in [
        (true, false, 8, 1),
        (true, false, 8, 2),
        (true, false, 1, 1),
        (true, false, 2, 2),
        (true, false, 2, 4),
        (true, false, 4, 16),
        (true, true, 1, 8),
        (true, true, 2, 8),
        (true, true, 2, 16),
        (false, false, 1, 8),
        (false, false, 2, 8),
        (false, false, 2, 16),
        (false, true, 1, 8),
        (true, false, 0, 8),
    ] {
        assert!(!selectable_luma_leaf_uses_actual_extent(
            is_luma, has_chroma, n4w, n4h
        ));
    }
}

#[test]
fn actual_extent_fallback_records_consumed_empty_partition_as_observed_leaf() {
    let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();

    let tx_size = apply_tx_partition_or_actual_extent(
        &mut grid,
        8,
        24,
        TX_8X32,
        TX_PARTITION_VERT5,
        Some((8, 2)),
    )
    .unwrap();

    assert_eq!(tx_size, TX_8X32);
    assert_eq!(
        grid.records_for_region(8, 24, 8, 2).unwrap(),
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
fn actual_extent_fallback_records_8x64_luma_leaf() {
    let mut grid = SelectableLumaTxGrid::new(64, 256).unwrap();

    let tx_size = apply_tx_partition_or_actual_extent(
        &mut grid,
        32,
        190,
        TX_8X64,
        TX_PARTITION_VERT5,
        Some((16, 2)),
    )
    .unwrap();

    assert_eq!(tx_size, TX_8X64);
    assert_eq!(
        grid.records_for_region(32, 190, 16, 2).unwrap(),
        vec![SelectableLumaTxRecord {
            row: 32,
            col: 190,
            rows: 16,
            cols: 2,
            tx_size: TX_8X64,
            middle: false,
            scan_order: false,
        }]
    );
}

#[test]
fn actual_extent_fallback_preserves_empty_partition_error_when_disallowed() {
    let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();

    assert_eq!(
        apply_tx_partition_or_actual_extent(&mut grid, 8, 24, TX_8X32, TX_PARTITION_VERT5, None,)
            .unwrap_err(),
        SelectableTransformRecordError::EmptyTransform { h4: 4, w4: 0 }
    );
}
