// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.5.3 per-MI intra neighbour-mode state.
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
//! This module tracks that per-MI `IntraJointModes` grid, plus the sibling
//! `UsesMrls` and `FscModes` grids used by MRL/FSC symbol contexts, for the
//! general intra partition walk so contexts can be derived from real decoded
//! neighbours instead of hardcoded tile-origin `ctx == 0`.

use std::collections::TryReserveError;

use super::cdf::block_context::IntraYMode;

/// AV2 § 3 `NON_DIRECTIONAL_MODES_COUNT` (`03-symbols.md`): the number of
/// non-directional intra modes; a `modeDelta` at or above this is directional.
const NON_DIRECTIONAL_MODES_COUNT: u8 = 5;

/// AV2 `DC_PRED` (intra mode `0`), and the `IntraJointMode` (`modeDelta`) value
/// `get_joint_mode` returns for an out-of-frame neighbour
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`, `get_joint_mode`).
/// `DC_PRED < NON_DIRECTIONAL_MODES_COUNT`, so an out-of-frame neighbour is
/// non-directional and contributes `0` to the § 8.3.2 context.
const DC_PRED_JOINT_MODE: u8 = 0;
/// AV2 § 5.20.5.3 `UsesMrls` value for no MRL reference sample offset.
const NO_MRL: u8 = 0;
/// AV2 § 5.20.5.3 `FscModes` value for ordinary transform coding.
const NO_FSC: u8 = 0;

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
            let Some(mv_row) = r.checked_sub(1) else {
                return DC_PRED_JOINT_MODE;
            };
            (mv_row, c.saturating_add(n4w.saturating_sub(1)))
        } else {
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
        let [left, above] = self.neighbour_joint_modes(r, c, n4w, n4h);
        (left >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (above >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }

    /// Returns the left and above `get_joint_mode` values for §5.20.5.5
    /// `get_intra_y_mode_set`, in spec `dir` order: `[dir == 0, dir == 1]`.
    pub(crate) fn neighbour_joint_modes(
        &self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
    ) -> [u8; 2] {
        [
            self.get_joint_mode(0, r, c, n4w, n4h),
            self.get_joint_mode(1, r, c, n4w, n4h),
        ]
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
        write_mi_grid_block(
            &mut self.joint_modes,
            (self.mi_rows, self.mi_cols),
            (r, c),
            (n4w, n4h),
            joint_mode,
        );
    }

    /// Reads `IntraJointModes[row][col]` when inside the grid.
    fn cell(&self, row: usize, col: usize) -> Option<u8> {
        self.cell_index(row, col)
            .map(|index| self.joint_modes[index])
    }

    /// Row-major grid index for (`row`, `col`), or `None` when out of bounds.
    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        mi_grid_cell_index((self.mi_rows, self.mi_cols), row, col)
    }
}

/// Mutable tile-local AV2 § 5.20.5.3 `UsesMrls[r][c]` grid.
///
/// Each cell stores the block's derived `UsesMrls` value for every MI unit it
/// covers:
///
/// - `0` when `mrl_index == 0`
/// - `1` when `mrl_index > 0 && mrl_sec_index == 0`
/// - `2` when `mrl_index > 0 && mrl_sec_index != 0`
///
/// AV2 § 8.3.2 derives both MRL CDF contexts from the first two `NPos` cells
/// populated by § 5.20.4.1 `add_neighbor`. Unlike `get_joint_mode`, `NPos`
/// excludes neighbours from a different superblock row and may use fallback
/// `(r, c - 1)` / `(r - 1, c)` positions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUsesMrlsState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    /// Row-major `UsesMrls` grid (`mi_rows * mi_cols` cells).
    uses_mrls: Vec<u8>,
}

impl TileUsesMrlsState {
    /// Creates a `UsesMrls` grid for the given tile MI dimensions, initialized to
    /// `0` (matching out-of-frame or not-yet-decoded neighbours). `sb_size4` is
    /// AV2 § 5.20.2.1 `sbSize4 = Num_4x4_Blocks_Wide[SbSize]`, used by
    /// § 5.20.4.1 `add_neighbor`'s `aboveSbBoundary` filter.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileUsesMrlsStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileUsesMrlsStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        if sb_size4 == 0 {
            return Err(TileUsesMrlsStateError::EmptySuperblockSize);
        }
        let cells =
            mi_rows
                .checked_mul(mi_cols)
                .ok_or(TileUsesMrlsStateError::ArithmeticOverflow {
                    operation: "mi_rows * mi_cols",
                    left: mi_rows,
                    right: mi_cols,
                })?;
        let mut uses_mrls = Vec::new();
        uses_mrls
            .try_reserve_exact(cells)
            .map_err(|source| TileUsesMrlsStateError::Allocation { source })?;
        uses_mrls.resize(cells, NO_MRL);
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4,
            uses_mrls,
        })
    }

    /// AV2 § 8.3.2 `mrl_index` CDF context for the block at MI position
    /// (`r`, `c`) with `n4w`/`n4h` MI width/height.
    ///
    /// `ctx += UsesMrls[NPos[n][0]][NPos[n][1]] > 0` for `n in 0..NNum`, so the
    /// result is in `0..=2`.
    pub(crate) fn mrl_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_uses_mrls(r, c, n4w, n4h);
        (first > 0) as usize + (second > 0) as usize
    }

    /// AV2 § 8.3.2 `mrl_sec_index` CDF context for the block at MI position
    /// (`r`, `c`) with `n4w`/`n4h` MI width/height.
    ///
    /// `ctx += UsesMrls[NPos[n][0]][NPos[n][1]] == 2` for `n in 0..NNum`, so the
    /// result is in `0..=2`.
    pub(crate) fn mrl_sec_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_uses_mrls(r, c, n4w, n4h);
        (first == 2) as usize + (second == 2) as usize
    }

    /// Returns the first two `UsesMrls` values selected by AV2 § 5.20.4.1
    /// `add_neighbor` / `NPos` order, with missing entries left at `0`.
    pub(crate) fn neighbour_uses_mrls(
        &self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
    ) -> [u8; 2] {
        let mut values = [NO_MRL; 2];
        let mut len = 0;
        let bottom = r.checked_add(n4h.saturating_sub(1));
        let right = c.checked_add(n4w.saturating_sub(1));

        self.add_uses_mrls_neighbor(&mut values, &mut len, r, bottom, c.checked_sub(1));
        self.add_uses_mrls_neighbor(&mut values, &mut len, r, r.checked_sub(1), right);
        self.add_uses_mrls_neighbor(&mut values, &mut len, r, Some(r), c.checked_sub(1));
        self.add_uses_mrls_neighbor(&mut values, &mut len, r, r.checked_sub(1), Some(c));

        values
    }

    /// Writes the block's derived `UsesMrls` value into every MI cell it covers.
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        uses_mrls: u8,
    ) {
        write_mi_grid_block(
            &mut self.uses_mrls,
            (self.mi_rows, self.mi_cols),
            (r, c),
            (n4w, n4h),
            uses_mrls,
        );
    }

    fn add_uses_mrls_neighbor(
        &self,
        values: &mut [u8; 2],
        len: &mut usize,
        current_row: usize,
        row: Option<usize>,
        col: Option<usize>,
    ) {
        if *len >= 2 {
            return;
        }
        let (Some(row), Some(col)) = (row, col) else {
            return;
        };
        if current_row / self.sb_size4 != row / self.sb_size4 {
            return;
        }
        if let Some(value) = self.cell(row, col) {
            values[*len] = value;
            *len += 1;
        }
    }

    fn cell(&self, row: usize, col: usize) -> Option<u8> {
        self.cell_index(row, col).map(|index| self.uses_mrls[index])
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        mi_grid_cell_index((self.mi_rows, self.mi_cols), row, col)
    }
}

/// Mutable tile-local AV2 § 5.20.5.3 `FscModes[r][c]` grid.
///
/// AV2 § 8.3.2 derives the intra `fsc_mode` CDF context from the first two
/// `NPos` cells populated by § 5.20.4.1 `add_neighbor`, summing their stored
/// `FscModes` values for intra blocks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileFscModeState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    fsc_modes: Vec<u8>,
}

impl TileFscModeState {
    /// Creates a `FscModes` grid initialized to `0`.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileFscModeStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileFscModeStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        if sb_size4 == 0 {
            return Err(TileFscModeStateError::EmptySuperblockSize);
        }
        let cells =
            mi_rows
                .checked_mul(mi_cols)
                .ok_or(TileFscModeStateError::ArithmeticOverflow {
                    operation: "mi_rows * mi_cols",
                    left: mi_rows,
                    right: mi_cols,
                })?;
        let mut fsc_modes = Vec::new();
        fsc_modes
            .try_reserve_exact(cells)
            .map_err(|source| TileFscModeStateError::Allocation { source })?;
        fsc_modes.resize(cells, NO_FSC);
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4,
            fsc_modes,
        })
    }

    /// AV2 § 8.3.2 `fsc_mode` CDF context for an intra block.
    pub(crate) fn fsc_mode_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_fsc_modes(r, c, n4w, n4h);
        usize::from(first) + usize::from(second)
    }

    /// Writes the block's decoded `fsc_mode` value into every MI cell it covers.
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        fsc_mode: u8,
    ) {
        write_mi_grid_block(
            &mut self.fsc_modes,
            (self.mi_rows, self.mi_cols),
            (r, c),
            (n4w, n4h),
            fsc_mode,
        );
    }

    fn neighbour_fsc_modes(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> [u8; 2] {
        let mut values = [NO_FSC; 2];
        let mut len = 0;
        let bottom = r.checked_add(n4h.saturating_sub(1));
        let right = c.checked_add(n4w.saturating_sub(1));

        self.add_fsc_neighbor(&mut values, &mut len, r, bottom, c.checked_sub(1));
        self.add_fsc_neighbor(&mut values, &mut len, r, r.checked_sub(1), right);
        self.add_fsc_neighbor(&mut values, &mut len, r, Some(r), c.checked_sub(1));
        self.add_fsc_neighbor(&mut values, &mut len, r, r.checked_sub(1), Some(c));

        values
    }

    fn add_fsc_neighbor(
        &self,
        values: &mut [u8; 2],
        len: &mut usize,
        current_row: usize,
        row: Option<usize>,
        col: Option<usize>,
    ) {
        if *len >= 2 {
            return;
        }
        let (Some(row), Some(col)) = (row, col) else {
            return;
        };
        if current_row / self.sb_size4 != row / self.sb_size4 {
            return;
        }
        if let Some(value) = self.cell(row, col) {
            values[*len] = value;
            *len += 1;
        }
    }

    fn cell(&self, row: usize, col: usize) -> Option<u8> {
        self.cell_index(row, col).map(|index| self.fsc_modes[index])
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        mi_grid_cell_index((self.mi_rows, self.mi_cols), row, col)
    }
}

/// Row-major flat index of MI cell (`row`, `col`) in a `dims = (mi_rows, mi_cols)`
/// grid, or `None` when out of bounds. Shared by the tile-local per-MI neighbour
/// grids.
fn mi_grid_cell_index(dims: (usize, usize), row: usize, col: usize) -> Option<usize> {
    let (mi_rows, mi_cols) = dims;
    if row >= mi_rows || col >= mi_cols {
        return None;
    }
    row.checked_mul(mi_cols)?.checked_add(col)
}

/// Writes `value` into every in-grid MI cell covered by the `extent = (n4w, n4h)`
/// block at `pos = (r, c)` in a `dims = (mi_rows, mi_cols)` grid, clipping at the
/// frame edge. Shared by the tile-local per-MI neighbour grids' `record_block`
/// writers.
fn write_mi_grid_block(
    grid: &mut [u8],
    dims: (usize, usize),
    pos: (usize, usize),
    extent: (usize, usize),
    value: u8,
) {
    let (mi_rows, mi_cols) = dims;
    let (r, c) = pos;
    let (n4w, n4h) = extent;
    for y in 0..n4h {
        let Some(row) = r.checked_add(y) else { break };
        if row >= mi_rows {
            break;
        }
        for x in 0..n4w {
            let Some(col) = c.checked_add(x) else { break };
            if col >= mi_cols {
                break;
            }
            if let Some(index) = mi_grid_cell_index(dims, row, col) {
                grid[index] = value;
            }
        }
    }
}

/// AV2 § 8.3.2 `is_cfl` CDF context (`0..=2`) for a leaf block, derived from the
/// chroma above/left `UVCfls` neighbours. Passed to the leaf decode so the chroma
/// `is_cfl` symbol reads `TileIsCflCdf[ctx]` instead of a hardcoded `ctx == 0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IsCflContext(usize);

