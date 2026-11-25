// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 5.20.2.1 `BruModes` of a `use_bru == 1` frame, written by the tile
//! parse and read by the § 7.17 / § 7.19 / § 7.20 superblock gates.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::bitstream::tile_payload::{BRU_ACTIVE, BRU_INACTIVE};
use crate::error::Result;

const BRU_SUPPORT: u8 = 1;
const BRU_UNSET: u8 = u8::MAX;

/// One `bru_mode` per superblock, in raster order.
pub(crate) struct BruModeMap {
    sb_shift: u32,
    sb_cols: usize,
    modes: Box<[AtomicU8]>,
}

#[derive(Clone, Copy)]
struct Region {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
}

impl Region {
    fn covers(self, other: Self) -> bool {
        self.top <= other.top
            && self.bottom >= other.bottom
            && self.left <= other.left
            && self.right >= other.right
    }
}

struct RegionEntry {
    bounds: Region,
    parent: usize,
}

fn region_root(regions: &mut [RegionEntry], mut id: usize) -> usize {
    while regions[id].parent != id {
        let parent = regions[id].parent;
        regions[id].parent = regions[parent].parent;
        id = parent;
    }
    id
}

fn region_allocation_error() -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane: splot_recon::PlaneId::Y,
        context: "BRU region validation",
    }
    .into()
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

    /// Checks the complete § 7.2 BRU map before the frame becomes a reference.
    ///
    /// Each active cell lies inside its original one-cell margin. The final
    /// unclipped rectangle edges therefore cannot contain active cells. The
    /// check can clip rectangles to the frame and reject inactive cells as
    /// soon as a rectangle covers them. Clipping preserves rectangle overlap.
    pub(crate) fn validate_regions(
        &self,
        invalid: impl Fn(usize, usize) -> crate::error::DecodeError,
    ) -> Result<()> {
        let stride = self.sb_cols;
        let len = self.modes.len();
        let rows = len.checked_div(stride).ok_or_else(|| invalid(0, 0))?;
        let invalid_at = |index| {
            invalid(
                (index / stride) << self.sb_shift,
                (index % stride) << self.sb_shift,
            )
        };
        let mut owners = Vec::new();
        let mut regions: Vec<RegionEntry> = Vec::new();
        let mut pending = Vec::new();

        for (index, mode) in self.modes.iter().enumerate() {
            match mode.load(Ordering::Acquire) {
                BRU_ACTIVE => {}
                BRU_INACTIVE | BRU_SUPPORT => continue,
                _ => return Err(invalid_at(index)),
            }
            if owners.is_empty() {
                owners
                    .try_reserve_exact(len)
                    .map_err(|_| region_allocation_error())?;
                owners.resize(len, usize::MAX);
            }
            let row = index / stride;
            let col = index % stride;
            let anchor = Region {
                top: row.saturating_sub(1),
                bottom: (row + 1).min(rows - 1),
                left: col.saturating_sub(1),
                right: (col + 1).min(stride - 1),
            };
            let owner = if owners[index] == usize::MAX {
                (anchor.top..=anchor.bottom)
                    .flat_map(|row| row * stride + anchor.left..=row * stride + anchor.right)
                    .map(|index| owners[index])
                    .find(|&owner| owner != usize::MAX)
                    .unwrap_or(usize::MAX)
            } else {
                owners[index]
            };
            let id = if owner == usize::MAX {
                let id = regions.len();
                regions
                    .try_reserve(1)
                    .map_err(|_| region_allocation_error())?;
                regions.push(RegionEntry {
                    bounds: anchor,
                    parent: id,
                });
                pending
                    .try_reserve(1)
                    .map_err(|_| region_allocation_error())?;
                pending.push(anchor);
                id
            } else {
                let id = region_root(&mut regions, owner);
                extend_region(&mut regions[id].bounds, anchor, &mut pending)?;
                id
            };
            while let Some(strip) = pending.pop() {
                for y in strip.top..=strip.bottom {
                    for x in strip.left..=strip.right {
                        let index = y * stride + x;
                        let owner = &mut owners[index];
                        if *owner == usize::MAX {
                            if self.modes[index].load(Ordering::Acquire) == BRU_INACTIVE {
                                return Err(invalid_at(index));
                            }
                        } else {
                            let other = region_root(&mut regions, *owner);
                            if other != id {
                                let incoming = regions[other].bounds;
                                regions[other].parent = id;
                                extend_region(&mut regions[id].bounds, incoming, &mut pending)?;
                            }
                        }
                        *owner = id;
                    }
                }
            }
        }

        Ok(())
    }

    /// Luma samples per superblock side.
    pub(crate) const fn sb_samples(&self) -> usize {
        4 << self.sb_shift
    }
}

