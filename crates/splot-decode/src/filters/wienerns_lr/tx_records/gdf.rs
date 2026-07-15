// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{FrameHeaderCore, GdfGeometry, gdf_block_size};
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector, TileCdfSubset,
};
use crate::error::Result;
use crate::filters::gdf::GdfBlockGrid;

use super::super::wienerns_lr_selectable_transform_record_error_reason;

const GDF_GRID_REASON: &str = "unsupported_wienerns_lr_selectable_transform_records_gdf_grid";
const GDF_SYMBOL_REASON: &str = "unsupported_wienerns_lr_selectable_transform_records_gdf_symbol";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GdfState {
    active: bool,
    block_size: usize,
    sb_size4: usize,
    sb_per_gdf: usize,
    grid_rows: usize,
    grid_cols: usize,
    values: Vec<u8>,
}

impl GdfState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let per_block = core
            .gdf_params
            .as_ref()
            .is_some_and(|gdf| gdf.gdf_frame_enable && gdf.gdf_per_block == Some(true));
        if !per_block {
            return Ok(Self::inactive());
        }
        let filter = sequence
            .filter
            .as_ref()
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let partition = sequence
            .partition
            .as_ref()
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let frame_is_intra = core
            .frame_is_intra
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let sb_size = match (frame_is_intra, partition.seq_sb_size()) {
            (true, SuperblockSize::Block256x256) => SuperblockSize::Block128x128,
            (_, sb_size) => sb_size,
        };
        let tile = core
            .tile_info
            .as_ref()
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let geometry = GdfGeometry {
            sb_size,
            mi_cols: u32::try_from(mi_cols).map_err(|_| gdf_error(tile_offset, GDF_GRID_REASON))?,
            mi_rows: u32::try_from(mi_rows).map_err(|_| gdf_error(tile_offset, GDF_GRID_REASON))?,
            tile_cols: tile.tile_cols,
            tile_rows: tile.tile_rows,
            mi_col_starts: &tile.mi_col_starts,
            mi_row_starts: &tile.mi_row_starts,
        };
        let block_size = usize::try_from(gdf_block_size(filter.gdf_unit_matches_sb_size, geometry))
            .map_err(|_| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let sb_size4 = sb_size4(sb_size);
        let sb_width = sb_size4
            .checked_mul(4)
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let sb_per_gdf = block_size
            .checked_div(sb_width)
            .filter(|&value| value != 0)
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let unit_mi = block_size
            .checked_div(4)
            .filter(|&value| value != 0)
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        let grid_rows = mi_rows.div_ceil(unit_mi);
        let grid_cols = mi_cols.div_ceil(unit_mi);
        let cells = grid_rows
            .checked_mul(grid_cols)
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        Ok(Self {
            active: true,
            block_size,
            sb_size4,
            sb_per_gdf,
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
        let index = self
            .index(unit_row, unit_col)
            .ok_or_else(|| gdf_error(tile_offset, GDF_GRID_REASON))?;
        self.values[index] =
            read_use_gdf(work_unit.cdf_mut().tile_cdfs_mut(), symbols, tile_offset)?;
        Ok(())
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.grid_rows || col >= self.grid_cols {
            return None;
        }
        row.checked_mul(self.grid_cols)?.checked_add(col)
    }

    pub(crate) fn into_grid(self, tile_offset: ByteOffset) -> Result<Option<GdfBlockGrid>> {
        if !self.active {
            return Ok(None);
        }
        GdfBlockGrid::new(self.block_size, self.grid_rows, self.grid_cols, self.values)
            .map(Some)
            .map_err(|()| gdf_error(tile_offset, GDF_GRID_REASON))
    }
}

fn read_use_gdf(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let value = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseGdf, symbols)
        .map_err(|_| gdf_error(tile_offset, GDF_SYMBOL_REASON))?
        .get();
    if value > 1 {
        return Err(gdf_error(tile_offset, GDF_SYMBOL_REASON));
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

fn gdf_error(tile_offset: ByteOffset, reason: &'static str) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(tile_offset, reason)
}

#[cfg(test)]
#[path = "gdf_tests.rs"]
mod tests;
