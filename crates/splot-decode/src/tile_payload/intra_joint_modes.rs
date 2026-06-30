// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local AV2 § 5.20.5.3 intra neighbour state.

use std::collections::TryReserveError;

use super::cdf::block_context::IntraYMode;

const NON_DIRECTIONAL_MODES_COUNT: u8 = 5;
const DC_PRED_JOINT_MODE: u8 = 0;
const NO_MRL: u8 = 0;
const NO_FSC: u8 = 0;
const JOINT_NEIGHBOUR_SAMPLES: [NeighbourSample; 2] =
    [NeighbourSample::LeftBottom, NeighbourSample::AboveRight];
const NPOS_NEIGHBOUR_SAMPLES: [NeighbourSample; 4] = [
    NeighbourSample::LeftBottom,
    NeighbourSample::AboveRight,
    NeighbourSample::LeftTop,
    NeighbourSample::AboveLeft,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NeighbourSample {
    LeftBottom,
    AboveRight,
    LeftTop,
    AboveLeft,
}

impl NeighbourSample {
    fn position(self, r: usize, c: usize, n4w: usize, n4h: usize) -> Option<(usize, usize)> {
        match self {
            Self::LeftBottom => Some((r.saturating_add(n4h.saturating_sub(1)), c.checked_sub(1)?)),
            Self::AboveRight => Some((r.checked_sub(1)?, c.saturating_add(n4w.saturating_sub(1)))),
            Self::LeftTop => Some((r, c.checked_sub(1)?)),
            Self::AboveLeft => Some((r.checked_sub(1)?, c)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MiGrid<T> {
    rows: usize,
    cols: usize,
    cells: Vec<T>,
}

impl<T: Copy> MiGrid<T> {
    fn new<E>(
        rows: usize,
        cols: usize,
        default: T,
        empty_dimensions: impl FnOnce(usize, usize) -> E,
        arithmetic_overflow: impl FnOnce(&'static str, usize, usize) -> E,
        allocation: impl FnOnce(TryReserveError) -> E,
        preallocate_check: Result<(), E>,
    ) -> Result<Self, E> {
        if rows == 0 || cols == 0 {
            return Err(empty_dimensions(rows, cols));
        }
        preallocate_check?;
        let len = rows
            .checked_mul(cols)
            .ok_or_else(|| arithmetic_overflow("mi_rows * mi_cols", rows, cols))?;
        let mut cells = Vec::new();
        cells.try_reserve_exact(len).map_err(allocation)?;
        cells.resize(len, default);
        Ok(Self { rows, cols, cells })
    }

    fn cell(&self, row: usize, col: usize) -> Option<T> {
        self.cell_index(row, col).map(|index| self.cells[index])
    }

    fn record_block(&mut self, pos: (usize, usize), extent: (usize, usize), value: T) {
        let (r, c) = pos;
        let (n4w, n4h) = extent;
        for y in 0..n4h {
            let Some(row) = r.checked_add(y) else {
                break;
            };
            if row >= self.rows {
                break;
            }
            for x in 0..n4w {
                let Some(col) = c.checked_add(x) else {
                    break;
                };
                if col >= self.cols {
                    break;
                }
                if let Some(index) = self.cell_index(row, col) {
                    self.cells[index] = value;
                }
            }
        }
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        row.checked_mul(self.cols)?.checked_add(col)
    }
}

fn require_nonzero<E>(value: usize, error: E) -> Result<(), E> {
    if value == 0 { Err(error) } else { Ok(()) }
}

/// Tile-local AV2 § 5.20.5.3 `IntraJointModes[r][c]` grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraJointModeState {
    grid: MiGrid<u8>,
}

impl TileIntraJointModeState {
    /// Creates a `DC_PRED`-initialized `IntraJointModes` grid.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<Self, TileIntraJointModeStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            DC_PRED_JOINT_MODE,
            |mi_rows, mi_cols| TileIntraJointModeStateError::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| TileIntraJointModeStateError::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| TileIntraJointModeStateError::Allocation { source },
            Ok(()),
        )?;
        Ok(Self { grid })
    }

    fn get_joint_mode(&self, dir: usize, r: usize, c: usize, n4w: usize, n4h: usize) -> u8 {
        let sample = JOINT_NEIGHBOUR_SAMPLES[usize::from(dir == 1)];
        neighbour_value_or(
            sample,
            DC_PRED_JOINT_MODE,
            (r, c),
            (n4w, n4h),
            |row, col| self.grid.cell(row, col),
        )
    }

    /// AV2 § 8.3.2 `y_mode_index` / `y_mode_offset` context.
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

    /// Writes the block's `IntraJointMode` into each covered in-grid MI cell.
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        joint_mode: u8,
    ) {
        self.grid.record_block((r, c), (n4w, n4h), joint_mode);
    }
}

/// Tile-local AV2 § 5.20.5.3 `UsesMrls[r][c]` grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUsesMrlsState {
    grid: MiGrid<u8>,
    sb_size4: usize,
}

impl TileUsesMrlsState {
    /// Creates a `UsesMrls` grid.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileUsesMrlsStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            NO_MRL,
            |mi_rows, mi_cols| TileUsesMrlsStateError::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| TileUsesMrlsStateError::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| TileUsesMrlsStateError::Allocation { source },
            require_nonzero(sb_size4, TileUsesMrlsStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid, sb_size4 })
    }

    /// AV2 § 8.3.2 `mrl_index` CDF context.
    pub(crate) fn mrl_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_uses_mrls(r, c, n4w, n4h);
        (first > 0) as usize + (second > 0) as usize
    }

    /// AV2 § 8.3.2 `mrl_sec_index` CDF context.
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
        npos_neighbour_values(NO_MRL, (r, c), (n4w, n4h), r, self.sb_size4, |row, col| {
            self.grid.cell(row, col)
        })
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
        self.grid.record_block((r, c), (n4w, n4h), uses_mrls);
    }
}

