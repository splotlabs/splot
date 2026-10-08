// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile and block-local decoder state.

use core::ops::Range;

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

/// Window of two superblock rows over a tile's mode-info rows.
///
/// Neighbour state read only by the superblock row being decoded and the row
/// above it stores [`SbRowWindow::plane_rows`] rows, and reuses a row once its
/// superblock row is two rows old. Rows outside the window read as absent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SbRowWindow {
    rows: usize,
    sb_h4_log2: u32,
    ring_mask: usize,
    current: Option<usize>,
}

impl SbRowWindow {
    /// A superblock height that spans any tile, so no row is ever reused.
    #[cfg(test)]
    pub(crate) const WHOLE_TILE_SB_H4: usize = 1 << (usize::BITS - 2);

    pub(crate) fn new(rows: usize, sb_h4: usize) -> Self {
        let sb_h4 = sb_h4.max(1).next_power_of_two();
        let ring_rows = sb_h4.saturating_mul(2);
        Self {
            rows,
            sb_h4_log2: sb_h4.trailing_zeros(),
            ring_mask: if rows > ring_rows {
                ring_rows - 1
            } else {
                usize::MAX
            },
            current: None,
        }
    }

    pub(crate) const fn plane_rows(&self) -> usize {
        if self.ring_mask == usize::MAX {
            self.rows
        } else {
            self.ring_mask + 1
        }
    }

    /// Moves the window down to the superblock row holding tile row `row` and
    /// returns the plane rows that now belong to it and must be cleared.
    pub(crate) fn enter(&mut self, row: usize) -> Option<Range<usize>> {
        let sb_row = row >> self.sb_h4_log2;
        let previous = self.current;
        if previous.is_some_and(|current| current >= sb_row) {
            return None;
        }
        self.current = Some(sb_row);
        match previous {
            Some(_) if self.ring_mask == usize::MAX => None,
            Some(current) if current + 1 == sb_row => {
                let start = (sb_row << self.sb_h4_log2) & self.ring_mask;
                Some(start..start + (1 << self.sb_h4_log2))
            }
            Some(_) => Some(0..self.plane_rows()),
            None => None,
        }
    }

    /// Plane row of tile row `row`, `None` outside the window.
    pub(crate) fn plane_row(&self, row: usize) -> Option<usize> {
        let sb_row = row >> self.sb_h4_log2;
        let current = self.current?;
        (row < self.rows && sb_row <= current && sb_row + 1 >= current)
            .then_some(row & self.ring_mask)
    }
}
