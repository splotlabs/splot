// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use crate::error::Result;

use super::{
    MI_SIZE, SelectableLumaTxGrid, selectable_transform_record_error, tx_dimension,
    wienerns_lr_selectable_transform_record_error_reason,
};

pub(crate) fn set_max_rect_tx_records(
    grid: &mut SelectableLumaTxGrid,
    row: usize,
    col: usize,
    n4h: usize,
    n4w: usize,
    max_tx_size: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let max_rect_error =
        |reason| wienerns_lr_selectable_transform_record_error_reason(tile_offset, reason);
    let tx_w4 = tx_dimension("Tx_Width", &TX_WIDTH, max_tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_dimension("Tx_Height", &TX_HEIGHT, max_tx_size, tile_offset)? / MI_SIZE;
    if tx_w4 == 0 || tx_h4 == 0 {
        return Err(max_rect_error(
            "unsupported_wienerns_lr_selectable_transform_records_max_tx_zero",
        ));
    }
    let row_end = row.checked_add(n4h).ok_or_else(|| {
        max_rect_error("unsupported_wienerns_lr_selectable_transform_records_max_tx_row_overflow")
    })?;
    let col_end = col.checked_add(n4w).ok_or_else(|| {
        max_rect_error("unsupported_wienerns_lr_selectable_transform_records_max_tx_col_overflow")
    })?;
    let mut tx_row = row;
    while tx_row < row_end {
        let h4 = tx_h4.min(row_end - tx_row);
        let mut tx_col = col;
        while tx_col < col_end {
            let w4 = tx_w4.min(col_end - tx_col);
            grid.set_tx_size(tx_row, tx_col, h4, w4, false, false)
                .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
            tx_col = tx_col.checked_add(w4).ok_or_else(|| {
                max_rect_error(
                    "unsupported_wienerns_lr_selectable_transform_records_max_tx_col_step_overflow",
                )
            })?;
        }
        tx_row = tx_row.checked_add(h4).ok_or_else(|| {
            max_rect_error(
                "unsupported_wienerns_lr_selectable_transform_records_max_tx_row_step_overflow",
            )
        })?;
    }
    Ok(())
}