/// Tile-local AV2 § 5.20.5.3 `FscModes[r][c]` grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileFscModeState {
    grid: MiGrid<u8>,
    sb_size4: usize,
}

impl TileFscModeState {
    /// Creates a `FscModes` grid initialized to `0`.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileFscModeStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            NO_FSC,
            |mi_rows, mi_cols| TileFscModeStateError::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| TileFscModeStateError::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| TileFscModeStateError::Allocation { source },
            require_nonzero(sb_size4, TileFscModeStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid, sb_size4 })
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
        self.grid.record_block((r, c), (n4w, n4h), fsc_mode);
    }

    fn neighbour_fsc_modes(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> [u8; 2] {
        npos_neighbour_values(NO_FSC, (r, c), (n4w, n4h), r, self.sb_size4, |row, col| {
            self.grid.cell(row, col)
        })
    }
}

fn neighbour_value_or(
    sample: NeighbourSample,
    default: u8,
    pos: (usize, usize),
    extent: (usize, usize),
    mut cell: impl FnMut(usize, usize) -> Option<u8>,
) -> u8 {
    let (r, c) = pos;
    let (n4w, n4h) = extent;
    sample
        .position(r, c, n4w, n4h)
        .and_then(|(row, col)| cell(row, col))
        .unwrap_or(default)
}

fn gated_neighbour_values<const N: usize>(
    samples: [(NeighbourSample, bool); N],
    default: u8,
    pos: (usize, usize),
    mut cell: impl FnMut(usize, usize) -> Option<u8>,
) -> [u8; N] {
    let mut values = [default; N];
    let (r, c) = pos;
    for (index, (sample, is_available)) in samples.into_iter().enumerate() {
        if is_available && let Some((row, col)) = sample.position(r, c, 1, 1) {
            values[index] = cell(row, col).unwrap_or(default);
        }
    }
    values
}