impl IsCflContext {
    /// Wraps the derived `is_cfl` context value.
    #[must_use]
    pub(crate) const fn new(ctx: usize) -> Self {
        Self(ctx)
    }

    /// The `TileIsCflCdf[ctx]` index.
    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// Mutable tile-local AV2 § 5.20.5.3 `UVCfls[r][c]` grid.
///
/// Each cell stores `!is_inter && (UVMode == UV_CFL_PRED)` for every chroma MI
/// unit a decoded block covers
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md` line 10102). AV2 § 8.3.2
/// derives the `is_cfl` CDF context from the directly-above and directly-left
/// chroma neighbours:
///
/// ```text
/// ctx = 0
/// if ( AvailUChroma && UVCfls[ ChromaMiRow - 1 ][ ChromaMiCol ] )   ctx += 1
/// if ( AvailLChroma && UVCfls[ ChromaMiRow ][ ChromaMiCol - 1 ] )   ctx += 1
/// ```
///
/// (`docs/spec/av2/1.0.0/08-parsing-process.md` line 658). The grid is tile-local
/// and initialized to `0`, so out-of-tile neighbours contribute `0`, matching the
/// `AvailUChroma` / `AvailLChroma` `is_inside` gate the caller applies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUvCflState {
    mi_rows: usize,
    mi_cols: usize,
    /// Row-major `UVCfls` grid (`mi_rows * mi_cols` cells).
    uv_cfls: Vec<u8>,
}

