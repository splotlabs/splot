// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tile::mi_width_log2;
use std::ops::Range;

use crate::bitstream::tile_payload::{DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector};
use crate::error::Result;
use crate::filters::ccso::CcsoUnitGrid;

use super::super::wienerns_lr_selectable_transform_record_error_reason;
use super::{CCSO_PLANES, CCSO_SYMBOL_VALUES, MI_SIZE_LOG2, read_tx_symbol};

const CCSO_GRID_OVERFLOW_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_ccso_grid_overflow";
const CCSO_BOUNDS_REASON: &str = "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds";
const CCSO_SYMBOL_RANGE_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range";
const CCSO_REFERENCE_REUSE_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_ccso_reference_reuse";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CcsoState {
    pub(crate) active: bool,
    pub(crate) shift: u32,
    pub(crate) plane_enabled: [bool; CCSO_PLANES],
    pub(crate) sb_reuse: [bool; CCSO_PLANES],
    pub(crate) blocks: [Vec<u8>; CCSO_PLANES],
    pub(crate) grid_rows: usize,
    pub(crate) grid_cols: usize,
}

impl CcsoState {
    pub(crate) fn try_clone_for_tile(&self, tile_offset: ByteOffset) -> Result<Self> {
        let clone_blocks = |source: &[u8]| -> Result<Vec<u8>> {
            let mut blocks = Vec::new();
            blocks
                .try_reserve_exact(source.len())
                .map_err(|_| ccso_error(tile_offset, CCSO_GRID_OVERFLOW_REASON))?;
            blocks.extend_from_slice(source);
            Ok(blocks)
        };
        Ok(Self {
            active: self.active,
            shift: self.shift,
            plane_enabled: self.plane_enabled,
            sb_reuse: self.sb_reuse,
            blocks: [
                clone_blocks(&self.blocks[0])?,
                clone_blocks(&self.blocks[1])?,
                clone_blocks(&self.blocks[2])?,
            ],
            grid_rows: self.grid_rows,
            grid_cols: self.grid_cols,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        ref_ccso_unit_grids: &[Option<CcsoUnitGrid>],
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let filter = sequence.filter.as_ref();
        let ccso = core.ccso_params.as_ref();
        let frame_flag = ccso.and_then(|c| c.ccso_frame_flag).unwrap_or(false);
        if !filter.is_some_and(|f| f.enable_ccso) || !frame_flag {
            return Ok(Self::inactive());
        }
        let shift = ccso_mi_width_log2(sequence, core, tile_offset)?;
        let grid = ccso_grid(mi_rows, mi_cols, shift, tile_offset)?;
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
            blocks: std::array::from_fn(|_| vec![0u8; cells]),
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
            return Err(ccso_error(tile_offset, CCSO_BOUNDS_REASON));
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
            )?;
            if value >= CCSO_SYMBOL_VALUES {
                return Err(ccso_error(tile_offset, CCSO_SYMBOL_RANGE_REASON));
            }
            self.set_block_value(plane, unit_row, unit_col, value as u8, tile_offset)?;
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
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let index = self
            .block_index(unit_row, unit_col)
            .ok_or_else(|| ccso_error(tile_offset, CCSO_BOUNDS_REASON))?;
        let cell = self
            .blocks
            .get_mut(plane)
            .and_then(|grid| grid.get_mut(index))
            .ok_or_else(|| ccso_error(tile_offset, CCSO_BOUNDS_REASON))?;
        *cell = value;
        Ok(())
    }

    fn block_index(&self, unit_row: usize, unit_col: usize) -> Option<usize> {
        if unit_row >= self.grid_rows || unit_col >= self.grid_cols {
            return None;
        }
        unit_row.checked_mul(self.grid_cols)?.checked_add(unit_col)
    }

    pub(crate) fn merge_tile(
        &mut self,
        tile: &Self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if self.active != tile.active
            || self.shift != tile.shift
            || self.plane_enabled != tile.plane_enabled
            || self.sb_reuse != tile.sb_reuse
            || self.grid_rows != tile.grid_rows
            || self.grid_cols != tile.grid_cols
            || self
                .blocks
                .iter()
                .zip(&tile.blocks)
                .any(|(frame, tile)| frame.len() != tile.len())
        {
            return Err(ccso_error(tile_offset, CCSO_BOUNDS_REASON));
        }
        if !self.active {
            return Ok(());
        }
        let unit_mi = 1usize
            .checked_shl(self.shift)
            .ok_or_else(|| ccso_error(tile_offset, CCSO_GRID_OVERFLOW_REASON))?;
        let row_end = mi_rows.end.div_ceil(unit_mi).min(self.grid_rows);
        let col_end = mi_cols.end.div_ceil(unit_mi).min(self.grid_cols);
        for row in mi_rows.start / unit_mi..row_end {
            for col in mi_cols.start / unit_mi..col_end {
                let index = self
                    .block_index(row, col)
                    .ok_or_else(|| ccso_error(tile_offset, CCSO_BOUNDS_REASON))?;
                for plane in 0..CCSO_PLANES {
                    self.blocks[plane][index] = tile.blocks[plane][index];
                }
            }
        }
        Ok(())
    }

    fn load_reused_blocks(
        &mut self,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        ref_ccso_unit_grids: &[Option<CcsoUnitGrid>],
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
                .ok_or_else(|| ccso_error(tile_offset, CCSO_REFERENCE_REUSE_REASON))?;
            let grid = ref_ccso_unit_grids
                .get(slot as usize)
                .and_then(Option::as_ref)
                .ok_or_else(|| ccso_error(tile_offset, CCSO_REFERENCE_REUSE_REASON))?;
            let source = grid
                .plane_blocks(plane)
                .ok_or_else(|| ccso_error(tile_offset, CCSO_REFERENCE_REUSE_REASON))?;
            if grid.shift() != self.shift
                || grid.grid_rows() != self.grid_rows
                || grid.grid_cols() != self.grid_cols
                || source.len() != self.blocks[plane].len()
            {
                return Err(ccso_error(tile_offset, CCSO_REFERENCE_REUSE_REASON));
            }
            self.blocks[plane].copy_from_slice(source);
        }
        Ok(())
    }