fn extend_region(bounds: &mut Region, other: Region, pending: &mut Vec<Region>) -> Result<()> {
    if bounds.covers(other) {
        return Ok(());
    }
    pending
        .try_reserve(4)
        .map_err(|_| region_allocation_error())?;
    let old = *bounds;
    bounds.top = old.top.min(other.top);
    bounds.bottom = old.bottom.max(other.bottom);
    bounds.left = old.left.min(other.left);
    bounds.right = old.right.max(other.right);
    if bounds.top < old.top {
        pending.push(Region {
            bottom: old.top - 1,
            ..*bounds
        });
    }
    if bounds.bottom > old.bottom {
        pending.push(Region {
            top: old.bottom + 1,
            ..*bounds
        });
    }
    if bounds.left < old.left {
        pending.push(Region {
            left: bounds.left,
            right: old.left - 1,
            ..old
        });
    }
    if bounds.right > old.right {
        pending.push(Region {
            left: old.right + 1,
            right: bounds.right,
            ..old
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::span::ByteOffset;

    fn validate_at_col(map: &BruModeMap) -> Result<()> {
        map.validate_regions(|_, col| {
            crate::pipeline::malformed_tile_payload(
                ByteOffset::new(col as u64),
                "7.2",
                "Invalid BRU superblock.",
            )
        })
    }

    #[test]
    fn validate_regions_checks_the_expansion_across_overlapping_rectangles() {
        let map = BruModeMap::new(7 * 16, 10 * 16, 16).unwrap();
        for row in 0..7 {
            for col in 0..10 {
                map.set(row * 16, col * 16, BRU_SUPPORT);
            }
        }
        for (row, col) in [(1, 1), (1, 3), (1, 5), (3, 1), (5, 1), (5, 7)] {
            map.set(row * 16, col * 16, BRU_ACTIVE);
        }

        map.set(0, 9 * 16, BRU_INACTIVE);
        validate_at_col(&map).unwrap();
        map.set(0, 8 * 16, BRU_INACTIVE);
        let Err(crate::error::DecodeError::MalformedSource { issue }) = validate_at_col(&map)
        else {
            panic!("Expected a malformed BRU region.");
        };
        assert_eq!(issue.spec_section(), Some("7.2"));
        assert_eq!(issue.offset(), Some(ByteOffset::new(8 * 16)));
    }

    #[test]
    fn validate_regions_merges_previously_disjoint_regions() {
        let map = BruModeMap::new(4 * 16, 8 * 16, 16).unwrap();
        for row in 0..4 {
            for col in 0..8 {
                map.set(row * 16, col * 16, BRU_SUPPORT);
            }
        }
        for (row, col) in [(0, 1), (0, 6), (2, 2), (2, 4)] {
            map.set(row * 16, col * 16, BRU_ACTIVE);
        }
        validate_at_col(&map).unwrap();
        map.set(3 * 16, 7 * 16, BRU_INACTIVE);
        assert!(validate_at_col(&map).is_err());
    }

    #[test]
    fn validate_regions_accepts_active_superblocks_at_frame_edges() {
        let map = BruModeMap::new(16, 32, 16).unwrap();
        map.set(0, 0, BRU_ACTIVE);
        map.set(0, 16, BRU_ACTIVE);
        validate_at_col(&map).unwrap();
        map.set(0, 16, BRU_INACTIVE);
        assert!(validate_at_col(&map).is_err());
        map.set(0, 16, BRU_UNSET);
        assert!(validate_at_col(&map).is_err());
    }

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
