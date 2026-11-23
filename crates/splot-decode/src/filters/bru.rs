// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 5.20.2.1 `BruModes` of a `use_bru == 1` frame, written by the tile
//! parse and read by the § 7.17 / § 7.19 / § 7.20 superblock gates.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::bitstream::tile_payload::BRU_ACTIVE;

const BRU_UNSET: u8 = u8::MAX;

/// One `bru_mode` per superblock, in raster order.
pub(crate) struct BruModeMap {
    sb_shift: u32,
    sb_cols: usize,
    modes: Box<[AtomicU8]>,
}

impl BruModeMap {
    /// The map of a frame of `mi_rows` x `mi_cols` 4x4 units and `sb_size4`
    /// units per superblock side, every superblock initialized to `BRU_UNSET`
    /// until tile parsing publishes its decoded `bru_mode`.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize, sb_size4: usize) -> Option<Self> {
        let sb_shift = sb_size4.checked_ilog2()?;
        let sb_cols = mi_cols.div_ceil(sb_size4);
        let sb_rows = mi_rows.div_ceil(sb_size4);
        let modes = (0..sb_rows.checked_mul(sb_cols)?)
            .map(|_| AtomicU8::new(BRU_UNSET))
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
            cell.store(mode, Ordering::Release);
        }
    }

    /// Returns the published `bru_mode` of the superblock holding
    /// `(mi_row, mi_col)`, or `None` if out of bounds or not yet published.
    pub(crate) fn mode(&self, mi_row: usize, mi_col: usize) -> Option<u8> {
        let mode = self
            .modes
            .get(self.index(mi_row, mi_col))?
            .load(Ordering::Acquire);
        (mode != BRU_UNSET).then_some(mode)
    }

    /// Evaluates the AV2 § 7.20.1 loop-restoration BRU activity gate at global
    /// 4x4 coordinates `(mi_row, mi_col)` within a tile ending at
    /// `tile_mi_row_end`, returning `None` if the governing superblock mode was
    /// not published.
    pub(crate) fn lr_stripe_active(
        &self,
        mi_row: usize,
        mi_col: usize,
        tile_mi_row_end: usize,
    ) -> Option<bool> {
        let stripe_row = tile_mi_row_end
            .saturating_sub(1)
            .min(((mi_row + 2) >> 4) << 4);
        self.mode(stripe_row, mi_col).map(|mode| mode == BRU_ACTIVE)
    }

    /// Whether the superblock holding luma `(mi_row, mi_col)` is `BRU_ACTIVE`
    /// or not yet explicitly marked non-active.
    pub(crate) fn active(&self, mi_row: usize, mi_col: usize) -> bool {
        self.modes
            .get(self.index(mi_row, mi_col))
            .is_none_or(|cell| matches!(cell.load(Ordering::Acquire), BRU_ACTIVE | BRU_UNSET))
    }

    /// Luma samples per superblock side.
    pub(crate) const fn sb_samples(&self) -> usize {
        4 << self.sb_shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superblocks_default_active_and_record_their_parsed_mode() {
        let map = BruModeMap::new(40, 70, 16).unwrap();
        assert!(map.active(39, 69));
        assert_eq!(map.mode(39, 69), None);
        assert_eq!(map.lr_stripe_active(39, 69, 40), None);
        map.set(16, 48, 1);
        assert_eq!(map.mode(16, 48), Some(1));
        assert!(!map.active(31, 63));
        assert!(map.active(32, 63));
        assert!(map.active(31, 64));
        assert_eq!(map.sb_samples(), 64);
        assert!(BruModeMap::new(8, 8, 0).is_none());
    }

    #[test]
    fn lr_stripe_active_uses_shifted_stripe_row_and_fails_closed_when_unset() {
        let map = BruModeMap::new(48, 32, 16).unwrap();
        assert_eq!(map.lr_stripe_active(13, 0, 32), None);

        for (first_sb_active, non_active_mode) in [
            (true, 1u8),
            (false, crate::bitstream::tile_payload::BRU_INACTIVE),
        ] {
            let row0_mode = if first_sb_active {
                BRU_ACTIVE
            } else {
                non_active_mode
            };
            let row1_mode = if first_sb_active {
                non_active_mode
            } else {
                BRU_ACTIVE
            };
            map.set(0, 0, row0_mode);
            map.set(16, 0, row1_mode);

            assert_eq!(map.lr_stripe_active(13, 0, 32), Some(first_sb_active));
            assert_eq!(map.lr_stripe_active(14, 0, 32), Some(!first_sb_active));
            assert_eq!(map.lr_stripe_active(15, 0, 32), Some(!first_sb_active));
            assert_eq!(map.lr_stripe_active(16, 0, 32), Some(!first_sb_active));
            assert_eq!(map.lr_stripe_active(30, 0, 32), Some(!first_sb_active));
        }

        let tile_map = BruModeMap::new(48, 32, 16).unwrap();
        tile_map.set(16, 16, BRU_ACTIVE);
        tile_map.set(32, 16, 1);
        assert_eq!(tile_map.lr_stripe_active(29, 16, 48), Some(true));
        assert_eq!(tile_map.lr_stripe_active(30, 16, 48), Some(false));
        assert_eq!(tile_map.lr_stripe_active(46, 16, 48), Some(false));
        assert_eq!(tile_map.lr_stripe_active(29, 0, 48), None);
    }
}