    pub(crate) fn into_grid(self, tile_offset: ByteOffset) -> Result<Option<CcsoUnitGrid>> {
        if !self.active {
            return Ok(None);
        }
        CcsoUnitGrid::new(
            self.active,
            self.shift,
            self.plane_enabled,
            self.blocks,
            self.grid_rows,
            self.grid_cols,
        )
        .map(Some)
        .map_err(|_| ccso_error(tile_offset, CCSO_GRID_OVERFLOW_REASON))
    }
}

fn ccso_mi_width_log2(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<u32> {
    let filter = sequence.filter.as_ref();
    let sb_mi_width_log2 = frame_sb_mi_width_log2(sequence, core, tile_offset)?;
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(ccso_error(tile_offset, CCSO_BOUNDS_REASON));
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

fn frame_sb_mi_width_log2(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<u32> {
    let partition = sequence
        .partition
        .as_ref()
        .ok_or_else(|| ccso_error(tile_offset, CCSO_BOUNDS_REASON))?;
    let frame_is_intra = core
        .frame_is_intra
        .ok_or_else(|| ccso_error(tile_offset, CCSO_BOUNDS_REASON))?;
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

fn ccso_grid(
    mi_rows: usize,
    mi_cols: usize,
    shift: u32,
    tile_offset: ByteOffset,
) -> Result<(usize, usize, usize)> {
    let unit_mi = 1usize
        .checked_shl(shift)
        .ok_or_else(|| ccso_error(tile_offset, CCSO_GRID_OVERFLOW_REASON))?;
    let grid_rows = mi_rows.div_ceil(unit_mi);
    let grid_cols = mi_cols.div_ceil(unit_mi);
    let cells = grid_rows
        .checked_mul(grid_cols)
        .ok_or_else(|| ccso_error(tile_offset, CCSO_GRID_OVERFLOW_REASON))?;
    Ok((grid_rows, grid_cols, cells))
}

fn ccso_error(tile_offset: ByteOffset, reason: &'static str) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(tile_offset, reason)
}

#[cfg(test)]
#[path = "ccso_tests.rs"]
mod tests;
