// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local AV2 § 5.20.5.3 intra neighbour state.

use std::collections::TryReserveError;
use std::num::NonZeroU32;
use std::ops::Range;

use splot_core::symbol::Symbol;

use super::cdf::block_context::{IntraJointMode, IntraYMode, MrlSelection};
use crate::support::reusable_scratch::{recycle_pooled_vec, take_pooled_vec};
use crate::tile::SbRowWindow;

pub(crate) const PALETTE_MAX_SIZE: usize = 8;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MiGrid<T> {
    origin_row: usize,
    origin_col: usize,
    rows: usize,
    cols: usize,
    window: SbRowWindow,
    /// [`SbRowWindow::readable_rows`] of `window`.
    readable: (usize, usize, usize),
    default: T,
    cells: Vec<T>,
}

impl<T: Copy> MiGrid<T> {
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn build<E>(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_h4: usize,
        default: T,
        mut cells: Vec<T>,
        empty_dimensions: impl FnOnce(usize, usize) -> E,
        arithmetic_overflow: impl FnOnce(&'static str, usize, usize) -> E,
        allocation: impl FnOnce(TryReserveError) -> E,
        preallocate_check: Result<(), E>,
    ) -> Result<Self, E> {
        let rows = row_range.end.saturating_sub(row_range.start);
        let cols = col_range.end.saturating_sub(col_range.start);
        if rows == 0 || cols == 0 {
            return Err(empty_dimensions(rows, cols));
        }
        preallocate_check?;
        let window = SbRowWindow::new(rows, sb_h4);
        let len = window
            .plane_rows()
            .checked_mul(cols)
            .ok_or_else(|| arithmetic_overflow("mi_rows * mi_cols", rows, cols))?;
        cells.clear();
        cells.try_reserve_exact(len).map_err(allocation)?;
        cells.resize(len, default);
        Ok(Self {
            origin_row: row_range.start,
            origin_col: col_range.start,
            rows,
            cols,
            window,
            readable: (0, 0, 0),
            default,
            cells,
        })
    }

    pub(crate) fn into_cells(self) -> Vec<T> {
        self.cells
    }

    pub(crate) fn window_violated(&self) -> bool {
        self.window.violated()
    }

    /// A tile cell outside the window reads as the default value.
    #[allow(clippy::inline_always, reason = "measured: per-block neighbour reads")]
    #[inline(always)]
    pub(crate) fn cell(&self, row: usize, col: usize) -> Option<T> {
        let row = row.wrapping_sub(self.origin_row);
        let col = col.wrapping_sub(self.origin_col);
        if row >= self.rows || col >= self.cols {
            return None;
        }
        Some(self.plane_row(row).map_or(self.default, |plane_row| {
            self.cells[plane_row * self.cols + col]
        }))
    }

    /// [`SbRowWindow::plane_row`] of tile row `row`, the readable rows inline.
    #[allow(clippy::inline_always, reason = "measured: per-block neighbour reads")]
    #[inline(always)]
    fn plane_row(&self, row: usize) -> Option<usize> {
        let (lo, hi, bias) = self.readable;
        if row.wrapping_sub(lo) < hi.wrapping_sub(lo) {
            return Some(row.wrapping_add(bias));
        }
        self.plane_row_outside_readable_rows(row)
    }

    #[cold]
    #[inline(never)]
    fn plane_row_outside_readable_rows(&self, row: usize) -> Option<usize> {
        self.window.plane_row(row)
    }

    fn with_origin(mut self, origin_row: usize, origin_col: usize) -> Self {
        self.origin_row = origin_row;
        self.origin_col = origin_col;
        self
    }

    pub(crate) fn record_block(&mut self, pos: (usize, usize), extent: (usize, usize), value: T) {
        let (n4w, n4h) = extent;
        let Some((r, c)) = self.enter_block(pos) else {
            return;
        };
        let row_end = r.saturating_add(n4h).min(self.rows);
        let col_end = c.saturating_add(n4w).min(self.cols);
        for row in r..row_end {
            if let Some(plane_row) = self.plane_row(row) {
                let start = plane_row * self.cols;
                crate::support::fill_mi_run(&mut self.cells[start + c..start + col_end], value);
            }
        }
    }

    /// [`MiGrid::record_block`] for a block that records the default value.
    ///
    /// Each cell belongs to one block per tile and holds the default from
    /// `build` or the window clear until that block records it, so only the
    /// window move remains.
    pub(crate) fn record_default_block(&mut self, pos: (usize, usize)) {
        self.enter_block(pos);
    }

    /// [`MiGrid::record_block`] that writes only a non-default value, for a
    /// grid that keeps the [`MiGrid::record_default_block`] cell ownership.
    pub(crate) fn record_owned_block(
        &mut self,
        pos: (usize, usize),
        extent: (usize, usize),
        value: T,
    ) where
        T: PartialEq,
    {
        if value == self.default {
            self.record_default_block(pos);
        } else {
            self.record_block(pos, extent, value);
        }
    }

    /// Moves the window down to the superblock row holding tile row `row`.
    pub(crate) fn enter_row(&mut self, row: usize) {
        self.enter_block((row, self.origin_col));
    }

    /// Calls `visit` with the cells of each tile row in `rows` the window
    /// holds, over the tile columns `cols`.
    pub(crate) fn visit_cells(
        &mut self,
        rows: Range<usize>,
        cols: &Range<usize>,
        mut visit: impl FnMut(&mut [T]),
    ) {
        let start = cols.start.saturating_sub(self.origin_col).min(self.cols);
        let end = cols.end.saturating_sub(self.origin_col).min(self.cols);
        for row in rows {
            let Some(plane_row) = row
                .checked_sub(self.origin_row)
                .and_then(|row| self.window.plane_row(row))
            else {
                continue;
            };
            let base = plane_row * self.cols;
            if let Some(cells) = self.cells.get_mut(base + start..base + end) {
                visit(cells);
            }
        }
    }

    /// Moves the window to a block at tile position `pos` and returns it
    /// tile-relative, or `None` outside the tile.
    #[allow(clippy::inline_always, reason = "measured: once per recorded block")]
    #[inline(always)]
    fn enter_block(&mut self, (r, c): (usize, usize)) -> Option<(usize, usize)> {
        let r = r.checked_sub(self.origin_row)?;
        let c = c.checked_sub(self.origin_col)?;
        if r >= self.rows || c >= self.cols {
            return None;
        }
        if r >= self.readable.1 {
            self.enter_row_of(r);
        }
        Some((r, c))
    }

    #[cold]
    #[inline(never)]
    fn enter_row_of(&mut self, r: usize) {
        if let Some(slide) = self.window.enter(r) {
            slide.apply(&mut self.cells, self.cols, self.default);
        }
        self.readable = self.window.readable_rows();
    }
}

impl<T: Copy + Send + 'static> MiGrid<T> {
    #[expect(clippy::too_many_arguments)]
    fn new_for_tile<E>(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_h4: usize,
        default: T,
        empty_dimensions: impl FnOnce(usize, usize) -> E,
        arithmetic_overflow: impl FnOnce(&'static str, usize, usize) -> E,
        allocation: impl FnOnce(TryReserveError) -> E,
        preallocate_check: Result<(), E>,
    ) -> Result<Self, E> {
        let cells = SbRowWindow::new(row_range.len(), sb_h4)
            .plane_rows()
            .saturating_mul(col_range.len());
        Self::build(
            row_range,
            col_range,
            sb_h4,
            default,
            take_pooled_vec::<T>(cells),
            empty_dimensions,
            arithmetic_overflow,
            allocation,
            preallocate_check,
        )
    }
}

fn require_nonzero<E>(value: usize, error: E) -> Result<(), E> {
    if value == 0 { Err(error) } else { Ok(()) }
}

macro_rules! mi_grid_new_for_tile {
    ($err:ident, $default:expr, $row_range:expr, $col_range:expr, $sb_h4:expr, $precheck:expr $(,)?) => {
        MiGrid::new_for_tile(
            $row_range,
            $col_range,
            $sb_h4,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaPalette {
    size: u8,
    colors: [u16; PALETTE_MAX_SIZE],
}

impl LumaPalette {
    /// Builds a 2 through 8 color palette from a `TilePaletteYSizeCdf` symbol.
    pub(crate) fn from_size_symbol(size_symbol: Symbol, colors: [u16; PALETTE_MAX_SIZE]) -> Self {
        Self {
            size: size_symbol.get() + 2,
            colors,
        }
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

/// The § 5.20.4.1 intra arrays of one mode-info position (`YModes` with its
/// angle, the joint mode, `FscModes`, `UseDip`, `UsesMrls`, the luma palette
/// and the § 7.11.2 smoothness of `YModes`), so a leaf fills one grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraFacts {
    palette: Option<NonZeroU32>,
    joint_mode: IntraJointMode,
    y_mode: IntraYMode,
    angle_delta_y: i8,
    bits: u8,
}

const _: () = assert!(size_of::<IntraFacts>() == 8);

impl IntraFacts {
    const HAS_Y_MODE: u8 = 1;
    const SMOOTH: u8 = 1 << 1;
    const FSC: u8 = 1 << 2;
    const DIP: u8 = 1 << 3;
    const MRL: u8 = 1 << 4;
    const MRL_SECONDARY: u8 = 1 << 5;
    /// Marks [`Self::ABSENT`], which no flag test reads as set.
    const ABSENT_BIT: u8 = 1 << 7;

    /// A position no intra leaf has published.
    pub(crate) const NONE: Self = Self {
        palette: None,
        joint_mode: IntraJointMode::DC,
        y_mode: IntraYMode::Dc,
        angle_delta_y: 0,
        bits: 0,
    };

    /// A position outside the tile.
    const ABSENT: Self = Self {
        bits: Self::ABSENT_BIT,
        ..Self::NONE
    };

    pub(crate) fn luma(
        joint_mode: IntraJointMode,
        y_mode: IntraYMode,
        angle_delta_y: i8,
        fsc_mode: u8,
        mrl: MrlSelection,
    ) -> Self {
        let flag = |set: bool, bit: u8| if set { bit } else { 0 };
        Self {
            palette: None,
            joint_mode,
            y_mode,
            angle_delta_y,
            bits: Self::HAS_Y_MODE
                | flag(y_mode.is_smooth(), Self::SMOOTH)
                | flag(fsc_mode != 0, Self::FSC)
                | flag(mrl.is_active(), Self::MRL)
                | flag(mrl.is_secondary(), Self::MRL_SECONDARY),
        }
    }

    #[must_use]
    pub(crate) const fn with_dip(mut self, use_dip: bool) -> Self {
        self.bits = self.bits & !Self::DIP | if use_dip { Self::DIP } else { 0 };
        self
    }

    /// An intra block copy leaf: § 5.20.5.3 `DC_PRED`, which an SDP
    /// chroma-part leaf reads as its collocated luma mode.
    pub(crate) const fn intrabc() -> Self {
        Self {
            bits: Self::HAS_Y_MODE,
            ..Self::NONE
        }
    }

    pub(crate) const fn y_mode_facts(self) -> Option<TileIntraYModeFacts> {
        if self.bits & Self::HAS_Y_MODE == 0 {
            return None;
        }
        Some(TileIntraYModeFacts {
            y_mode: self.y_mode,
            angle_delta_y: self.angle_delta_y,
        })
    }
}

/// The neighbour facts one luma leaf's mode syntax reads: the § 8.3.2
/// `get_joint_mode` probes, the first two `NPos` positions, and the above
/// and left positions of the § 7.11.2 smoothness.
#[derive(Clone, Copy)]
pub(crate) struct IntraNeighbours {
    joint: [IntraJointMode; 2],
    /// The OR and the AND of the flag bits at the first two `NPos` positions.
    npos_any: u8,
    npos_both: u8,
    above_left_bits: [u8; 2],
}

impl IntraNeighbours {
    pub(crate) const fn joint_modes(self) -> [IntraJointMode; 2] {
        self.joint
    }

    pub(crate) const fn y_mode_index_ctx(self) -> usize {
        self.joint[0].is_directional() as usize + self.joint[1].is_directional() as usize
    }

    const fn npos_sum(self, bit: u8) -> usize {
        (self.npos_any & bit != 0) as usize + (self.npos_both & bit != 0) as usize
    }

    pub(crate) const fn fsc_mode_ctx(self) -> usize {
        self.npos_sum(IntraFacts::FSC)
    }

    pub(crate) const fn use_dip_ctx(self) -> usize {
        self.npos_sum(IntraFacts::DIP)
    }

    pub(crate) const fn mrl_index_ctx(self) -> usize {
        self.npos_sum(IntraFacts::MRL)
    }

    pub(crate) const fn mrl_sec_index_ctx(self) -> usize {
        self.npos_sum(IntraFacts::MRL_SECONDARY)
    }

    /// `(above, left)` smoothness of the luma neighbours.
    pub(crate) const fn smoothness(self) -> (bool, bool) {
        (
            self.above_left_bits[0] & IntraFacts::SMOOTH != 0,
            self.above_left_bits[1] & IntraFacts::SMOOTH != 0,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraFactsState {
    /// The palette field indexes `palettes` from 1.
    grid: MiGrid<IntraFacts>,
    palettes: Vec<LumaPalette>,
    sb_size4: usize,
}

impl TileIntraFactsState {
    pub(crate) fn new_for_tile(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_size4: usize,
    ) -> Result<Self, TileIntraFactsStateError> {
        let grid = mi_grid_new_for_tile!(
            TileIntraFactsStateError,
            IntraFacts::NONE,
            row_range,
            col_range,
            sb_size4,
            require_nonzero(sb_size4, TileIntraFactsStateError::EmptySuperblockSize),
        )?;
        Ok(Self {
            grid,
            palettes: take_pooled_vec(0),
            sb_size4,
        })
    }

    /// Reads the four neighbour positions of the block at `(r, c)` once.
    #[allow(clippy::inline_always, reason = "measured: once per intra luma leaf")]
    #[inline(always)]
    pub(crate) fn neighbours(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> IntraNeighbours {
        let bottom = r.saturating_add(n4h.saturating_sub(1));
        let right = c.saturating_add(n4w.saturating_sub(1));
        let at = |row: Option<usize>, col: Option<usize>| {
            row.zip(col)
                .and_then(|(row, col)| self.grid.cell(row, col))
                .unwrap_or(IntraFacts::ABSENT)
        };
        let (left, above) = (c.checked_sub(1), r.checked_sub(1));
        let left_bottom = at(Some(bottom), left);
        let above_right = at(above, Some(right));
        let left_top = at(Some(r), left);
        let above_left = at(above, Some(c));
        let same_row = |row: usize| crate::tile::same_superblock_row(r, row, self.sb_size4);
        let above_in_row = above.is_some_and(same_row);
        let (mut npos_any, mut npos_both, mut len) = (0, 0, 0);
        for (facts, in_row) in [
            (left_bottom, same_row(bottom)),
            (above_right, above_in_row),
            (left_top, true),
            (above_left, above_in_row),
        ] {
            if in_row && facts.bits & IntraFacts::ABSENT_BIT == 0 && len < 2 {
                npos_both = npos_any & facts.bits;
                npos_any |= facts.bits;
                len += 1;
            }
        }
        IntraNeighbours {
            joint: [left_bottom.joint_mode, above_right.joint_mode],
            npos_any,
            npos_both,
            above_left_bits: [above_left.bits, left_top.bits],
        }
    }

    pub(crate) fn record_leaf(
        &mut self,
        (r, c): (usize, usize),
        extent: (usize, usize),
        mut facts: IntraFacts,
        palette: Option<LumaPalette>,
    ) {
        facts.palette = palette.and_then(|palette| {
            let index = NonZeroU32::new(u32::try_from(self.palettes.len().checked_add(1)?).ok()?)?;
            self.palettes.push(palette);
            Some(index)
        });
        self.grid.record_block((r, c), extent, facts);
    }

    pub(crate) fn y_mode_facts_at(&self, r: usize, c: usize) -> Option<TileIntraYModeFacts> {
        self.grid.cell(r, c).and_then(IntraFacts::y_mode_facts)
    }

    pub(crate) fn uses_fsc_at(&self, r: usize, c: usize) -> bool {
        self.grid
            .cell(r, c)
            .is_some_and(|facts| facts.bits & IntraFacts::FSC != 0)
    }

    pub(crate) fn palette_cache(&self, r: usize, c: usize) -> ([u16; 2 * PALETTE_MAX_SIZE], usize) {
        const MIN_SB_SIZE4: usize = 16;

        let above = if r != 0 && !r.is_multiple_of(MIN_SB_SIZE4) {
            self.palette_at(r - 1, c)
        } else {
            None
        };
        let left = c.checked_sub(1).and_then(|col| self.palette_at(r, col));
        let mut cache = [0u16; 2 * PALETTE_MAX_SIZE];
        let mut len = 0usize;
        let above_len = above.map_or(0, LumaPalette::size);
        let left_len = left.map_or(0, LumaPalette::size);
        let above_colors = above.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);
        let left_colors = left.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);
        for index in 0..above_len.max(left_len) {
            if index < above_len {
                push_palette_cache(&mut cache, &mut len, above_colors[index]);
            }
            if index < left_len {
                push_palette_cache(&mut cache, &mut len, left_colors[index]);
            }
        }
        (cache, len)
    }

    fn palette_at(&self, row: usize, col: usize) -> Option<LumaPalette> {
        let index = usize::try_from(self.grid.cell(row, col)?.palette?.get() - 1).ok()?;
        self.palettes.get(index).copied()
    }

    /// The palette list the grid indexes, for the encoder facade's rollback.
    pub(crate) fn palettes_mut(&mut self) -> &mut Vec<LumaPalette> {
        &mut self.palettes
    }

    /// Whether an access touched a row the window had already reused.
    pub(crate) fn window_violated(&self) -> bool {
        self.grid.window_violated()
    }

    /// The neighbour grid, for the encoder facade's rollback.
    pub(crate) fn grid_mut(&mut self) -> &mut MiGrid<IntraFacts> {
        &mut self.grid
    }

    pub(crate) fn recycle(self) {
        recycle_pooled_vec(self.grid.into_cells());
        recycle_pooled_vec(self.palettes);
    }
}

fn push_palette_cache(cache: &mut [u16; 2 * PALETTE_MAX_SIZE], len: &mut usize, value: u16) {
    if *len < cache.len() {
        cache[*len] = value;
        *len += 1;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileSegmentIdState {
    grid: MiGrid<u8>,
    predicted: MiGrid<u8>,
}

/// Returns both grids to the per-thread MI-grid pool. Unlike its frontier
/// siblings this state is merged into the frame map rather than handed back at
/// a single call site, so the buffers come home on drop.
impl Drop for TileSegmentIdState {
    fn drop(&mut self) {
        recycle_pooled_vec(core::mem::take(&mut self.grid.cells));
        recycle_pooled_vec(core::mem::take(&mut self.predicted.cells));
    }
}

impl TileSegmentIdState {
    pub(crate) fn new_for_tile(
        mi_rows: core::ops::Range<usize>,
        mi_cols: core::ops::Range<usize>,
    ) -> Result<Self, TileSegmentIdStateError> {
        let grid = mi_grid_new_for_tile!(
            TileSegmentIdStateError,
            0u8,
            mi_rows.clone(),
            mi_cols.clone(),
            SbRowWindow::WHOLE_TILE_SB_H4,
            Ok(())
        )?;
        let predicted = mi_grid_new_for_tile!(
            TileSegmentIdStateError,
            0u8,
            mi_rows,
            mi_cols,
            SbRowWindow::WHOLE_TILE_SB_H4,
            Ok(())
        )?;
        Ok(Self { grid, predicted })
    }

    /// With segmentation off every segment id is zero, so the grids hold no
    /// cells: reads fall outside them and records are dropped.
    pub(crate) fn disabled() -> Self {
        let grid = || MiGrid {
            origin_row: 0,
            origin_col: 0,
            rows: 0,
            cols: 0,
            window: SbRowWindow::new(0, SbRowWindow::WHOLE_TILE_SB_H4),
            readable: (0, 0, 0),
            default: 0,
            cells: Vec::new(),
        };
        Self {
            grid: grid(),
            predicted: grid(),
        }
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

    pub(crate) fn predicted_context(&self, r: usize, c: usize) -> usize {
        let above = r
            .checked_sub(1)
            .and_then(|r| self.predicted.cell(r, c))
            .unwrap_or(0);
        let left = c
            .checked_sub(1)
            .and_then(|c| self.predicted.cell(r, c))
            .unwrap_or(0);
        usize::from(above) + usize::from(left)
    }

    pub(crate) fn record_predicted(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        predicted: bool,
    ) {
        self.predicted
            .record_block((r, c), (n4w, n4h), u8::from(predicted));
    }

    pub(crate) fn predictor_and_ctx(
        &self,
        r: usize,
        c: usize,
        avail_u: bool,
        avail_l: bool,
    ) -> (u8, usize) {
        let cell = |rr: usize, cc: usize| self.cell(rr, cc).map_or(-1i16, i16::from);
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

/// The default map is empty: a frame with segmentation off saves only zero
/// segment ids, and an empty map loads as zeros (§ 6.17.2
/// `load_previous_segment_ids`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FrameSegmentIdMap {
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<u8>,
}

/// Returns the frame map to the per-thread MI-grid pool; the map outlives its
/// frame as a published entropy product, so it comes home on drop.
impl Drop for FrameSegmentIdMap {
    fn drop(&mut self) {
        recycle_pooled_vec(core::mem::take(&mut self.cells));
    }
}

impl FrameSegmentIdMap {
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileSegmentIdStateError> {
        let grid = mi_grid_new_for_tile!(
            TileSegmentIdStateError,
            0u8,
            0..mi_rows,
            0..mi_cols,
            SbRowWindow::WHOLE_TILE_SB_H4,
            Ok(())
        )?;
        Ok(Self {
            mi_rows,
            mi_cols,
            cells: grid.into_cells(),
        })
    }

    pub(crate) fn reset(
        &mut self,
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<(), TileSegmentIdStateError> {
        let cells =
            mi_rows
                .checked_mul(mi_cols)
                .ok_or(TileSegmentIdStateError::ArithmeticOverflow {
                    operation: "frame segment-id map size",
                    left: mi_rows,
                    right: mi_cols,
                })?;
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileSegmentIdStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        self.cells
            .try_reserve(cells.saturating_sub(self.cells.len()))
            .map_err(|source| TileSegmentIdStateError::Allocation { source })?;
        self.cells.clear();
        self.cells.resize(cells, 0);
        self.mi_rows = mi_rows;
        self.mi_cols = mi_cols;
        Ok(())
    }

    pub(crate) fn copy_from(&mut self, source: &Self) -> Result<(), TileSegmentIdStateError> {
        self.cells
            .try_reserve(source.cells.len().saturating_sub(self.cells.len()))
            .map_err(|source| TileSegmentIdStateError::Allocation { source })?;
        self.mi_rows = source.mi_rows;
        self.mi_cols = source.mi_cols;
        self.cells.clone_from(&source.cells);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fill(&mut self, value: u8) {
        self.cells.fill(value);
    }

    pub(crate) fn dimensions(&self) -> (usize, usize) {
        (self.mi_rows, self.mi_cols)
    }

    pub(crate) fn block_min(&self, r: usize, c: usize, n4w: usize, n4h: usize) -> u8 {
        if r >= self.mi_rows || c >= self.mi_cols {
            return 0;
        }
        let row_end = r.saturating_add(n4h).min(self.mi_rows);
        let col_end = c.saturating_add(n4w).min(self.mi_cols);
        (r..row_end)
            .flat_map(|row| {
                self.cells[row * self.mi_cols + c.min(self.mi_cols)..row * self.mi_cols + col_end]
                    .iter()
                    .copied()
            })
            .min()
            .unwrap_or(0)
    }

    pub(crate) fn merge_tile(&mut self, tile: &TileSegmentIdState) {
        let rows = tile
            .grid
            .rows
            .min(self.mi_rows.saturating_sub(tile.grid.origin_row));
        let cols = tile
            .grid
            .cols
            .min(self.mi_cols.saturating_sub(tile.grid.origin_col));
        for local_row in 0..rows {
            let source = local_row.saturating_mul(tile.grid.cols);
            let target = tile
                .grid
                .origin_row
                .saturating_add(local_row)
                .saturating_mul(self.mi_cols)
                .saturating_add(tile.grid.origin_col);
            let (Some(source), Some(target)) = (
                tile.grid.cells.get(source..source.saturating_add(cols)),
                self.cells.get_mut(target..target.saturating_add(cols)),
            ) else {
                continue;
            };
            // splot-copy-ok: publish one tile's segment-id rows into the frame map
            target.copy_from_slice(source);
        }
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
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size4: usize,
    ) -> Result<Self, TileUvCflStateError> {
        let grid = mi_grid_new_for_tile!(
            TileUvCflStateError,
            0,
            0..mi_rows,
            0..mi_cols,
            sb_size4,
            Ok(())
        )?;
        Ok(Self { grid })
    }

    pub(crate) fn with_origin(mut self, row: usize, col: usize) -> Self {
        self.grid = self.grid.with_origin(row, col);
        self
    }

    pub(crate) fn recycle(self) {
        recycle_pooled_vec(self.grid.into_cells());
    }

    /// Whether an access touched a row the window had already reused.
    pub(crate) fn window_violated(&self) -> bool {
        self.grid.window_violated()
    }

    /// The neighbour grid, for the encoder facade's rollback.
    pub(crate) fn grid_mut(&mut self) -> &mut MiGrid<u8> {
        &mut self.grid
    }

    pub(crate) fn is_cfl_ctx(&self, r: usize, c: usize, avail_u: bool, avail_l: bool) -> usize {
        let is_cfl = |available: bool, row: Option<usize>, col: Option<usize>| {
            available
                && row
                    .zip(col)
                    .and_then(|(row, col)| self.grid.cell(row, col))
                    .is_some_and(|is_cfl| is_cfl != 0)
        };
        usize::from(is_cfl(avail_u, r.checked_sub(1), Some(c)))
            + usize::from(is_cfl(avail_l, Some(r), c.checked_sub(1)))
    }

    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_cfl: bool,
    ) {
        self.grid
            .record_owned_block((r, c), (n4w, n4h), u8::from(is_cfl));
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
pub(crate) enum TileIntraFactsStateError {
    #[error("intra mode facts state requires non-empty MI dimensions, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("intra mode facts state requires non-empty superblock size")]
    EmptySuperblockSize,
    #[error("intra mode facts state arithmetic overflow in {operation}: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("intra mode facts state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileIntraYModeFacts {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
}

#[cfg(test)]
#[path = "intra_joint_modes_tests.rs"]
mod tests;
