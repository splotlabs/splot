// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary and the immediate
//! neighbour facts that ride on it.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::collections::TryReserveError;
use std::ops::Range;

use super::intra_joint_modes::{
    IntraFacts, IntraNeighbours, IsCflContext, LumaPalette, PALETTE_MAX_SIZE,
};
use super::partition_size::{BlockSize, PartitionSizeError};
use crate::support::reusable_scratch::{recycle_pooled_vec, take_pooled_vec};

const PLANE_COUNT: usize = 2;
const LUMA_PLANE: usize = 0;
const CHROMA_PLANE: usize = 1;
const CLEAR_PARTITION_CONTEXT: u8 = 0;
const PARTITION_CONTEXT_ABOVE: [u8; 29] = [
    63, 63, 62, 62, 62, 60, 60, 60, 56, 56, 56, 48, 48, 48, 32, 32, 32, 0, 0, 63, 60, 62, 56, 60,
    48, 63, 56, 62, 48,
];
const PARTITION_CONTEXT_LEFT: [u8; 29] = [
    63, 62, 63, 62, 60, 62, 60, 56, 60, 56, 48, 56, 48, 32, 48, 32, 0, 32, 0, 60, 63, 56, 62, 48,
    60, 56, 63, 48, 62,
];

/// Bit 6 of a luma entry: its leaf recorded [`IntraFacts`] (an intra or intra
/// block copy leaf), which the facts lines hold at the same position.
const INTRA_FACTS: u8 = 1 << 6;
/// Bits 6 and 7 of a chroma entry: § 5.20.5.3 `UVCfls` and `UVSmooth`.
pub(crate) const UV_CFL: u8 = 1 << 6;
pub(crate) const UV_SMOOTH: u8 = 1 << 7;

/// The § 5.20.4.1 `LeftMiSizes` / `AboveMiSizes` lines of a tile, stored as
/// partition-context bit masks (bit `k` set when the neighbour's log2 extent
/// is at most `k`, in 4x4 units), and the other immediate neighbour facts
/// the mode syntax reads.
///
/// The § 8.3.2 square-split context reads `MiSizes[ 0 ][ r - 1 ][ c ]` and
/// `MiSizes[ 0 ][ r ][ c - 1 ]`. The partition tree decodes the blocks
/// covering one column top to bottom, and the blocks covering one row left
/// to right, so those are the last blocks written to the lines. The same
/// holds for every other neighbour the mode syntax reads: the § 8.3.2
/// `get_joint_mode` / `NPos` probes and palette cache, the § 8.3.2 `is_cfl`
/// context and the § 7.13.2.15/16 smoothness all sit on the column left of
/// a block or the row above it. So a leaf writes them with its MiSizes: the
/// chroma entries carry `UVCfls` and `UVSmooth` (§ 5.20.5.3 writes both for
/// every `HasChroma` leaf), and an intra luma entry flags the [`IntraFacts`]
/// its leaf writes to the facts lines.
///
/// Every read and write stays inside the current superblock row, which each
/// row start clears, so the left lines hold one superblock row indexed by the
/// row modulo `SB_ROW_SPAN`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    origin_row: usize,
    origin_col: usize,
    rows: usize,
    above_len: usize,
    /// The tile's rows and columns inside the frame; facts beyond read as
    /// outside the tile.
    tile_rows: usize,
    tile_cols: usize,
    sb_size4: usize,
    left: [[u8; SB_ROW_SPAN]; PLANE_COUNT],
    /// `[above L | above C]`.
    above: Vec<u8>,
    left_facts: [IntraFacts; SB_ROW_SPAN],
    above_facts: Vec<IntraFacts>,
    /// The palette field of the facts indexes `palettes` from 1.
    palettes: Vec<LumaPalette>,
}

/// The tallest superblock in 4x4 units; every superblock height divides it.
const SB_ROW_SPAN: usize = 64;