impl TileUvCflState {
    /// Creates a `UVCfls` grid for the given tile MI dimensions, initialized to
    /// `0` (matching out-of-frame or not-yet-decoded chroma neighbours).
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileUvCflStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileUvCflStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        let cells =
            mi_rows
                .checked_mul(mi_cols)
                .ok_or(TileUvCflStateError::ArithmeticOverflow {
                    operation: "mi_rows * mi_cols",
                    left: mi_rows,
                    right: mi_cols,
                })?;
        let mut uv_cfls = Vec::new();
        uv_cfls
            .try_reserve_exact(cells)
            .map_err(|source| TileUvCflStateError::Allocation { source })?;
        uv_cfls.resize(cells, 0);
        Ok(Self {
            mi_rows,
            mi_cols,
            uv_cfls,
        })
    }

    /// AV2 § 8.3.2 `is_cfl` CDF context for the chroma block at MI position
    /// (`r`, `c`). `avail_u` / `avail_l` are the caller-computed
    /// `AvailUChroma` / `AvailLChroma` `is_inside` results; the result is in
    /// `0..=2`.
    pub(crate) fn is_cfl_ctx(&self, r: usize, c: usize, avail_u: bool, avail_l: bool) -> usize {
        let above = avail_u
            && r.checked_sub(1)
                .and_then(|row| self.cell(row, c))
                .is_some_and(|value| value != 0);
        let left = avail_l
            && c.checked_sub(1)
                .and_then(|col| self.cell(r, col))
                .is_some_and(|value| value != 0);
        usize::from(above) + usize::from(left)
    }

    /// Writes the block's `UVCfls` value into every chroma MI cell it covers.
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_cfl: bool,
    ) {
        write_mi_grid_block(
            &mut self.uv_cfls,
            (self.mi_rows, self.mi_cols),
            (r, c),
            (n4w, n4h),
            u8::from(is_cfl),
        );
    }

    fn cell(&self, row: usize, col: usize) -> Option<u8> {
        self.cell_index(row, col).map(|index| self.uv_cfls[index])
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        mi_grid_cell_index((self.mi_rows, self.mi_cols), row, col)
    }
}

