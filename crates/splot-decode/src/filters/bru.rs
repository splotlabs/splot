// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 5.20.2.1 `BruModes` of a `use_bru == 1` frame, written by the tile
//! parse and read by the § 7.17 / § 7.19 / § 7.20 superblock gates.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::bitstream::tile_payload::BRU_ACTIVE;

/// One `bru_mode` per superblock, in raster order.
pub(crate) struct BruModeMap {
    sb_shift: u32,
    sb_cols: usize,
    modes: Box<[AtomicU8]>,
}

impl BruModeMap {
    /// The map of a frame of `mi_rows` x `mi_cols` 4x4 units and `sb_size4`
    /// units per superblock side, every superblock `BRU_ACTIVE` until parsed.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize, sb_size4: usize) -> Option<Self> {
        let sb_shift = sb_size4.checked_ilog2()?;
        let sb_cols = mi_cols.div_ceil(sb_size4);
        let sb_rows = mi_rows.div_ceil(sb_size4);
        let modes = (0..sb_rows.checked_mul(sb_cols)?)
            .map(|_| AtomicU8::new(BRU_ACTIVE))
            .collect();
        Some(Self {
            sb_shift,
            sb_cols,
            modes,
        })
    }

    fn index(&self, mi_row: usize, mi_col: usize) -> usize {
        (mi_row >> self.sb_shift) * self.sb_cols + (mi_col >> self.sb_shift)
    }

    /// Records the `bru_mode` of the superblock holding `(mi_row, mi_col)`.
    pub(crate) fn set(&self, mi_row: usize, mi_col: usize, mode: u8) {
        if let Some(cell) = self.modes.get(self.index(mi_row, mi_col)) {
            cell.store(mode, Ordering::Relaxed);
        }
    }

    /// Whether the superblock holding luma `(mi_row, mi_col)` is `BRU_ACTIVE`.
    pub(crate) fn active(&self, mi_row: usize, mi_col: usize) -> bool {
        self.modes
            .get(self.index(mi_row, mi_col))
            .is_none_or(|cell| cell.load(Ordering::Relaxed) == BRU_ACTIVE)
    }

    /// Luma samples per superblock side.
    pub(crate) const fn sb_samples(&self) -> usize {
        4 << self.sb_shift
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn superblocks_default_active_and_record_their_parsed_mode() {
        let map = BruModeMap::new(40, 70, 16).unwrap();
        assert!(map.active(39, 69));
        map.set(16, 48, 1);
        assert!(!map.active(31, 63));
        assert!(map.active(32, 63));
        assert!(map.active(31, 64));
        assert_eq!(map.sb_samples(), 64);
        assert!(BruModeMap::new(8, 8, 0).is_none());
    }
}
