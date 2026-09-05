// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tile::mi_width_log2;
use std::ops::Range;
use std::sync::Arc;

use crate::bitstream::tile_payload::{DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector};
use crate::error::Result;
use crate::filters::ccso::CcsoUnitGrid;

use super::{CCSO_PLANES, CCSO_SYMBOL_VALUES, MI_SIZE_LOG2, read_tx_symbol};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CcsoState {
    pub(crate) active: bool,
    pub(crate) shift: u32,
    pub(crate) plane_enabled: [bool; CCSO_PLANES],
    pub(crate) sb_reuse: [bool; CCSO_PLANES],
    pub(crate) blocks: [Vec<u8>; CCSO_PLANES],
    pub(crate) row_start: usize,
    pub(crate) col_start: usize,
    pub(crate) grid_rows: usize,
    pub(crate) grid_cols: usize,
}

impl CcsoState {
    pub(crate) fn try_for_tile(
        &self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
    ) -> Result<Self> {
        if !self.active {
            return Ok(Self::inactive());
        }
        let unit_mi = 1usize
            .checked_shl(self.shift)
            .ok_or_else(ccso_state_error)?;
        let row_start = mi_rows.start / unit_mi;
        let col_start = mi_cols.start / unit_mi;
        let row_end = mi_rows.end.div_ceil(unit_mi).min(self.grid_rows);
        let col_end = mi_cols.end.div_ceil(unit_mi).min(self.grid_cols);
        let grid_rows = row_end.saturating_sub(row_start);
        let grid_cols = col_end.saturating_sub(col_start);
        let len = grid_rows
            .checked_mul(grid_cols)
            .ok_or_else(ccso_state_error)?;
        let copy_region = |source: &[u8], plane: splot_recon::PlaneId| -> Result<Vec<u8>> {
            let mut blocks = Vec::new();
            blocks
                .try_reserve_exact(len)
                .map_err(|_| ccso_allocation_error(plane))?;
            for row in row_start..row_end {
                let start = row
                    .checked_mul(self.grid_cols)
                    .and_then(|start| start.checked_add(col_start))
                    .ok_or_else(ccso_state_error)?;
                let end = start.checked_add(grid_cols).ok_or_else(ccso_state_error)?;
                blocks.extend_from_slice(source.get(start..end).ok_or_else(ccso_state_error)?);
            }
            Ok(blocks)
        };
        Ok(Self {
            active: self.active,
            shift: self.shift,
            plane_enabled: self.plane_enabled,
            sb_reuse: self.sb_reuse,
            blocks: [
                copy_region(&self.blocks[0], splot_recon::PlaneId::Y)?,
                copy_region(&self.blocks[1], splot_recon::PlaneId::U)?,
                copy_region(&self.blocks[2], splot_recon::PlaneId::V)?,
            ],
            row_start,
            col_start,
            grid_rows,
            grid_cols,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        ref_ccso_unit_grids: &[Option<Arc<CcsoUnitGrid>>],
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let filter = sequence.filter.as_ref();
        let ccso = core.ccso_params.as_ref();
        let frame_flag = ccso.and_then(|c| c.ccso_frame_flag).unwrap_or(false);
        if !filter.is_some_and(|f| f.enable_ccso) || !frame_flag {
            return Ok(Self::inactive());
        }
        let shift = ccso_mi_width_log2(sequence, core)?;
        let grid = ccso_grid(mi_rows, mi_cols, shift)?;
        let plane_enabled = std::array::from_fn(|plane| {
            ccso.and_then(|c| c.planes.get(plane))
                .is_some_and(|params| params.ccso_planes)
        });
        let sb_reuse = std::array::from_fn(|plane| {
            ccso.and_then(|c| c.planes.get(plane))
                .is_some_and(|params| params.sb_reuse_ccso)
        });
        let mut state = Self::active(shift, plane_enabled, sb_reuse, grid);
        state.load_reused_blocks(core, ref_frame_idx, ref_ccso_unit_grids, tile_offset)?;
        Ok(state)
    }

    fn active(
        shift: u32,
        plane_enabled: [bool; CCSO_PLANES],
        sb_reuse: [bool; CCSO_PLANES],
        grid: (usize, usize, usize),
    ) -> Self {
        let (grid_rows, grid_cols, cells) = grid;
        Self {
            active: true,
            shift,
            plane_enabled,
            sb_reuse,
            blocks: std::array::from_fn(|_| vec![0; cells]),
            row_start: 0,
            col_start: 0,
            grid_rows,
            grid_cols,
        }
    }

    pub(crate) fn inactive() -> Self {
        Self {
            active: false,
            shift: 0,
            plane_enabled: [false; CCSO_PLANES],
            sb_reuse: [false; CCSO_PLANES],
            blocks: [Vec::new(), Vec::new(), Vec::new()],
            row_start: 0,
            col_start: 0,
            grid_rows: 0,
            grid_cols: 0,
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
        let unit_mask = (1usize << self.shift) - 1;
        if frontier.r & unit_mask != 0 || frontier.c & unit_mask != 0 {
            return Ok(());
        }
        let unit_row = frontier.r >> self.shift;
        let unit_col = frontier.c >> self.shift;
        if self.block_index(unit_row, unit_col).is_none() {
            return Err(ccso_state_error());
        }
        for plane in 0..CCSO_PLANES {
            if !self.plane_enabled[plane] {
                continue;
            }
            if self.sb_reuse[plane] {
                continue;
            }
            let tile_col_start = work_unit.mi_col_range().start as usize;
            let unit_width = 1usize << self.shift;
            let left_available = frontier
                .c
                .checked_sub(unit_width)
                .is_some_and(|left_col| left_col >= tile_col_start);
            let ctx = if left_available {
                let left = self.block_value(plane, unit_row, unit_col - 1);
                2 * usize::from(left)
            } else {
                0
            };
            let value = read_tx_symbol(
                work_unit,
                symbols,
                TileCdfSelector::CcsoBlk { plane, ctx },
                tile_offset,
                "5.20.10.2",
            )?;
            if value >= CCSO_SYMBOL_VALUES {
                return Err(ccso_state_error());
            }
            self.set_block_value(plane, unit_row, unit_col, value as u8)?;
        }
        Ok(())
    }

    pub(crate) fn block_value(&self, plane: usize, unit_row: usize, unit_col: usize) -> u8 {
        self.block_index(unit_row, unit_col)
            .and_then(|index| self.blocks.get(plane).and_then(|grid| grid.get(index)))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn set_block_value(
        &mut self,
        plane: usize,
        unit_row: usize,
        unit_col: usize,
        value: u8,
    ) -> Result<()> {
        let index = self
            .block_index(unit_row, unit_col)
            .ok_or_else(ccso_state_error)?;
        let cell = self
            .blocks
            .get_mut(plane)
            .and_then(|grid| grid.get_mut(index))
            .ok_or_else(ccso_state_error)?;
        *cell = value;
        Ok(())
    }

    fn block_index(&self, unit_row: usize, unit_col: usize) -> Option<usize> {
        crate::tile::local_grid_index(
            unit_row,
            unit_col,
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
    ) -> Result<()> {
        if self.active != tile.active
            || self.shift != tile.shift
            || self.plane_enabled != tile.plane_enabled
            || self.sb_reuse != tile.sb_reuse
            || self.row_start != 0
            || self.col_start != 0
        {
            return Err(ccso_state_error());
        }
        if !self.active {
            return Ok(());
        }
        let unit_mi = 1usize
            .checked_shl(self.shift)
            .ok_or_else(ccso_state_error)?;
        let row_end = mi_rows.end.div_ceil(unit_mi).min(self.grid_rows);
        let col_end = mi_cols.end.div_ceil(unit_mi).min(self.grid_cols);
        let expected_row_start = mi_rows.start / unit_mi;
        let expected_col_start = mi_cols.start / unit_mi;
        if tile.row_start != expected_row_start
            || tile.col_start != expected_col_start
            || tile.grid_rows != row_end.saturating_sub(expected_row_start)
            || tile.grid_cols != col_end.saturating_sub(expected_col_start)
        {
            return Err(ccso_state_error());
        }
        for row in mi_rows.start / unit_mi..row_end {
            for col in mi_cols.start / unit_mi..col_end {
                let index = self.block_index(row, col).ok_or_else(ccso_state_error)?;
                for plane in 0..CCSO_PLANES {
                    let tile_index = tile.block_index(row, col).ok_or_else(ccso_state_error)?;
                    self.blocks[plane][index] = tile.blocks[plane][tile_index];
                }
            }
        }
        Ok(())
    }

    fn load_reused_blocks(
        &mut self,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        ref_ccso_unit_grids: &[Option<Arc<CcsoUnitGrid>>],
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let Some(ccso) = core.ccso_params.as_ref() else {
            return Ok(());
        };
        for plane in 0..CCSO_PLANES {
            if !self.sb_reuse[plane] {
                continue;
            }
            let ref_index = ccso
                .planes
                .get(plane)
                .and_then(|params| params.ccso_ref_idx)
                .unwrap_or(0);
            let slot = ref_frame_idx
                .get(ref_index as usize)
                .copied()
                .ok_or_else(|| ccso_reuse_error(tile_offset))?;
            let grid = ref_ccso_unit_grids
                .get(slot as usize)
                .and_then(Option::as_ref)
                .ok_or_else(|| ccso_reuse_error(tile_offset))?;
            let source = grid
                .plane_blocks(plane)
                .ok_or_else(|| ccso_reuse_error(tile_offset))?;
            if grid.shift() != self.shift
                || grid.grid_rows() != self.grid_rows
                || grid.grid_cols() != self.grid_cols
                || source.len() != self.blocks[plane].len()
            {
                return Err(ccso_reuse_error(tile_offset));
            }
            self.blocks[plane].copy_from_slice(source);
        }
        Ok(())
    }

    pub(crate) fn into_grid(mut self) -> Result<Option<CcsoUnitGrid>> {
        if !self.active {
            return Ok(None);
        }
        if self.row_start != 0 || self.col_start != 0 {
            return Err(ccso_state_error());
        }
        let blocks = core::mem::take(&mut self.blocks);
        CcsoUnitGrid::new(
            self.active,
            self.shift,
            self.plane_enabled,
            blocks,
            self.grid_rows,
            self.grid_cols,
        )
        .map(Some)
        .map_err(|_| ccso_state_error())
    }
}

fn ccso_mi_width_log2(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Result<u32> {
    let filter = sequence.filter.as_ref();
    let sb_mi_width_log2 = frame_sb_mi_width_log2(sequence, core)?;
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(ccso_state_error());
    };
    Ok(ccso_mi_width_log2_for_layout(
        filter.is_some_and(|f| f.ccso_unit_matches_sb_size),
        sb_mi_width_log2,
        tile_info.tile_cols,
        tile_info.tile_rows,
        &tile_info.mi_col_starts,
        &tile_info.mi_row_starts,
    ))
}

fn frame_sb_mi_width_log2(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Result<u32> {
    let partition = sequence.partition.as_ref().ok_or_else(ccso_state_error)?;
    let frame_is_intra = core.frame_is_intra.ok_or_else(ccso_state_error)?;
    let sb_size = if partition.use_256x256_superblock {
        if frame_is_intra {
            SuperblockSize::Block128x128
        } else {
            SuperblockSize::Block256x256
        }
    } else if partition.use_128x128_superblock {
        SuperblockSize::Block128x128
    } else {
        SuperblockSize::Block64x64
    };
    Ok(mi_width_log2(sb_size))
}

fn ccso_mi_width_log2_for_layout(
    ccso_unit_matches_sb_size: bool,
    sb_mi_width_log2: u32,
    tile_cols: u32,
    tile_rows: u32,
    mi_col_starts: &[u32],
    mi_row_starts: &[u32],
) -> u32 {
    if ccso_unit_matches_sb_size {
        return sb_mi_width_log2;
    }
    if tile_cols <= 1 && tile_rows <= 1 {
        return 8 - MI_SIZE_LOG2;
    }
    let mut alignment = 0;
    for &start in mi_col_starts.iter().take(tile_cols as usize) {
        alignment |= start;
    }
    for &start in mi_row_starts.iter().take(tile_rows as usize) {
        alignment |= start;
    }
    let ccso_luma_size_log2 = if alignment.trailing_zeros() >= 6 {
        8
    } else if alignment.trailing_zeros() >= 5 {
        7
    } else {
        6
    };
    ccso_luma_size_log2 - MI_SIZE_LOG2
}

fn ccso_grid(mi_rows: usize, mi_cols: usize, shift: u32) -> Result<(usize, usize, usize)> {
    let unit_mi = 1usize.checked_shl(shift).ok_or_else(ccso_state_error)?;
    let grid_rows = mi_rows.div_ceil(unit_mi);
    let grid_cols = mi_cols.div_ceil(unit_mi);
    let cells = grid_rows
        .checked_mul(grid_cols)
        .ok_or_else(ccso_state_error)?;
    Ok((grid_rows, grid_cols, cells))
}

fn ccso_reuse_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    crate::pipeline::malformed_tile_payload(
        tile_offset,
        "6.17.7.8",
        "CCSO block-flag reuse references an incompatible saved grid",
    )
}

fn ccso_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into()
}

fn ccso_allocation_error(plane: splot_recon::PlaneId) -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane,
        context: "CCSO block grid",
    }
    .into()
}

#[cfg(test)]
#[path = "ccso_tests.rs"]
mod tests;