/// Error raised while building or sizing the `UVCfls` grid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileUvCflStateError {
    /// The tile MI dimensions were empty.
    #[error("intra UVCfls state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// A dimension product overflowed `usize`.
    #[error("intra UVCfls state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        /// The overflowing operation.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// The grid allocation failed.
    #[error("intra UVCfls state allocation failed: {source}")]
    Allocation {
        /// The underlying reservation error.
        source: TryReserveError,
    },
}

/// Error raised while building or sizing the `FscModes` grid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileFscModeStateError {
    /// The tile MI dimensions were empty.
    #[error("intra FscModes state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// The superblock width in MI units was empty.
    #[error("intra FscModes state requires non-empty superblock size")]
    EmptySuperblockSize,
    /// A dimension product overflowed `usize`.
    #[error("intra FscModes state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        /// The overflowing operation.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// The grid allocation failed.
    #[error("intra FscModes state allocation failed: {source}")]
    Allocation {
        /// The underlying reservation error.
        source: TryReserveError,
    },
}

/// Error raised while building or sizing the `UsesMrls` grid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileUsesMrlsStateError {
    /// The tile MI dimensions were empty.
    #[error("intra UsesMrls state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// The superblock width in MI units was empty.
    #[error("intra UsesMrls state requires non-empty superblock size")]
    EmptySuperblockSize,
    /// A dimension product overflowed `usize`.
    #[error("intra UsesMrls state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        /// The overflowing operation.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// The grid allocation failed.
    #[error("intra UsesMrls state allocation failed: {source}")]
    Allocation {
        /// The underlying reservation error.
        source: TryReserveError,
    },
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

