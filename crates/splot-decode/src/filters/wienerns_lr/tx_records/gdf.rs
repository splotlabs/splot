// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{FrameHeaderCore, GdfGeometry, gdf_block_size};
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use std::ops::Range;

use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector, TileCdfSubset,
};
use crate::error::Result;
use crate::filters::gdf::GdfBlockGrid;

use super::super::selectable_symbol_read_error;
use super::UnitWindow;
use crate::bitstream::tile_payload::BlockSymbolTraceReadError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GdfState {
    active: bool,
    block_size: usize,
    sb_size4: usize,
    sb_per_gdf: usize,
    window: UnitWindow,
    values: Vec<u8>,
}

impl GdfState {
    /// Lays out one tile's state, on the grid a spent tile state left when
    /// `spare` carries one.
    pub(crate) fn for_tile(
        &self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        spare: Option<Self>,
    ) -> Result<Self> {
        if !self.active {
            let mut inactive = Self::inactive();
            inactive.values = spare.map(|spare| spare.values).unwrap_or_default();
            inactive.values.clear();
            return Ok(inactive);
        }
        let window = self.window.tile(&mi_rows, &mi_cols, self.block_size / 4);
        let len = window.len().ok_or_else(gdf_state_error)?;
        let mut values = spare.map(|spare| spare.values).unwrap_or_default();
        values.clear();
        values
            .try_reserve_exact(len)
            .map_err(|_| gdf_allocation_error())?;
        values.resize(len, 2);
        Ok(Self {
            active: self.active,
            block_size: self.block_size,
            sb_size4: self.sb_size4,
            sb_per_gdf: self.sb_per_gdf,
            window,
            values,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
    ) -> Result<Self> {
        let per_block = core
            .gdf_params
            .as_ref()
            .is_some_and(|gdf| gdf.gdf_frame_enable && gdf.gdf_per_block == Some(true));
        if !per_block {
            return Ok(Self::inactive());
        }
        let filter = sequence.filter.as_ref().ok_or_else(gdf_state_error)?;
        let partition = sequence.partition.as_ref().ok_or_else(gdf_state_error)?;
        let frame_is_intra = core.frame_is_intra.ok_or_else(gdf_state_error)?;
        let sb_size = match (frame_is_intra, partition.seq_sb_size()) {
            (true, SuperblockSize::Block256x256) => SuperblockSize::Block128x128,
            (_, sb_size) => sb_size,
        };
        let tile = core.tile_info.as_ref().ok_or_else(gdf_state_error)?;
        let geometry = GdfGeometry {
            sb_size,
            mi_cols: u32::try_from(mi_cols).map_err(|_| gdf_state_error())?,
            mi_rows: u32::try_from(mi_rows).map_err(|_| gdf_state_error())?,
            tile_cols: tile.tile_cols,
            tile_rows: tile.tile_rows,
            mi_col_starts: &tile.mi_col_starts,
            mi_row_starts: &tile.mi_row_starts,
        };
        let block_size = usize::try_from(gdf_block_size(filter.gdf_unit_matches_sb_size, geometry))
            .map_err(|_| gdf_state_error())?;
        let sb_size4 = crate::tile::frame_superblock_h4(partition.seq_sb_size(), frame_is_intra);
        let sb_width = sb_size4.checked_mul(4).ok_or_else(gdf_state_error)?;
        let sb_per_gdf = block_size
            .checked_div(sb_width)
            .filter(|&value| value != 0)
            .ok_or_else(gdf_state_error)?;
        let unit_mi = block_size
            .checked_div(4)
            .filter(|&value| value != 0)
            .ok_or_else(gdf_state_error)?;
        let window = UnitWindow::frame(mi_rows.div_ceil(unit_mi), mi_cols.div_ceil(unit_mi));
        let cells = window.len().ok_or_else(gdf_state_error)?;
        Ok(Self {
            active: true,
            block_size,
            sb_size4,
            sb_per_gdf,
            window,
            values: vec![2; cells],
        })
    }

    fn inactive() -> Self {
        Self {
            active: false,
            block_size: 0,
            sb_size4: 0,
            sb_per_gdf: 0,
            window: UnitWindow::default(),
            values: Vec::new(),
        }
    }

    #[allow(clippy::inline_always, reason = "measured: a per-leaf guard")]
    #[inline(always)]
    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        match self.unit_for(frontier) {
            Some(unit) => self.read_unit(work_unit, symbols, unit, tile_offset),
            None => Ok(()),
        }
    }

    #[inline(never)]
    fn read_unit(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        (unit_row, unit_col): (usize, usize),
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let index = self
            .window
            .index(unit_row, unit_col)
            .ok_or_else(gdf_state_error)?;
        self.values[index] =
            read_use_gdf(work_unit.cdf_mut().tile_cdfs_mut(), symbols, tile_offset)?;
        Ok(())
    }

    /// The `(row, col)` of the GDF unit whose `use_gdf` the block at
    /// `frontier` codes (§ 5.20.10.3), if any.
    #[allow(clippy::inline_always, reason = "measured: a per-leaf guard")]
    #[inline(always)]
    pub(crate) fn unit_for(&self, frontier: &DecodeBlockFrontier) -> Option<(usize, usize)> {
        let sb_mask = self.sb_size4.wrapping_sub(1);
        if !self.active || frontier.is_chroma_part() || (frontier.r | frontier.c) & sb_mask != 0 {
            return None;
        }
        let (sb_row, sb_col) = crate::tile::superblock_of(frontier.r, frontier.c, self.sb_size4);
        if !sb_row.is_multiple_of(self.sb_per_gdf) || !sb_col.is_multiple_of(self.sb_per_gdf) {
            return None;
        }
        Some((sb_row / self.sb_per_gdf, sb_col / self.sb_per_gdf))
    }

    /// The units of this tile state in raster order, which is their coding order.
    pub(crate) fn units(&self) -> impl Iterator<Item = (usize, usize)> {
        self.window.units()
    }

    /// `GdfBlkSize`, `0` when no block codes `use_gdf`.
    pub(crate) const fn block_size(&self) -> usize {
        self.block_size
    }

    pub(crate) fn merge_tile(
        &mut self,
        tile: &Self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
    ) -> Result<()> {
        if self.active != tile.active
            || self.block_size != tile.block_size
            || self.sb_size4 != tile.sb_size4
            || self.sb_per_gdf != tile.sb_per_gdf
            || !self.window.is_frame()
        {
            return Err(gdf_state_error());
        }
        if !self.active {
            return Ok(());
        }
        if tile.window != self.window.tile(&mi_rows, &mi_cols, self.block_size / 4) {
            return Err(gdf_state_error());
        }
        self.window
            .merge(tile.window, &tile.values, &mut self.values)
            .ok_or_else(gdf_state_error)
    }

    pub(crate) fn into_grid(self) -> Result<Option<GdfBlockGrid>> {
        if !self.active {
            return Ok(None);
        }
        if !self.window.is_frame() {
            return Err(gdf_state_error());
        }
        GdfBlockGrid::new(
            self.block_size,
            self.window.rows,
            self.window.cols,
            self.values,
        )
        .map(Some)
        .map_err(|()| gdf_state_error())
    }
}

fn read_use_gdf(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let value = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseGdf, symbols)
        .map_err(|error| match error {
            BlockSymbolTraceReadError::Cdf(_) => gdf_state_error(),
            BlockSymbolTraceReadError::Symbol(_) => {
                selectable_symbol_read_error(tile_offset, "5.20.10.3")
            }
        })?
        .get();
    if value > 1 {
        return Err(gdf_state_error());
    }
    Ok(value)
}

fn gdf_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidGdfFilterState.into()
}

fn gdf_allocation_error() -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane: splot_recon::PlaneId::Y,
        context: "GDF block grid",
    }
    .into()
}

#[cfg(test)]
#[path = "gdf_tests.rs"]
mod tests;
