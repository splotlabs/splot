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
use crate::bitstream::tile_payload::BlockSymbolTraceReadError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GdfState {
    active: bool,
    block_size: usize,
    sb_size4: usize,
    sb_per_gdf: usize,
    row_start: usize,
    col_start: usize,
    grid_rows: usize,
    grid_cols: usize,
    values: Vec<u8>,
}

impl GdfState {
    pub(crate) fn for_tile(
        &self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        _tile_offset: ByteOffset,
    ) -> Result<Self> {
        if !self.active {
            return Ok(Self::inactive());
        }
        let unit_mi = self.block_size / 4;
        let row_start = mi_rows.start / unit_mi;
        let col_start = mi_cols.start / unit_mi;
        let row_end = mi_rows.end.div_ceil(unit_mi).min(self.grid_rows);
        let col_end = mi_cols.end.div_ceil(unit_mi).min(self.grid_cols);
        let grid_rows = row_end.saturating_sub(row_start);
        let grid_cols = col_end.saturating_sub(col_start);
        let len = grid_rows
            .checked_mul(grid_cols)
            .ok_or_else(gdf_state_error)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| gdf_allocation_error())?;
        values.resize(len, 2);
        Ok(Self {
            active: self.active,
            block_size: self.block_size,
            sb_size4: self.sb_size4,
            sb_per_gdf: self.sb_per_gdf,
            row_start,
            col_start,
            grid_rows,
            grid_cols,
            values,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        _tile_offset: ByteOffset,
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
        let sb_size4 = sb_size4(sb_size);
        let sb_width = sb_size4.checked_mul(4).ok_or_else(gdf_state_error)?;
        let sb_per_gdf = block_size
            .checked_div(sb_width)
            .filter(|&value| value != 0)
            .ok_or_else(gdf_state_error)?;
        let unit_mi = block_size
            .checked_div(4)
            .filter(|&value| value != 0)
            .ok_or_else(gdf_state_error)?;
        let grid_rows = mi_rows.div_ceil(unit_mi);
        let grid_cols = mi_cols.div_ceil(unit_mi);
        let cells = grid_rows
            .checked_mul(grid_cols)
            .ok_or_else(gdf_state_error)?;
        Ok(Self {
            active: true,
            block_size,
            sb_size4,
            sb_per_gdf,
            row_start: 0,
            col_start: 0,
            grid_rows,
            grid_cols,
            values: vec![2; cells],
        })
    }

    fn inactive() -> Self {
        Self {
            active: false,
            block_size: 0,
            sb_size4: 0,
            sb_per_gdf: 0,
            row_start: 0,
            col_start: 0,
            grid_rows: 0,
            grid_cols: 0,
            values: Vec::new(),
        }
    }

    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.active || frontier.is_chroma_part() {
            return Ok(());
        }
        if !frontier.r.is_multiple_of(self.sb_size4) || !frontier.c.is_multiple_of(self.sb_size4) {
            return Ok(());
        }
        let sb_row = frontier.r / self.sb_size4;
        let sb_col = frontier.c / self.sb_size4;
        if !sb_row.is_multiple_of(self.sb_per_gdf) || !sb_col.is_multiple_of(self.sb_per_gdf) {
            return Ok(());
        }
        let unit_row = sb_row / self.sb_per_gdf;
        let unit_col = sb_col / self.sb_per_gdf;
        let index = self.index(unit_row, unit_col).ok_or_else(gdf_state_error)?;
        self.values[index] =
            read_use_gdf(work_unit.cdf_mut().tile_cdfs_mut(), symbols, tile_offset)?;
        Ok(())
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        crate::tile::local_grid_index(
            row,
            col,
            self.row_start,
            self.col_start,
            self.grid_rows,
            self.grid_cols,
        )
    }

    pub(crate) fn merge_tile(
        &mut self,
        tile: &Self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        _tile_offset: ByteOffset,
    ) -> Result<()> {
        if self.active != tile.active
            || self.block_size != tile.block_size
            || self.sb_size4 != tile.sb_size4
            || self.sb_per_gdf != tile.sb_per_gdf
            || self.row_start != 0
            || self.col_start != 0
        {
            return Err(gdf_state_error());
        }
        if !self.active {
            return Ok(());
        }
        let unit_mi = self.block_size / 4;
        let row_end = mi_rows.end.div_ceil(unit_mi).min(self.grid_rows);
        let col_end = mi_cols.end.div_ceil(unit_mi).min(self.grid_cols);
        let expected_row_start = mi_rows.start / unit_mi;
        let expected_col_start = mi_cols.start / unit_mi;
        if tile.row_start != expected_row_start
            || tile.col_start != expected_col_start
            || tile.grid_rows != row_end.saturating_sub(expected_row_start)
            || tile.grid_cols != col_end.saturating_sub(expected_col_start)
        {
            return Err(gdf_state_error());
        }
        for row in mi_rows.start / unit_mi..row_end {
            for col in mi_cols.start / unit_mi..col_end {
                let index = self.index(row, col).ok_or_else(gdf_state_error)?;
                let tile_index = tile.index(row, col).ok_or_else(gdf_state_error)?;
                self.values[index] = tile.values[tile_index];
            }
        }
        Ok(())
    }

    pub(crate) fn into_grid(self, _tile_offset: ByteOffset) -> Result<Option<GdfBlockGrid>> {
        if !self.active {
            return Ok(None);
        }
        if self.row_start != 0 || self.col_start != 0 {
            return Err(gdf_state_error());
        }
        GdfBlockGrid::new(self.block_size, self.grid_rows, self.grid_cols, self.values)
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

const fn sb_size4(sb_size: SuperblockSize) -> usize {
    match sb_size {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 => 32,
        SuperblockSize::Block256x256 => 64,
    }
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