/// Mutable tile-local AV2 § 5.20.5.3 `YModes[r][c]` grid.
///
/// SDP chroma partition leaves do not read `read_intra_y_mode()`. Instead, AV2
/// §5.20.5.3 copies `YMode = YModes[MiRow][MiCol]` from the already-decoded luma
/// side before reading chroma mode syntax. This grid stores that luma result for
/// every MI cell covered by a luma/shared leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraYModeState {
    mi_rows: usize,
    mi_cols: usize,
    y_modes: Vec<Option<IntraYMode>>,
}

impl TileIntraYModeState {
    /// Creates an empty `YModes` grid for the given tile MI dimensions.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileIntraYModeStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileIntraYModeStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        let cells =
            mi_rows
                .checked_mul(mi_cols)
                .ok_or(TileIntraYModeStateError::ArithmeticOverflow {
                    operation: "mi_rows * mi_cols",
                    left: mi_rows,
                    right: mi_cols,
                })?;
        let mut y_modes = Vec::new();
        y_modes
            .try_reserve_exact(cells)
            .map_err(|source| TileIntraYModeStateError::Allocation { source })?;
        y_modes.resize(cells, None);
        Ok(Self {
            mi_rows,
            mi_cols,
            y_modes,
        })
    }

    /// Writes a luma/shared block's `YMode` into every MI cell it covers.
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        y_mode: IntraYMode,
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
                    self.y_modes[index] = Some(y_mode);
                }
            }
        }
    }

    /// Reads the stored luma `YMode` for a chroma-only SDP block.
    pub(crate) fn y_mode_at(&self, row: usize, col: usize) -> Option<IntraYMode> {
        self.cell_index(row, col)
            .and_then(|index| self.y_modes[index])
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return None;
        }
        row.checked_mul(self.mi_cols)?.checked_add(col)
    }
}