fn npos_neighbour_values(
    default: u8,
    pos: (usize, usize),
    extent: (usize, usize),
    current_row: usize,
    sb_size4: usize,
    mut cell: impl FnMut(usize, usize) -> Option<u8>,
) -> [u8; 2] {
    let mut values = [default; 2];
    let mut len = 0;
    let (r, c) = pos;
    let (n4w, n4h) = extent;

    for sample in NPOS_NEIGHBOUR_SAMPLES {
        if len >= values.len() {
            break;
        }
        let Some((row, col)) = sample.position(r, c, n4w, n4h) else {
            continue;
        };
        if current_row / sb_size4 != row / sb_size4 {
            continue;
        }
        if let Some(value) = cell(row, col) {
            values[len] = value;
            len += 1;
        }
    }

    values
}

/// AV2 § 8.3.2 `is_cfl` CDF context.
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

/// Tile-local AV2 § 5.20.5.3 `UVCfls[r][c]` grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUvCflState {
    grid: MiGrid<u8>,
}

impl TileUvCflState {
    /// Creates a `UVCfls` grid.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileUvCflStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            0,
            |mi_rows, mi_cols| TileUvCflStateError::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| TileUvCflStateError::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| TileUvCflStateError::Allocation { source },
            Ok(()),
        )?;
        Ok(Self { grid })
    }

    /// AV2 § 8.3.2 `is_cfl` CDF context.
    pub(crate) fn is_cfl_ctx(&self, r: usize, c: usize, avail_u: bool, avail_l: bool) -> usize {
        let [above, left] = gated_neighbour_values(
            [
                (NeighbourSample::AboveLeft, avail_u),
                (NeighbourSample::LeftTop, avail_l),
            ],
            0,
            (r, c),
            |row, col| self.grid.cell(row, col),
        );
        usize::from(above != 0) + usize::from(left != 0)
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
        self.grid.record_block((r, c), (n4w, n4h), u8::from(is_cfl));
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileUvCflStateError {
    #[error("intra UVCfls state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra UVCfls state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra UVCfls state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileFscModeStateError {
    #[error("intra FscModes state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra FscModes state requires non-empty superblock size")]
    EmptySuperblockSize,
    #[error("intra FscModes state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra FscModes state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileUsesMrlsStateError {
    #[error("intra UsesMrls state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra UsesMrls state requires non-empty superblock size")]
    EmptySuperblockSize,
    #[error("intra UsesMrls state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra UsesMrls state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileIntraJointModeStateError {
    #[error("intra joint-mode state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra joint-mode state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra joint-mode state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

/// Tile-local AV2 § 5.20.5.3 `YModes[r][c]` grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraYModeState {
    grid: MiGrid<Option<IntraYMode>>,
}

impl TileIntraYModeState {
    /// Creates an empty `YModes` grid for the given tile MI dimensions.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileIntraYModeStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            None::<IntraYMode>,
            |mi_rows, mi_cols| TileIntraYModeStateError::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| TileIntraYModeStateError::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| TileIntraYModeStateError::Allocation { source },
            Ok(()),
        )?;
        Ok(Self { grid })
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
        self.grid.record_block((r, c), (n4w, n4h), Some(y_mode));
    }

    /// Reads the stored luma `YMode` for a chroma-only SDP block.
    pub(crate) fn y_mode_at(&self, row: usize, col: usize) -> Option<IntraYMode> {
        self.grid.cell(row, col).flatten()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileIntraYModeStateError {
    #[error("intra YMode state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra YMode state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra YMode state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const D135_JOINT_MODE: u8 = 36;
    const SMOOTH_V_JOINT_MODE: u8 = 2;
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
        state.record_block(0, 0, 16, 16, true);
        state.record_block(16, 0, 16, 16, true);
        state.record_block(0, 16, 16, 16, true);
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
