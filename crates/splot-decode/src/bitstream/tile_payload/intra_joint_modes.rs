// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local AV2 § 5.20.5.3 intra neighbour state.

use std::collections::TryReserveError;

use super::cdf::block_context::IntraYMode;

const NON_DIRECTIONAL_MODES_COUNT: u8 = 5;
const DC_PRED_JOINT_MODE: u8 = 0;
const NO_MRL: u8 = 0;
const NO_FSC: u8 = 0;
const NO_DIP: u8 = 0;
pub(crate) const PALETTE_MAX_SIZE: usize = 8;
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
    row_start: usize,
    col_start: usize,
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
        Ok(Self {
            row_start: 0,
            col_start: 0,
            rows,
            cols,
            cells,
        })
    }

    fn with_origin(mut self, row_start: usize, col_start: usize) -> Self {
        self.row_start = row_start;
        self.col_start = col_start;
        self
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
            if row >= self.row_start.saturating_add(self.rows) {
                break;
            }
            for x in 0..n4w {
                let Some(col) = c.checked_add(x) else {
                    break;
                };
                if col >= self.col_start.saturating_add(self.cols) {
                    break;
                }
                if let Some(index) = self.cell_index(row, col) {
                    self.cells[index] = value;
                }
            }
        }
    }

    fn cell_index(&self, row: usize, col: usize) -> Option<usize> {
        crate::support::rect_index(
            row,
            col,
            (self.row_start, self.col_start),
            (self.rows, self.cols),
        )
    }
}

fn require_nonzero<E>(value: usize, error: E) -> Result<(), E> {
    if value == 0 { Err(error) } else { Ok(()) }
}

macro_rules! mi_grid_new {
    ($err:ident, $default:expr, $mi_rows:expr, $mi_cols:expr, $precheck:expr $(,)?) => {
        MiGrid::new(
            $mi_rows,
            $mi_cols,
            $default,
            |mi_rows, mi_cols| $err::EmptyDimensions { mi_rows, mi_cols },
            |operation, left, right| $err::ArithmeticOverflow {
                operation,
                left,
                right,
            },
            |source| $err::Allocation { source },
            $precheck,
        )
    };
}

macro_rules! impl_grid_origin {
    ($($state:ty),+ $(,)?) => {
        $(
            impl $state {
                pub(crate) fn with_origin(
                    mut self,
                    row_start: usize,
                    col_start: usize,
                ) -> Self {
                    self.grid = self.grid.with_origin(row_start, col_start);
                    self
                }
            }
        )+
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraJointModeState {
    grid: MiGrid<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaPalette {
    size: u8,
    colors: [u16; PALETTE_MAX_SIZE],
}

impl LumaPalette {
    pub(crate) fn new(size: u8, colors: [u16; PALETTE_MAX_SIZE]) -> Option<Self> {
        let size_usize = usize::from(size);
        if !(2..=PALETTE_MAX_SIZE).contains(&size_usize) {
            return None;
        }
        Some(Self { size, colors })
    }

    pub(crate) const fn size(self) -> usize {
        self.size as usize
    }

    pub(crate) fn colors(self) -> [u16; PALETTE_MAX_SIZE] {
        self.colors
    }

    pub(crate) fn sample(self, color_index: u8) -> Option<u16> {
        let color_index = usize::from(color_index);
        (color_index < self.size()).then_some(self.colors[color_index])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileLumaPaletteState {
    grid: MiGrid<Option<LumaPalette>>,
}

impl TileLumaPaletteState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileLumaPaletteStateError> {
        let grid = mi_grid_new!(
            TileLumaPaletteStateError,
            None::<LumaPalette>,
            mi_rows,
            mi_cols,
            require_nonzero(sb_size4, TileLumaPaletteStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid })
    }

    pub(crate) fn palette_cache(&self, r: usize, c: usize) -> ([u16; 2 * PALETTE_MAX_SIZE], usize) {
        const MIN_SB_SIZE4: usize = 16;

        let above = if r != 0 && !r.is_multiple_of(MIN_SB_SIZE4) {
            self.grid.cell(r - 1, c).flatten()
        } else {
            None
        };
        let left = c
            .checked_sub(1)
            .and_then(|col| self.grid.cell(r, col).flatten());
        let mut cache = [0u16; 2 * PALETTE_MAX_SIZE];
        let mut len = 0usize;
        let mut above_idx = 0usize;
        let mut left_idx = 0usize;
        let mut above_remaining = above.map_or(0, LumaPalette::size);
        let mut left_remaining = left.map_or(0, LumaPalette::size);
        let above_colors = above.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);
        let left_colors = left.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);

        while above_remaining > 0 && left_remaining > 0 {
            push_palette_cache(&mut cache, &mut len, above_colors[above_idx]);
            above_idx += 1;
            above_remaining -= 1;
            push_palette_cache(&mut cache, &mut len, left_colors[left_idx]);
            left_idx += 1;
            left_remaining -= 1;
        }
        while above_remaining > 0 {
            push_palette_cache(&mut cache, &mut len, above_colors[above_idx]);
            above_idx += 1;
            above_remaining -= 1;
        }
        while left_remaining > 0 {
            push_palette_cache(&mut cache, &mut len, left_colors[left_idx]);
            left_idx += 1;
            left_remaining -= 1;
        }
        (cache, len)
    }

    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        palette: Option<LumaPalette>,
    ) {
        self.grid.record_block((r, c), (n4w, n4h), palette);
    }

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid.record_block((r, c), (n4w, n4h), None);
    }
}