/// Error raised while building or sizing the `YModes` grid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileIntraYModeStateError {
    /// The tile MI dimensions were empty.
    #[error("intra YMode state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// A dimension product overflowed `usize`.
    #[error("intra YMode state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        /// The overflowing operation.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// The grid allocation failed.
    #[error("intra YMode state allocation failed: {source}")]
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
    /// A 64x64 superblock is 16x16 MI units.
    const SB_N4: usize = 16;

    #[test]
    fn out_of_frame_neighbours_give_context_zero() {
        let state = TileIntraJointModeState::new(16, 16).unwrap();
        assert_eq!(state.y_mode_index_ctx(0, 0, 16, 16), 0);
    }

    #[test]
    fn non_directional_neighbour_keeps_context_zero() {
        let mut state = TileIntraJointModeState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, SMOOTH_V_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 0);
    }

    #[test]
    fn directional_left_neighbour_raises_context_to_one() {
        let mut state = TileIntraJointModeState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 1);
    }

    #[test]
    fn directional_above_neighbour_raises_context_to_one() {
        let mut state = TileIntraJointModeState::new(32, 16).unwrap();
        state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(16, 0, 16, 16), 1);
    }

    #[test]
    fn directional_both_neighbours_raise_context_to_two() {
        let mut state = TileIntraJointModeState::new(32, 32).unwrap();
        state.record_block(0, 16, 16, 16, D135_JOINT_MODE);
        state.record_block(16, 0, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.y_mode_index_ctx(16, 16, 16, 16), 2);
    }

    #[test]
    fn get_joint_mode_uses_the_spec_neighbour_positions() {
        let mut state = TileIntraJointModeState::new(8, 8).unwrap();
        state.record_block(3, 1, 1, 1, D135_JOINT_MODE);
        assert_eq!(state.get_joint_mode(0, 2, 2, 2, 2), D135_JOINT_MODE);
        state.record_block(1, 3, 1, 1, D135_JOINT_MODE);
        assert_eq!(state.get_joint_mode(1, 2, 2, 2, 2), D135_JOINT_MODE);
    }

    #[test]
    fn last_non_directional_mode_does_not_raise_the_context() {
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
        let mut state = TileIntraJointModeState::new(4, 4).unwrap();
        state.record_block(2, 2, 16, 16, D135_JOINT_MODE);
        assert_eq!(state.get_joint_mode(0, 2, 3, 1, 1), D135_JOINT_MODE);
    }

    #[test]
    fn uses_mrls_out_of_frame_neighbours_give_context_zero() {
        let state = TileUsesMrlsState::new(16, 16, SB_N4).unwrap();

        assert_eq!(state.mrl_index_ctx(0, 0, 16, 16), 0);
        assert_eq!(state.mrl_sec_index_ctx(0, 0, 16, 16), 0);
    }

    #[test]
    fn uses_mrls_neighbours_select_index_and_secondary_contexts() {
        let mut state = TileUsesMrlsState::new(32, 32, SB_N4).unwrap();
        state.record_block(7, 11, 1, 1, 2);
        state.record_block(11, 7, 1, 1, 1);

        assert_eq!(state.neighbour_uses_mrls(8, 8, 4, 4), [1, 2]);
        assert_eq!(state.mrl_index_ctx(8, 8, 4, 4), 2);
        assert_eq!(state.mrl_sec_index_ctx(8, 8, 4, 4), 1);
    }

    #[test]
    fn uses_mrls_npos_excludes_above_superblock_row_neighbours() {
        let mut state = TileUsesMrlsState::new(32, 32, SB_N4).unwrap();
        state.record_block(31, 15, 1, 1, 1);
        state.record_block(15, 31, 1, 1, 2);
        state.record_block(15, 16, 1, 1, 2);

        assert_eq!(state.neighbour_uses_mrls(16, 16, 16, 16), [1, 0]);
        assert_eq!(state.mrl_index_ctx(16, 16, 16, 16), 1);
        assert_eq!(state.mrl_sec_index_ctx(16, 16, 16, 16), 0);
    }

    #[test]
    fn uses_mrls_npos_uses_fallback_positions() {
        let mut state = TileUsesMrlsState::new(16, 16, SB_N4).unwrap();
        state.record_block(7, 3, 1, 1, 1);
        state.record_block(7, 0, 1, 1, 2);

        assert_eq!(state.neighbour_uses_mrls(8, 0, 4, 4), [1, 2]);
        assert_eq!(state.mrl_index_ctx(8, 0, 4, 4), 2);
        assert_eq!(state.mrl_sec_index_ctx(8, 0, 4, 4), 1);
    }

    #[test]
    fn uses_mrls_record_block_clips_to_the_grid() {
        let mut state = TileUsesMrlsState::new(4, 4, SB_N4).unwrap();
        state.record_block(2, 2, 16, 16, 2);

        assert_eq!(state.neighbour_uses_mrls(2, 3, 1, 1), [2, 0]);
        assert_eq!(state.mrl_index_ctx(0, 0, 1, 1), 0);
    }

    #[test]
    fn uses_mrls_empty_dimensions_are_rejected() {
        assert!(matches!(
            TileUsesMrlsState::new(0, 4, SB_N4),
            Err(TileUsesMrlsStateError::EmptyDimensions { .. })
        ));
        assert!(matches!(
            TileUsesMrlsState::new(4, 0, SB_N4),
            Err(TileUsesMrlsStateError::EmptyDimensions { .. })
        ));
        assert!(matches!(
            TileUsesMrlsState::new(4, 4, 0),
            Err(TileUsesMrlsStateError::EmptySuperblockSize)
        ));
    }

    #[test]
    fn fsc_modes_neighbours_select_context_sum() {
        let mut state = TileFscModeState::new(32, 32, SB_N4).unwrap();
        state.record_block(7, 11, 1, 1, 1);
        state.record_block(11, 7, 1, 1, 1);

        assert_eq!(state.fsc_mode_ctx(8, 8, 4, 4), 2);
    }

    #[test]
    fn fsc_modes_npos_excludes_above_superblock_row_neighbours() {
        let mut state = TileFscModeState::new(32, 32, SB_N4).unwrap();
        state.record_block(31, 15, 1, 1, 1);
        state.record_block(15, 31, 1, 1, 1);
        state.record_block(15, 16, 1, 1, 1);

        assert_eq!(state.fsc_mode_ctx(16, 16, 16, 16), 1);
    }

    #[test]
    fn fsc_modes_empty_dimensions_are_rejected() {
        assert!(matches!(
            TileFscModeState::new(0, 4, SB_N4),
            Err(TileFscModeStateError::EmptyDimensions { .. })
        ));
        assert!(matches!(
            TileFscModeState::new(4, 0, SB_N4),
            Err(TileFscModeStateError::EmptyDimensions { .. })
        ));
        assert!(matches!(
            TileFscModeState::new(4, 4, 0),
            Err(TileFscModeStateError::EmptySuperblockSize)
        ));
    }

    #[test]
    fn y_mode_state_records_and_clips_blocks() {
        let mut state = TileIntraYModeState::new(4, 4).unwrap();
        state.record_block(2, 2, 16, 16, IntraYMode::DC_PRED);

        assert_eq!(state.y_mode_at(2, 2), Some(IntraYMode::DC_PRED));
        assert_eq!(state.y_mode_at(3, 3), Some(IntraYMode::DC_PRED));
        assert_eq!(state.y_mode_at(0, 0), None);
        assert_eq!(state.y_mode_at(4, 4), None);
    }

    #[test]
    fn uv_cfl_out_of_frame_neighbours_give_context_zero() {
        let state = TileUvCflState::new(16, 16).unwrap();
        assert_eq!(state.is_cfl_ctx(0, 0, false, false), 0);
    }

    #[test]
    fn uv_cfl_non_cfl_neighbour_keeps_context_zero() {
        let mut state = TileUvCflState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, false);
        assert_eq!(state.is_cfl_ctx(0, 16, false, true), 0);
    }

    #[test]
    fn uv_cfl_left_neighbour_raises_context_to_one() {
        let mut state = TileUvCflState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, true);
        assert_eq!(state.is_cfl_ctx(0, 16, false, true), 1);
    }

    #[test]
    fn uv_cfl_above_neighbour_raises_context_to_one() {
        let mut state = TileUvCflState::new(32, 16).unwrap();
        state.record_block(0, 0, 16, 16, true);
        assert_eq!(state.is_cfl_ctx(16, 0, true, false), 1);
    }

    #[test]
    fn uv_cfl_both_neighbours_raise_context_to_two() {
        let mut state = TileUvCflState::new(32, 32).unwrap();
        state.record_block(0, 0, 16, 16, true); // above-left fills above (16, *)
        state.record_block(16, 0, 16, 16, true); // left of (16, 16)
        state.record_block(0, 16, 16, 16, true); // above of (16, 16)
        assert_eq!(state.is_cfl_ctx(16, 16, true, true), 2);
    }

    #[test]
    fn uv_cfl_availability_gate_overrides_a_cfl_neighbour() {
        let mut state = TileUvCflState::new(16, 32).unwrap();
        state.record_block(0, 0, 16, 16, true);
        assert_eq!(state.is_cfl_ctx(0, 16, false, false), 0);
    }

    #[test]
    fn uv_cfl_record_block_clips_to_the_grid_and_rejects_empty_dimensions() {
        let mut state = TileUvCflState::new(4, 4).unwrap();
        state.record_block(2, 2, 16, 16, true);
        assert_eq!(state.is_cfl_ctx(3, 3, true, true), 2);
        assert_eq!(state.is_cfl_ctx(2, 3, false, true), 1);
        assert!(TileUvCflState::new(0, 4).is_err());
        assert!(TileUvCflState::new(4, 0).is_err());
    }
}
