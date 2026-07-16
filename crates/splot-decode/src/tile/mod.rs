// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile and block-local decoder state.

pub(crate) mod block_context;

pub(crate) fn local_grid_index(
    row: usize,
    col: usize,
    origin_row: usize,
    origin_col: usize,
    rows: usize,
    cols: usize,
) -> Option<usize> {
    let row = row.checked_sub(origin_row)?;
    let col = col.checked_sub(origin_col)?;
    if row >= rows || col >= cols {
        return None;
    }
    row.checked_mul(cols)?.checked_add(col)
}