fn push_palette_cache(cache: &mut [u16; 2 * PALETTE_MAX_SIZE], len: &mut usize, value: u16) {
    if *len < cache.len() {
        cache[*len] = value;
        *len += 1;
    }
}

impl TileIntraJointModeState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<Self, TileIntraJointModeStateError> {
        let grid = mi_grid_new!(
            TileIntraJointModeStateError,
            DC_PRED_JOINT_MODE,
            mi_rows,
            mi_cols,
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

    pub(crate) fn y_mode_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [left, above] = self.neighbour_joint_modes(r, c, n4w, n4h);
        (left >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (above >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }

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

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid
            .record_block((r, c), (n4w, n4h), DC_PRED_JOINT_MODE);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUsesMrlsState {
    grid: MiGrid<u8>,
    sb_size4: usize,
}

impl TileUsesMrlsState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileUsesMrlsStateError> {
        let grid = mi_grid_new!(
            TileUsesMrlsStateError,
            NO_MRL,
            mi_rows,
            mi_cols,
            require_nonzero(sb_size4, TileUsesMrlsStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid, sb_size4 })
    }

    pub(crate) fn mrl_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_uses_mrls(r, c, n4w, n4h);
        (first > 0) as usize + (second > 0) as usize
    }

    pub(crate) fn mrl_sec_index_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_uses_mrls(r, c, n4w, n4h);
        (first == 2) as usize + (second == 2) as usize
    }

    pub(crate) fn neighbour_uses_mrls(
        &self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
    ) -> [u8; 2] {
        npos_grid_values(NO_MRL, &self.grid, r, c, n4w, n4h, self.sb_size4)
    }

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

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid.record_block((r, c), (n4w, n4h), NO_MRL);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUseDipState {
    grid: MiGrid<u8>,
    sb_size4: usize,
}

impl TileUseDipState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileUseDipStateError> {
        let grid = mi_grid_new!(
            TileUseDipStateError,
            NO_DIP,
            mi_rows,
            mi_cols,
            require_nonzero(sb_size4, TileUseDipStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid, sb_size4 })
    }

    pub(crate) fn use_dip_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = npos_grid_values(NO_DIP, &self.grid, r, c, n4w, n4h, self.sb_size4);
        usize::from(first != 0) + usize::from(second != 0)
    }

    pub(crate) fn record_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize, use_dip: u8) {
        self.grid
            .record_block((r, c), (n4w, n4h), u8::from(use_dip != 0));
    }

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid.record_block((r, c), (n4w, n4h), NO_DIP);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileSegmentIdState {
    grid: MiGrid<u8>,
}

impl TileSegmentIdState {
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileSegmentIdStateError> {
        let grid = mi_grid_new!(TileSegmentIdStateError, 0u8, mi_rows, mi_cols, Ok(()))?;
        Ok(Self { grid })
    }

    pub(crate) fn cell(&self, r: usize, c: usize) -> Option<u8> {
        self.grid.cell(r, c)
    }

    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        segment_id: u8,
    ) {
        self.grid.record_block((r, c), (n4w, n4h), segment_id);
    }

    pub(crate) fn predictor_and_ctx(
        &self,
        r: usize,
        c: usize,
        avail_u: bool,
        avail_l: bool,
    ) -> (u8, usize) {
        let cell = |rr: usize, cc: usize| self.grid.cell(rr, cc).map_or(-1i16, i16::from);
        let prev_ul = if avail_u && avail_l {
            match (r.checked_sub(1), c.checked_sub(1)) {
                (Some(ru), Some(cl)) => cell(ru, cl),
                _ => -1,
            }
        } else {
            -1
        };
        let prev_u = if avail_u {
            r.checked_sub(1).map_or(-1, |ru| cell(ru, c))
        } else {
            -1
        };
        let prev_l = if avail_l {
            c.checked_sub(1).map_or(-1, |cl| cell(r, cl))
        } else {
            -1
        };
        let pred = if prev_u == -1 {
            if prev_l == -1 { 0 } else { prev_l }
        } else if prev_l == -1 || prev_ul == prev_u {
            prev_u
        } else {
            prev_l
        };
        let ctx = if prev_ul < 0 {
            0
        } else if prev_ul == prev_u && prev_ul == prev_l {
            2
        } else {
            usize::from(prev_ul == prev_u || prev_ul == prev_l || prev_u == prev_l)
        };
        (u8::try_from(pred.max(0)).unwrap_or(0), ctx)
    }
}

