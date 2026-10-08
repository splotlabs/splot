// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile and block-local decoder state.

use core::ops::Range;
use core::sync::atomic::{AtomicBool, Ordering};

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
/// superblock row is two rows old. The window is enough because no neighbour
/// read reaches above the superblock row above: the § 7.12.2.6 scan points,
/// § 7.12.2.4 warp corners and § 5.20.5.3 contexts read one MI row above the
/// block, and the § 7.12.2.15 TIP candidate aligns that row down to its TIP
/// block, at most 4 MI rows above. A row of a later superblock row reads as
/// unpublished. A row older than the window is a decoder defect: the access
/// fails and [`SbRowWindow::violated`] stays set until the next reset.
#[derive(Debug, Default)]
pub(crate) struct SbRowWindow {
    rows: usize,
    sb_h4_log2: u32,
    ring_mask: usize,
    current: Option<usize>,
    violated: AtomicBool,
}

/// An access to a row the window has already reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutsideSbRowWindow;

impl Clone for SbRowWindow {
    fn clone(&self) -> Self {
        Self {
            violated: AtomicBool::new(self.violated()),
            ..*self
        }
    }
}

impl PartialEq for SbRowWindow {
    fn eq(&self, other: &Self) -> bool {
        (
            self.rows,
            self.sb_h4_log2,
            self.ring_mask,
            self.current,
            self.violated(),
        ) == (
            other.rows,
            other.sb_h4_log2,
            other.ring_mask,
            other.current,
            other.violated(),
        )
    }
}

impl Eq for SbRowWindow {}

impl SbRowWindow {
    /// A superblock height that spans any tile, so no row is ever reused.
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
            violated: AtomicBool::new(false),
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

    /// Plane row of tile row `row`: `None` for a row past the tile or not yet
    /// published, an error for a row the window has reused.
    pub(crate) fn checked_plane_row(
        &self,
        row: usize,
    ) -> Result<Option<usize>, OutsideSbRowWindow> {
        let sb_row = row >> self.sb_h4_log2;
        let Some(current) = self.current else {
            return Ok(None);
        };
        if row >= self.rows || sb_row > current {
            return Ok(None);
        }
        if sb_row + 1 < current {
            return Err(OutsideSbRowWindow);
        }
        Ok(Some(row & self.ring_mask))
    }

    /// [`SbRowWindow::checked_plane_row`] for a caller with no error path:
    /// a reused row reads as `None` and sets [`SbRowWindow::violated`].
    pub(crate) fn plane_row(&self, row: usize) -> Option<usize> {
        self.checked_plane_row(row)
            .unwrap_or_else(|OutsideSbRowWindow| {
                self.violated.store(true, Ordering::Relaxed);
                None
            })
    }

    /// Whether any access since the last reset touched a reused row.
    pub(crate) fn violated(&self) -> bool {
        self.violated.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::{OutsideSbRowWindow, SbRowWindow};

    #[test]
    fn a_row_the_window_reused_is_an_error_not_an_absent_cell() {
        let mut window = SbRowWindow::new(12, 4);
        assert_eq!(window.checked_plane_row(0), Ok(None));
        let _ = window.enter(0);
        let _ = window.enter(4);
        assert_eq!(window.enter(8), Some(0..4));
        assert_eq!(window.checked_plane_row(9), Ok(Some(1)));
        assert_eq!(window.checked_plane_row(5), Ok(Some(5)));
        assert_eq!(window.checked_plane_row(11), Ok(Some(3)));
        assert_eq!(window.checked_plane_row(3), Err(OutsideSbRowWindow));
        assert!(!window.violated());
        assert_eq!(window.plane_row(3), None);
        assert!(window.violated());
    }
}
