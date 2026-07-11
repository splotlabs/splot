// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn block(plane: usize, x: usize, y: usize) -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        plane,
        row: y / 4,
        col: x / 4,
        unit_row: 0,
        unit_col: 0,
        tile_mi_row_start: 0,
        tile_mi_row_end: 4,
        tile_mi_col_start: 0,
        tile_mi_col_end: 4,
        x,
        y,
        width: 4,
        height: 4,
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 15,
        frame_luma_end_y: 15,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 15,
    }
}

#[test]
fn merges_contiguous_row_blocks_and_splits_on_filter_visible_fields() {
    let mut stripe_split = block(0, 8, 0);
    stripe_split.luma_stripe_end_y = 7;
    let mut unit_split = block(0, 12, 4);
    unit_split.unit_col = 1;
    let blocks = [
        block(0, 4, 0),
        block(1, 0, 0),
        block(0, 0, 0),
        stripe_split,
        block(0, 12, 0),
        block(0, 0, 4),
        block(0, 4, 4),
        block(0, 8, 4),
        unit_split,
    ];

    let runs = coalesced_lr_source_rows(&blocks, 0);
    let shapes: Vec<_> = runs
        .iter()
        .map(|run| (run.x, run.y, run.width, run.height))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (0, 0, 8, 4),
            (8, 0, 4, 4),
            (12, 0, 4, 4),
            (0, 4, 12, 4),
            (12, 4, 4, 4)
        ],
        "runs must merge contiguous same-row blocks and split when any \
         filter-visible field differs"
    );
    assert!(runs.iter().all(|run| run.plane == 0));
}

#[test]
fn does_not_merge_across_row_gaps() {
    let blocks = [block(0, 0, 0), block(0, 8, 0)];
    let runs = coalesced_lr_source_rows(&blocks, 0);
    assert_eq!(runs.len(), 2);
}

#[test]
fn merges_compatible_adjacent_rows_into_rectangles() {
    let blocks = [
        block(0, 0, 0),
        block(0, 4, 0),
        block(0, 0, 4),
        block(0, 4, 4),
    ];
    let runs = coalesced_lr_source_rows(&blocks, 0);

    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].x, runs[0].y), (0, 0));
    assert_eq!((runs[0].width, runs[0].height), (8, 8));
}

#[test]
fn does_not_merge_rows_across_source_boundaries() {
    let top = block(0, 0, 0);
    let mut bottom = block(0, 0, 4);
    bottom.luma_stripe_start_y = 4;
    let runs = coalesced_lr_source_rows(&[top, bottom], 0);

    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].height, 4);
    assert_eq!(runs[1].height, 4);
}