impl TileMiSizeState {
    pub(crate) fn allocation(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: BlockSize,
    ) -> Result<TileMiSizeStateAllocation, TileMiSizeStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileMiSizeStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        let padded_rows = padded_dimension("rows", mi_rows, sb_size.num_4x4_high()?)?;
        let padded_cols = padded_dimension("cols", mi_cols, sb_size.num_4x4_wide()?)?;
        let entry_count = checked_mul_usize("padded_cols * planes", padded_cols, PLANE_COUNT)?;
        Ok(TileMiSizeStateAllocation {
            padded_rows,
            padded_cols,
            entry_count,
        })
    }

    pub(crate) fn new_for_tile(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_size: BlockSize,
    ) -> Result<Self, TileMiSizeStateError> {
        let mi_rows = row_range.end.saturating_sub(row_range.start);
        let mi_cols = col_range.end.saturating_sub(col_range.start);
        let allocation = Self::allocation(mi_rows, mi_cols, sb_size)?;
        let mut above = take_pooled_vec::<u8>(allocation.entry_count);
        above.try_reserve_exact(allocation.entry_count)?;
        above.resize(allocation.entry_count, CLEAR_PARTITION_CONTEXT);
        let mut above_facts = take_pooled_vec::<IntraFacts>(allocation.padded_cols);
        above_facts.try_reserve_exact(allocation.padded_cols)?;
        above_facts.resize(allocation.padded_cols, IntraFacts::NONE);
        Ok(Self {
            origin_row: row_range.start,
            origin_col: col_range.start,
            rows: allocation.padded_rows,
            above_len: allocation.padded_cols,
            tile_rows: mi_rows,
            tile_cols: mi_cols,
            sb_size4: sb_size.num_4x4_high()?,
            left: [[CLEAR_PARTITION_CONTEXT; SB_ROW_SPAN]; PLANE_COUNT],
            above,
            left_facts: [IntraFacts::NONE; SB_ROW_SPAN],
            above_facts,
            palettes: take_pooled_vec(0),
        })
    }

    const fn above_base(&self, plane: usize) -> usize {
        plane * self.above_len
    }

    #[cfg(test)]
    fn left_at(&self, plane: usize, row: usize) -> u8 {
        self.left[plane][row % SB_ROW_SPAN]
    }

    #[cfg(test)]
    fn above_plane(&self, plane: usize) -> &[u8] {
        let base = self.above_base(plane);
        &self.above[base..base + self.above_len]
    }

    /// Starts a superblock row: clears the left context lines.
    pub(crate) fn enter_sb_row(&mut self, _sb_row: usize) {
        self.left = [[CLEAR_PARTITION_CONTEXT; SB_ROW_SPAN]; PLANE_COUNT];
    }

    /// The tile-relative `(row, col)` of `(r, c)`, which a reader of a block
    /// inside the tile gets back unchanged.
    #[allow(clippy::inline_always, reason = "measured: per-leaf edge reads")]
    #[inline(always)]
    const fn local(&self, r: usize, c: usize) -> (usize, usize) {
        (
            r.wrapping_sub(self.origin_row),
            c.wrapping_sub(self.origin_col),
        )
    }

    /// The facts of the luma leaf on line entry `index` of `line`, `NONE`
    /// for an inter leaf.
    #[allow(clippy::inline_always, reason = "measured: per-leaf edge reads")]
    #[inline(always)]
    fn facts_at(entry: Option<&u8>, facts: Option<&IntraFacts>) -> IntraFacts {
        match (entry, facts) {
            (Some(entry), Some(&facts)) if entry & INTRA_FACTS != 0 => facts,
            _ => IntraFacts::NONE,
        }
    }

    /// The four § 8.3.2 `get_joint_mode` / `NPos` probes of the luma block at
    /// `(r, c)`: left-bottom, above-right, left-top and above-left, a
    /// position outside the tile reading as absent.
    #[allow(clippy::inline_always, reason = "measured: once per intra luma leaf")]
    #[inline(always)]
    pub(crate) fn intra_neighbours(
        &self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
    ) -> IntraNeighbours {
        let (row, col) = self.local(r, c);
        let has_up = row.wrapping_sub(1) < self.tile_rows;
        let has_left = col.wrapping_sub(1) < self.tile_cols;
        let bottom = r + n4h.saturating_sub(1);
        let right = col + n4w.saturating_sub(1);
        let left = |r: usize| {
            let i = r % SB_ROW_SPAN;
            Self::facts_at(self.left[LUMA_PLANE].get(i), self.left_facts.get(i))
        };
        let above = |col: usize| Self::facts_at(self.above.get(col), self.above_facts.get(col));
        let left_bottom = if has_left && row + n4h.saturating_sub(1) < self.tile_rows {
            left(bottom)
        } else {
            IntraFacts::ABSENT
        };
        let above_right = if has_up && right < self.tile_cols {
            above(right)
        } else {
            IntraFacts::ABSENT
        };
        let left_top = if has_left {
            left(r)
        } else {
            IntraFacts::ABSENT
        };
        let above_left = if has_up {
            above(col)
        } else {
            IntraFacts::ABSENT
        };
        let same_row = |row: usize| crate::tile::same_superblock_row(r, row, self.sb_size4);
        IntraNeighbours::from_probes(
            [left_bottom, above_right, left_top, above_left],
            [same_row(bottom), r.checked_sub(1).is_some_and(same_row)],
        )
    }

    /// The chroma entries above and left of the chroma reference block at
    /// `(r, c)`; a line outside the tile reads clear.
    fn uv_edges(&self, r: usize, c: usize) -> (u8, u8) {
        let (_, col) = self.local(r, c);
        (
            self.above
                .get(self.above_base(CHROMA_PLANE) + col)
                .copied()
                .unwrap_or(0),
            self.left[CHROMA_PLANE][r % SB_ROW_SPAN],
        )
    }

    /// The § 8.3.2 `is_cfl` context of the chroma reference block at `(r, c)`.
    pub(crate) fn uv_cfl_ctx(&self, r: usize, c: usize) -> IsCflContext {
        let (above, left) = self.uv_edges(r, c);
        IsCflContext::new(usize::from(above & UV_CFL != 0) + usize::from(left & UV_CFL != 0))
    }

    /// `(above, left)` § 7.13.2.15/16 `UVSmooth` of the chroma reference block
    /// at `(r, c)`.
    pub(crate) fn uv_smoothness(&self, r: usize, c: usize) -> (bool, bool) {
        let (above, left) = self.uv_edges(r, c);
        (above & UV_SMOOTH != 0, left & UV_SMOOTH != 0)
    }

    /// The § 5.20.5.8 palette cache of the luma block at `(r, c)`.
    pub(crate) fn palette_cache(&self, r: usize, c: usize) -> ([u16; 2 * PALETTE_MAX_SIZE], usize) {
        const MIN_SB_SIZE4: usize = 16;

        let (row, col) = self.local(r, c);
        let palette = |facts: IntraFacts| {
            let index = usize::try_from(facts.palette()?.get() - 1).ok()?;
            self.palettes.get(index).copied()
        };
        let above = (row.wrapping_sub(1) < self.tile_rows && !r.is_multiple_of(MIN_SB_SIZE4))
            .then(|| Self::facts_at(self.above.get(col), self.above_facts.get(col)))
            .and_then(palette);
        let left = (col.wrapping_sub(1) < self.tile_cols)
            .then(|| {
                let i = r % SB_ROW_SPAN;
                Self::facts_at(self.left[LUMA_PLANE].get(i), self.left_facts.get(i))
            })
            .and_then(palette);
        let mut cache = [0u16; 2 * PALETTE_MAX_SIZE];
        let mut len = 0usize;
        let above_len = above.map_or(0, LumaPalette::size);
        let left_len = left.map_or(0, LumaPalette::size);
        let above_colors = above.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);
        let left_colors = left.map_or([0; PALETTE_MAX_SIZE], LumaPalette::colors);
        for index in 0..above_len.max(left_len) {
            for (colors, colors_len) in [(above_colors, above_len), (left_colors, left_len)] {
                if index < colors_len && len < cache.len() {
                    cache[len] = colors[index];
                    len += 1;
                }
            }
        }
        (cache, len)
    }

    /// Calls `visit` with both planes' entries and `visit_facts` with the
    /// facts over tile rows `rows` x columns `cols`, for the encoder facade's
    /// rollback.
    pub(crate) fn visit_region(
        &mut self,
        rows: Range<usize>,
        cols: &Range<usize>,
        mut visit: impl FnMut(&mut [u8]),
        mut visit_facts: impl FnMut(&mut [IntraFacts]),
    ) {
        let local = |range: &Range<usize>, origin: usize, len: usize| {
            range.start.saturating_sub(origin).min(len)..range.end.saturating_sub(origin).min(len)
        };
        let (r, c) = (
            local(&rows, self.origin_row, self.rows),
            local(cols, self.origin_col, self.above_len),
        );
        let start = (self.origin_row + r.start) % SB_ROW_SPAN;
        let r = start..(start + r.len()).min(SB_ROW_SPAN);
        for plane in [LUMA_PLANE, CHROMA_PLANE] {
            let above = self.above_base(plane);
            visit(&mut self.left[plane][r.clone()]);
            visit(&mut self.above[above + c.start..above + c.end]);
        }
        visit_facts(&mut self.left_facts[r]);
        visit_facts(&mut self.above_facts[c]);
    }

    /// The palette list the facts index, for the encoder facade's rollback.
    pub(crate) fn palettes_mut(&mut self) -> &mut Vec<LumaPalette> {
        &mut self.palettes
    }

    /// Writes a luma leaf's MiSizes and, for an intra or intra block copy
    /// leaf, its `facts` with `palette`.
    #[inline]
    pub(crate) fn update_luma_block(
        &mut self,
        r: usize,
        c: usize,
        mi_size: BlockSize,
        facts: Option<IntraFacts>,
        palette: Option<LumaPalette>,
    ) -> Result<(), TileMiSizeStateError> {
        let flag = if facts.is_some() { INTRA_FACTS } else { 0 };
        let (left, above) = self.update_plane_block(LUMA_PLANE, r, c, mi_size, flag)?;
        if let Some(facts) = facts {
            let index = palette.and_then(|palette| {
                let index = u32::try_from(self.palettes.len().checked_add(1)?).ok()?;
                self.palettes.push(palette);
                core::num::NonZeroU32::new(index)
            });
            let facts = facts.with_palette(index);
            if let (Some(left), Some(above)) = (
                self.left_facts.get_mut(left),
                self.above_facts.get_mut(above),
            ) {
                crate::support::fill_mi_run(left, facts);
                crate::support::fill_mi_run(above, facts);
            }
        }
        Ok(())
    }

    /// Writes a chroma reference block's MiSizes with its `UVCfls` /
    /// `UVSmooth` bits `uv`.
    #[inline]
    pub(crate) fn update_chroma_block(
        &mut self,
        chroma_mi_row: usize,
        chroma_mi_col: usize,
        chroma_mi_size: BlockSize,
        uv: u8,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(
            CHROMA_PLANE,
            chroma_mi_row,
            chroma_mi_col,
            chroma_mi_size,
            uv,
        )
        .map(drop)
    }

    /// The § 8.3.2 partition-context entries of the block at tile position
    /// (`r`, `c`): plane `plane` lines at `r`, `r + half_h`, `c` and
    /// `c + half_w`, and the luma lines at `r` and `c`.
    #[inline]
    pub(crate) fn partition_neighbours(
        &self,
        plane: usize,
        r: usize,
        c: usize,
        half_h: usize,
        half_w: usize,
    ) -> Option<PartitionNeighbours> {
        let local_r = r.checked_sub(self.origin_row)?;
        let local_c = c.checked_sub(self.origin_col)?;
        if plane >= PLANE_COUNT
            || local_r + half_h >= self.rows
            || local_c + half_w >= self.above_len
        {
            return None;
        }
        let (left, luma_left) = (&self.left[plane & 1], &self.left[LUMA_PLANE]);
        let row = r % SB_ROW_SPAN;
        let base = self.above_base(plane) + local_c;
        let above = self.above.get(base..=base + half_w)?;
        Some(PartitionNeighbours {
            left: [left[row], left[(row + half_h) % SB_ROW_SPAN]],
            above: [*above.first()?, *above.last()?],
            luma_left: luma_left[row],
            luma_above: *self.above.get(self.above_base(LUMA_PLANE) + local_c)?,
        })
    }

    /// Writes one plane's entries of a block, `flags` beside its MiSizes,
    /// and returns the left and above line ranges it covered.
    #[allow(clippy::inline_always, reason = "measured: two calls per decoded leaf")]
    #[inline(always)]
    fn update_plane_block(
        &mut self,
        plane: usize,
        r: usize,
        c: usize,
        mi_size: BlockSize,
        flags: u8,
    ) -> Result<(Range<usize>, Range<usize>), TileMiSizeStateError> {
        let index = mi_size.index();
        let (Some(local_r), Some(local_c), Some(&left), Some(&above)) = (
            r.checked_sub(self.origin_row),
            c.checked_sub(self.origin_col),
            PARTITION_CONTEXT_LEFT.get(index),
            PARTITION_CONTEXT_ABOVE.get(index),
        ) else {
            return Err(TileMiSizeStateError::BlockOutOfBounds { plane, r, c });
        };
        let rows_end = local_r.saturating_add(mi_size.num_4x4_high()?);
        let cols_end = local_c.saturating_add(mi_size.num_4x4_wide()?);
        if rows_end > self.rows || cols_end > self.above_len {
            return Err(TileMiSizeStateError::BlockOutOfBounds { plane, r, c });
        }
        let row = r % SB_ROW_SPAN;
        let left_run = row..row + (rows_end - local_r);
        if let Some(left_run) = self.left[plane & 1].get_mut(left_run.clone()) {
            crate::support::fill_mi_run(left_run, left | flags);
        }
        let above_base = self.above_base(plane);
        if let Some(above_run) = self
            .above
            .get_mut(above_base + local_c..above_base + cols_end)
        {
            crate::support::fill_mi_run(above_run, above | flags);
        }
        Ok((left_run, local_c..cols_end))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionNeighbours {
    pub(crate) left: [u8; 2],
    pub(crate) above: [u8; 2],
    pub(crate) luma_left: u8,
    pub(crate) luma_above: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeStateAllocation {
    padded_rows: usize,
    padded_cols: usize,
    entry_count: usize,
}

impl TileMiSizeStateAllocation {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn padded_rows(self) -> usize {
        self.padded_rows
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn padded_cols(self) -> usize {
        self.padded_cols
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn entry_count(self) -> usize {
        self.entry_count
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileMiSizeStateError {
    #[error("MI-size state dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("{operation} overflow: left {left}, right {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("MI-size state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("MI-size state block-size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    #[error("MI-size state plane {plane} block at ({r},{c}) leaves the tile's padded extent")]
    BlockOutOfBounds { plane: usize, r: usize, c: usize },
    #[error(
        "MI-size state padded {axis} dimension overflow: dimension {dimension}, superblock span {sb_span}"
    )]
    PaddedDimensionOverflow {
        axis: &'static str,
        dimension: usize,
        sb_span: usize,
    },
}

fn padded_dimension(
    axis: &'static str,
    dimension: usize,
    sb_span: usize,
) -> Result<usize, TileMiSizeStateError> {
    let sb_pad = sb_span
        .checked_sub(1)
        .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
            axis,
            dimension,
            sb_span,
        })?;
    let adjusted =
        dimension
            .checked_add(sb_pad)
            .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
                axis,
                dimension,
                sb_span,
            })?;
    let sb_count = adjusted / sb_span;
    sb_count
        .checked_mul(sb_span)
        .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
            axis,
            dimension,
            sb_span,
        })
}

fn checked_mul_usize(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TileMiSizeStateError> {
    left.checked_mul(right)
        .ok_or(TileMiSizeStateError::ArithmeticOverflow {
            operation,
            left,
            right,
        })
}

/// Returns the store to the per-thread MI-grid pool; the frontier cursor
/// drops this state at the end of every tile.
impl Drop for TileMiSizeState {
    fn drop(&mut self) {
        recycle_pooled_vec(core::mem::take(&mut self.above));
        recycle_pooled_vec(core::mem::take(&mut self.above_facts));
        recycle_pooled_vec(core::mem::take(&mut self.palettes));
    }
}

#[cfg(test)]
fn partition_context_above(mi_size_index: usize) -> Option<u8> {
    PARTITION_CONTEXT_ABOVE.get(mi_size_index).copied()
}

#[cfg(test)]
fn partition_context_left(mi_size_index: usize) -> Option<u8> {
    PARTITION_CONTEXT_LEFT.get(mi_size_index).copied()
}

#[cfg(test)]
#[path = "mi_size_state_tests.rs"]
mod tests;