pub(crate) fn neg_deinterleave(diff: i32, reference: i32, max: i32) -> i32 {
    if reference == 0 {
        return diff;
    }
    if reference >= max - 1 {
        return max - diff - 1;
    }
    if 2 * reference < max {
        if diff <= 2 * reference {
            if diff & 1 != 0 {
                reference + ((diff + 1) >> 1)
            } else {
                reference - (diff >> 1)
            }
        } else {
            diff
        }
    } else if diff <= 2 * (max - reference - 1) {
        if diff & 1 != 0 {
            reference + ((diff + 1) >> 1)
        } else {
            reference - (diff >> 1)
        }
    } else {
        max - (diff + 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileFscModeState {
    grid: MiGrid<u8>,
    sb_size4: usize,
}

impl TileFscModeState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileFscModeStateError> {
        let grid = mi_grid_new!(
            TileFscModeStateError,
            NO_FSC,
            mi_rows,
            mi_cols,
            require_nonzero(sb_size4, TileFscModeStateError::EmptySuperblockSize),
        )?;
        Ok(Self { grid, sb_size4 })
    }

    pub(crate) fn fsc_mode_ctx(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> usize {
        let [first, second] = self.neighbour_fsc_modes(r, c, n4w, n4h);
        usize::from(first) + usize::from(second)
    }

    pub(crate) fn fsc_mode_at(&self, r: usize, c: usize) -> Option<u8> {
        self.grid.cell(r, c)
    }

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

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid.record_block((r, c), (n4w, n4h), NO_FSC);
    }

    fn neighbour_fsc_modes(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> [u8; 2] {
        npos_grid_values(NO_FSC, &self.grid, r, c, n4w, n4h, self.sb_size4)
    }
}

fn npos_grid_values(
    default: u8,
    grid: &MiGrid<u8>,
    r: usize,
    c: usize,
    n4w: usize,
    n4h: usize,
    sb_size4: usize,
) -> [u8; 2] {
    npos_neighbour_values(default, (r, c), (n4w, n4h), r, sb_size4, |row, col| {
        grid.cell(row, col)
    })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IsCflContext(usize);

impl IsCflContext {
    #[must_use]
    pub(crate) const fn new(ctx: usize) -> Self {
        Self(ctx)
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileUvCflState {
    grid: MiGrid<u8>,
}

impl TileUvCflState {
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
pub(crate) enum TileLumaPaletteStateError {
    #[error("intra luma palette state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra luma palette state requires non-empty superblock size")]
    EmptySuperblockSize,
    #[error("intra luma palette state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra luma palette state allocation failed: {source}")]
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
pub(crate) enum TileUseDipStateError {
    #[error("intra UseDip state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra UseDip state requires non-empty superblock size")]
    EmptySuperblockSize,
    #[error("intra UseDip state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra UseDip state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileSegmentIdStateError {
    #[error("intra SegmentIds state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra SegmentIds state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra SegmentIds state allocation failed: {source}")]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraYModeFacts {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraYModeState {
    grid: MiGrid<Option<TileIntraYModeFacts>>,
}

impl TileIntraYModeState {
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileIntraYModeStateError> {
        let grid = MiGrid::new(
            mi_rows,
            mi_cols,
            None::<TileIntraYModeFacts>,
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

    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        y_mode: IntraYMode,
        angle_delta_y: i8,
    ) {
        self.grid.record_block(
            (r, c),
            (n4w, n4h),
            Some(TileIntraYModeFacts {
                y_mode,
                angle_delta_y,
            }),
        );
    }

    pub(crate) fn record_non_intra_block(&mut self, r: usize, c: usize, n4w: usize, n4h: usize) {
        self.grid.record_block((r, c), (n4w, n4h), None);
    }

    pub(crate) fn y_mode_facts_at(&self, row: usize, col: usize) -> Option<TileIntraYModeFacts> {
        self.grid.cell(row, col).flatten()
    }
}

impl_grid_origin!(
    TileFscModeState,
    TileIntraJointModeState,
    TileIntraYModeState,
    TileLumaPaletteState,
    TileSegmentIdState,
    TileUseDipState,
    TileUsesMrlsState,
    TileUvCflState,
);

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
#[path = "intra_joint_modes_tests.rs"]
mod tests;
