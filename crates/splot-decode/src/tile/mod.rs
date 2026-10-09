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

/// Window of one superblock row and the mode-info rows above it.
///
/// Neighbour state read only by the superblock row being decoded and the
/// [`SB_ROW_WINDOW_ABOVE_ROWS`] rows above it stores [`SbRowWindow::plane_rows`]
/// rows, and moves those rows up as the window moves down. That is enough
/// because the § 7.12.2.6 scan points, § 7.12.2.4 warp corners and § 5.20.5.3
/// contexts read one MI row above the block, and the § 7.12.2.15 TIP candidate
/// aligns that row down to its TIP block, at most 4 MI rows above. A row of a
/// later superblock row reads as unpublished. A row older than the window is a
/// decoder defect: the access fails and [`SbRowWindow::violated`] stays set.
#[derive(Debug, Default)]
pub(crate) struct SbRowWindow {
    rows: usize,
    sb_h4_log2: u32,
    windowed: bool,
    current: Option<usize>,
    violated: AtomicBool,
}

/// Mode-info rows above the current superblock row that [`SbRowWindow`] keeps.
pub(crate) const SB_ROW_WINDOW_ABOVE_ROWS: usize = 4;

/// An access to a row the window has already reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutsideSbRowWindow;

/// The plane rows one [`SbRowWindow::enter`] keeps and clears.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SbRowSlide {
    keep: Range<usize>,
    clear: Range<usize>,
}

impl SbRowSlide {
    /// Moves the kept rows of a `cols`-wide plane to its top, then clears the
    /// current superblock row's rows. An unsized plane is left alone.
    pub(crate) fn apply<T: Copy>(&self, plane: &mut [T], cols: usize, default: T) {
        let keep = self.keep.start * cols..self.keep.end * cols;
        if plane.get(keep.clone()).is_some() {
            plane.copy_within(keep, 0);
        }
        if let Some(clear) = plane.get_mut(self.clear.start * cols..self.clear.end * cols) {
            clear.fill(default);
        }
    }
}

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
            self.windowed,
            self.current,
            self.violated(),
        ) == (
            other.rows,
            other.sb_h4_log2,
            other.windowed,
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
        Self {
            rows,
            sb_h4_log2: sb_h4.trailing_zeros(),
            windowed: sb_h4 < Self::WHOLE_TILE_SB_H4 && rows > sb_h4.saturating_mul(2),
            current: None,
            violated: AtomicBool::new(false),
        }
    }

    pub(crate) const fn plane_rows(&self) -> usize {
        if self.windowed {
            (1 << self.sb_h4_log2) + SB_ROW_WINDOW_ABOVE_ROWS
        } else {
            self.rows
        }
    }

    /// Moves the window down to the superblock row holding tile row `row` and
    /// returns the plane rows to keep and to clear.
    pub(crate) fn enter(&mut self, row: usize) -> Option<SbRowSlide> {
        let sb_row = row >> self.sb_h4_log2;
        let previous = self.current;
        if previous.is_some_and(|current| current >= sb_row) {
            return None;
        }
        self.current = Some(sb_row);
        let sb_h4 = 1 << self.sb_h4_log2;
        match previous {
            Some(_) if !self.windowed => None,
            Some(current) if current + 1 == sb_row => Some(SbRowSlide {
                keep: sb_h4..sb_h4 + SB_ROW_WINDOW_ABOVE_ROWS,
                clear: SB_ROW_WINDOW_ABOVE_ROWS..self.plane_rows(),
            }),
            Some(_) => Some(SbRowSlide {
                keep: 0..0,
                clear: 0..self.plane_rows(),
            }),
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
        if !self.windowed {
            return Ok(Some(row));
        }
        (row + SB_ROW_WINDOW_ABOVE_ROWS)
            .checked_sub(current << self.sb_h4_log2)
            .map(Some)
            .ok_or(OutsideSbRowWindow)
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
        let mut window = SbRowWindow::new(24, 8);
        assert_eq!(window.plane_rows(), 12);
        assert_eq!(window.checked_plane_row(0), Ok(None));
        assert_eq!(window.enter(0), None);
        assert_eq!(window.checked_plane_row(7), Ok(Some(11)));
        let mut plane: Vec<usize> = (0..12).collect();
        if let Some(slide) = window.enter(8) {
            slide.apply(&mut plane, 1, 0);
        }
        assert_eq!(plane, [8, 9, 10, 11, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(window.checked_plane_row(4), Ok(Some(0)));
        assert_eq!(window.checked_plane_row(9), Ok(Some(5)));
        assert_eq!(window.checked_plane_row(16), Ok(None));
        assert_eq!(window.checked_plane_row(3), Err(OutsideSbRowWindow));
        assert!(!window.violated());
        assert_eq!(window.plane_row(3), None);
        assert!(window.violated());
        if let Some(jump) = window.enter(23) {
            jump.apply(&mut plane, 1, 0);
        }
        assert_eq!(plane, [0; 12]);
    }
}
