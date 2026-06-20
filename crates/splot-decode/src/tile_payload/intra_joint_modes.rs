// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.5.3 per-MI `IntraJointModes` neighbour-mode state.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`.
//!
//! The general intra `y_mode_index` (and `y_mode_offset`) § 8.3.2 CDF context is
//! `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1)
//! >= NON_DIRECTIONAL_MODES_COUNT)`
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`), where
//! `get_joint_mode(dir)` reads the left (`dir == 0`) / above (`dir == 1`)
//! neighbour's stored `IntraJointModes[mvRow][mvCol]`
//! (`= IntraJointMode = modeDelta`, the § 5.20.5.3 reorder index) or `DC_PRED`
//! (`0`) when that neighbour is out of frame
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`, `get_joint_mode`).
//!
//! This module tracks that per-MI `IntraJointModes` grid for the general intra
//! partition walk so the `y_mode_index` context can be derived from real
//! reconstructed neighbours instead of the hardcoded tile-origin `ctx == 0`.

use std::collections::TryReserveError;

/// AV2 § 3 `NON_DIRECTIONAL_MODES_COUNT` (`03-symbols.md`): the number of
/// non-directional intra modes; a `modeDelta` at or above this is directional.
const NON_DIRECTIONAL_MODES_COUNT: u8 = 5;

/// AV2 `DC_PRED` (intra mode `0`), and the `IntraJointMode` (`modeDelta`) value
/// `get_joint_mode` returns for an out-of-frame neighbour
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`, `get_joint_mode`).
/// `DC_PRED < NON_DIRECTIONAL_MODES_COUNT`, so an out-of-frame neighbour is
/// non-directional and contributes `0` to the § 8.3.2 context.
const DC_PRED_JOINT_MODE: u8 = 0;

/// Mutable tile-local AV2 § 5.20.5.3 `IntraJointModes[r][c]` grid.
///
/// Each cell stores the block's `IntraJointMode` (`= modeDelta`, the § 5.20.5.3
/// reorder index, not the canonical § 9.2 intra mode value) for every MI unit it
/// covers (`IntraJointModes[r + y][c + x] = IntraJointMode` for `y in 0..bh4`,
/// `x in 0..bw4`, `docs/spec/av2/1.0.0/05-syntax-structures.md` line 9998). The
/// grid is initialized to `DC_PRED` (`0`); reads outside the grid resolve to
/// `DC_PRED` via [`Self::get_joint_mode`]'s `is_inside` check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraJointModeState {
    mi_rows: usize,
    mi_cols: usize,
    /// Row-major `IntraJointModes` grid (`mi_rows * mi_cols` cells).
    joint_modes: Vec<u8>,
}

impl TileIntraJointModeState {
    /// Creates an `IntraJointModes` grid for the given tile MI dimensions,
    /// initialized to `DC_PRED` (matching the out-of-frame `get_joint_mode`
    /// default for cells not yet written).
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<Self, TileIntraJointModeStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileIntraJointModeStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        let cells = mi_rows.checked_mul(mi_cols).ok_or(
            TileIntraJointModeStateError::ArithmeticOverflow {
                operation: "mi_rows * mi_cols",
                left: mi_rows,
                right: mi_cols,
            },
        )?;
        let mut joint_modes = Vec::new();
        joint_modes
            .try_reserve_exact(cells)
            .map_err(|source| TileIntraJointModeStateError::Allocation { source })?;
        joint_modes.resize(cells, DC_PRED_JOINT_MODE);
        Ok(Self {
            mi_rows,
            mi_cols,
            joint_modes,
        })
    }

    /// AV2 § 5.20.5.3 `get_joint_mode(dir)` for the block at MI position
    /// (`r`, `c`) with `n4w`/`n4h` MI width/height
    /// (`Num_4x4_Blocks_Wide/High[MiSize]`).
    ///
    /// `dir == 0` reads the left neighbour (`mvCol = MiCol - 1`,
    /// `mvRow = MiRow + Num_4x4_Blocks_High - 1`); `dir == 1` reads the above
    /// neighbour (`mvRow = MiRow - 1`, `mvCol = MiCol + Num_4x4_Blocks_Wide - 1`).
    /// Returns `IntraJointModes[mvRow][mvCol]` when that cell is inside the grid,
    /// or `DC_PRED` (`0`) otherwise
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
    fn get_joint_mode(&self, dir: usize, r: usize, c: usize, n4w: usize, n4h: usize) -> u8 {
        let (mv_row, mv_col) = if dir == 1 {
            // Above neighbour: mvRow = MiRow - 1, mvCol = MiCol + n4w - 1.
            let Some(mv_row) = r.checked_sub(1) else {
                return DC_PRED_JOINT_MODE;
            };
            (mv_row, c.saturating_add(n4w.saturating_sub(1)))
        } else {
            // Left neighbour: mvCol = MiCol - 1, mvRow = MiRow + n4h - 1.
            let Some(mv_col) = c.checked_sub(1) else {
                return DC_PRED_JOINT_MODE;
            };
            (r.saturating_add(n4h.saturating_sub(1)), mv_col)
        };
        match self.cell(mv_row, mv_col) {
            Some(value) => value,
            None => DC_PRED_JOINT_MODE,
        }
    }

    /// AV2 § 8.3.2 `y_mode_index` (and `y_mode_offset`) CDF context for the block
    /// at MI position (`r`, `c`) with `n4w`/`n4h` MI width/height.
    ///
    /// `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) +
    /// (get_joint_mode(1) >= NON_DIRECTIONAL_MODES_COUNT)`
    /// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`); the result is in
    /// `0..=2`.
    pub(crate) fn y_mode_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let left = self.get_joint_mode(0, r, c, n4w, n4h);
        let above = self.get_joint_mode(1, r, c, n4w, n4h);
        (left >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (above >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }

    /// Writes the block's `IntraJointMode` (`= modeDelta`) into every MI cell it
    /// covers (`IntraJointModes[r + y][c + x] = IntraJointMode`, AV2
    /// § 5.20.5.3 / `05-syntax-structures.md` line 9998), bounded to the grid.
    /// Cells outside the grid are silently skipped (the grid is sized to the tile
    /// MI extent; a block straddling the frame edge writes only its in-frame MIs).
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        joint_mode: u8,
    ) {
        for y in 0..n4h {
            let Some(row) = r.checked_add(y) else { break };
            if row >= self.mi_rows {
                break;
            }
            for x in 0..n4w {
                let Some(col) = c.checked_add(x) else { break };
                if col >= self.mi_cols {
                    break;
                }
                if let Some(index) = self.cell_index(row, col) {
                    self.joint_modes[index] = joint_mode;
                }
            }
        }
    }

    /// Reads `IntraJointModes[row][col]` when inside the grid.
    fn cell(&self, row: usize, col: usize) -> Option<u8> {
        self.cell_index(row, col)
            .map(|index| self.joint_modes[index])
    }

    /// Row-major grid index for (`row`, `col`), or `None` when out of bounds.
    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return None;
        }
        row.checked_mul(self.mi_cols)?.checked_add(col)
    }
}

