// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const TX_64X64: usize = 4;

#[test]
fn max_rect_tx_records_cover_skipped_inter_block_without_partition_symbols() {
    let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();

    max_rect::set_max_rect_tx_records(&mut grid, 0, 0, 16, 32, TX_64X64, ByteOffset::new(0))
        .unwrap();

    let records = grid.records_for_region(0, 0, 16, 32).unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record.row,
                record.col,
                record.rows,
                record.cols,
                record.tx_size
            ))
            .collect::<Vec<_>>(),
        vec![(0, 0, 16, 16, TX_64X64), (0, 16, 16, 16, TX_64X64)]
    );
}