/// Error raised while building or sizing the `IntraJointModes` grid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileIntraJointModeStateError {
    /// The tile MI dimensions were empty.
    #[error("intra joint-mode state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// A dimension product overflowed `usize`.
    #[error("intra joint-mode state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        /// The overflowing operation.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// The grid allocation failed.
    #[error("intra joint-mode state allocation failed: {source}")]
    Allocation {
        /// The underlying reservation error.
        source: TryReserveError,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A representative directional `modeDelta` (>= NON_DIRECTIONAL_MODES_COUNT):
    /// the merged D135 `IntraJointMode == 36` decoded at the top-left no-neighbour
    /// block (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
    const D135_JOINT_MODE: u8 = 36;
    /// A representative non-directional `modeDelta` (< NON_DIRECTIONAL_MODES_COUNT):
    /// SMOOTH_V `IntraJointMode == 2`.
    const SMOOTH_V_JOINT_MODE: u8 = 2;

    #[test]
    fn out_of_frame_neighbours_give_context_zero() {
        // The single-block tile origin: left (MiCol - 1) and above (MiRow - 1)
        // are both out of frame, so get_joint_mode returns DC_PRED (0), which is
        // non-directional -> ctx 0 (matches the hardcoded tile-origin literal).
        let state = TileIntraJointModeState::new(16, 16).unwrap();
        assert_eq!(state.y_mode_index_ctx(0, 0, 16, 16), 0);
    }

    #[test]
    fn non_directional_neighbour_keeps_context_zero() {
        // A left neighbour whose IntraJointMode is non-directional (SMOOTH_V,
        // modeDelta 2 < 5) contributes 0 to the context: the verified mbvg
        // neighbour-SMOOTH case.
        let mut state = TileIntraJointModeState::new(16, 32).unwrap();
        // Left 64x64 superblock (16x16 MIs) at (0, 0) stores SMOOTH_V.
        state.record_block(0, 0, 16, 16, SMOOTH_V_JOINT_MODE);
        // The right superblock at (0, 16) reads the left neighbour -> still ctx 0.
        assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 0);
    }

    #[test]
    fn directional_left_neighbour_raises_context_to_one() {
        // A left neighbour storing a directional IntraJointMode (D135, modeDelta
        // 36 >= 5) raises get_joint_mode(0) >= 5 -> ctx 1. This is the latent
        // #383 case the codex P2 finding flagged: a DC block to the right of a
        // D135 top-left block needs ctx 1, not the hardcoded 0.
        let mut state = TileIntraJointModeState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 1);
    }

    #[test]
    fn directional_above_neighbour_raises_context_to_one() {
        // A directional above neighbour (D135) likewise raises ctx to 1 via
        // get_joint_mode(1).
        let mut state = TileIntraJointModeState::new(32, 16).unwrap();
        state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(16, 0, 16, 16), 1);
    }

    #[test]
    fn directional_both_neighbours_raise_context_to_two() {
        // Directional left AND above neighbours sum to ctx 2 (the § 8.3.2 sum of
        // two indicators).
        let mut state = TileIntraJointModeState::new(32, 32).unwrap();
        // Above neighbour of the block at (16, 16): get_joint_mode(1) reads
        // IntraJointModes[15][16 + 16 - 1] = [15][31].
        state.record_block(0, 16, 16, 16, D135_JOINT_MODE);
        // Left neighbour: get_joint_mode(0) reads IntraJointModes[16 + 16 - 1][15]
        // = [31][15].
        state.record_block(16, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(16, 16, 16, 16), 2);
    }

    #[test]
    fn get_joint_mode_uses_the_spec_neighbour_positions() {
        // get_joint_mode(0) (left): mvCol = c - 1, mvRow = r + n4h - 1.
        // get_joint_mode(1) (above): mvRow = r - 1, mvCol = c + n4w - 1.
        let mut state = TileIntraJointModeState::new(8, 8).unwrap();
        // Mark the exact left-neighbour cell for a 2x2 block at (2, 2):
        // mvRow = 2 + 2 - 1 = 3, mvCol = 2 - 1 = 1.
        state.record_block(3, 1, 1, 1, D135_JOINT_MODE);
        assert_eq!(state.get_joint_mode(0, 2, 2, 2, 2), D135_JOINT_MODE);
        // Mark the exact above-neighbour cell: mvRow = 2 - 1 = 1, mvCol = 2 + 2 - 1 = 3.
        state.record_block(1, 3, 1, 1, D135_JOINT_MODE);
        assert_eq!(state.get_joint_mode(1, 2, 2, 2, 2), D135_JOINT_MODE);
    }

    #[test]
    fn last_non_directional_mode_does_not_raise_the_context() {
        // modeDelta == NON_DIRECTIONAL_MODES_COUNT - 1 is still non-directional.
        let mut state = TileIntraJointModeState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, NON_DIRECTIONAL_MODES_COUNT - 1);
        assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 0);
    }

    #[test]
    fn empty_dimensions_are_rejected() {
        assert!(matches!(
            TileIntraJointModeState::new(0, 4),
            Err(TileIntraJointModeStateError::EmptyDimensions { .. })
        ));
        assert!(matches!(
            TileIntraJointModeState::new(4, 0),
            Err(TileIntraJointModeStateError::EmptyDimensions { .. })
        ));
    }

    #[test]
    fn record_block_clips_to_the_grid() {
        // A block straddling the frame edge writes only its in-frame MIs; the
        // out-of-grid cells are skipped without panicking.
        let mut state = TileIntraJointModeState::new(4, 4).unwrap();
        state.record_block(2, 2, 16, 16, D135_JOINT_MODE);
        // In-frame neighbour read still resolves the written cell.
        assert_eq!(state.get_joint_mode(0, 2, 3, 1, 1), D135_JOINT_MODE);
    }
}
