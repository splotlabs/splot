// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tile::mi_width_log2;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CcsoState {
    pub(crate) active: bool,
    pub(crate) shift: u32,
    pub(crate) plane_enabled: [bool; CCSO_PLANES],
    pub(crate) blocks: [Vec<u8>; CCSO_PLANES],
    pub(crate) grid_rows: usize,
    pub(crate) grid_cols: usize,
}

impl CcsoState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
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
        Ok(Self::active(shift, plane_enabled, grid))
    }

    fn active(shift: u32, plane_enabled: [bool; CCSO_PLANES], grid: (usize, usize, usize)) -> Self {
        let (grid_rows, grid_cols, cells) = grid;
        Self {
            active: true,
            shift,
            plane_enabled,
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn active_luma_ccso_state(mi_rows: usize, mi_cols: usize) -> CcsoState {
        let shift = 8 - MI_SIZE_LOG2;
        let grid = ccso_grid(mi_rows, mi_cols, shift, ByteOffset::new(0)).unwrap();
        CcsoState::active(shift, [true, false, false], grid)
    }

    #[test]
    fn ccso_state_reads_only_at_aligned_origins() {
        let state = active_luma_ccso_state(270, 480);
        assert_eq!(state.shift, 6);
        assert_eq!((state.grid_rows, state.grid_cols), (5, 8));
        let unit_mask = (1usize << state.shift) - 1;
        let aligned = |mi: usize| mi & unit_mask == 0;
        assert!(aligned(0));
        assert!(!aligned(32));
        assert!(aligned(64));
    }

    #[test]
    fn ccso_unit_size_follows_tile_alignment() {
        assert_eq!(
            ccso_mi_width_log2_for_layout(false, 4, 1, 1, &[0, 16], &[0, 16]),
            6
        );
        assert_eq!(
            ccso_mi_width_log2_for_layout(false, 4, 2, 1, &[0, 16, 32], &[0, 16]),
            4
        );
        assert_eq!(
            ccso_mi_width_log2_for_layout(false, 5, 2, 1, &[0, 32, 64], &[0, 16]),
            5
        );
        assert_eq!(
            ccso_mi_width_log2_for_layout(false, 6, 2, 1, &[0, 64, 128], &[0, 16]),
            6
        );
        assert_eq!(
            ccso_mi_width_log2_for_layout(true, 4, 2, 1, &[0, 64, 128], &[0, 16]),
            4
        );
    }

    #[test]
    fn ccso_state_left_neighbour_context_matches_spec_8_3_2() {
        let mut state = active_luma_ccso_state(270, 480);
        assert_eq!(state.block_value(0, 0, 0), 0);
        state
            .set_block_value(0, 0, 0, 1, ByteOffset::new(0))
            .unwrap();
        assert_eq!(state.block_value(0, 0, 0), 1);
        let ctx = 2 * usize::from(state.block_value(0, 0, 0));
        assert_eq!(ctx, 2);
    }

    #[test]
    fn ccso_state_inactive_reads_nothing() {
        let state = CcsoState::inactive();
        assert!(!state.active);
        assert_eq!(state.grid_rows, 0);
    }

    #[test]
    fn ccso_state_rejects_out_of_grid_access() {
        let mut state = active_luma_ccso_state(270, 480);
        assert!(
            state
                .set_block_value(0, state.grid_rows, 0, 1, ByteOffset::new(0))
                .is_err()
        );
        assert!(
            state
                .set_block_value(0, 0, state.grid_cols, 1, ByteOffset::new(0))
                .is_err()
        );
        assert_eq!(state.block_value(0, 0, state.grid_cols), 0);
        assert_eq!(state.block_value(0, 99, 99), 0);
        assert_eq!(state.block_value(0, usize::MAX, 0), 0);
    }

    #[test]
    fn first_sb_block_16x64_horz4_partition_matches_avm_tx_16x16() {
        use super::super::{
            MI_SIZE, SelectableLumaTxGrid, TX_PARTITION_HORZ, TX_PARTITION_HORZ4,
            apply_tx_partition, table_usize, tx_size_from_dimensions,
        };
        use splot_core::tables::conversion::MAX_TX_SIZE_RECT;

        const TX_16X64: usize = 17;
        const TX_16X16: usize = 2;
        const TX_16X32: usize = 9;

        assert_eq!(
            table_usize("Max_Tx_Size_Rect", &MAX_TX_SIZE_RECT, 23).unwrap(),
            TX_16X64
        );
        let mut grid = SelectableLumaTxGrid::new(16, 4).unwrap();
        apply_tx_partition(&mut grid, 0, 0, TX_16X64, TX_PARTITION_HORZ4).unwrap();
        let records = grid.records_for_region(0, 0, 16, 4).unwrap();
        assert_eq!(records.len(), 4);
        for record in &records {
            assert_eq!((record.rows, record.cols), (4, 4));
            assert_eq!(
                tx_size_from_dimensions(record.cols * MI_SIZE, record.rows * MI_SIZE),
                Some(TX_16X16)
            );
        }
        let mut wrong = SelectableLumaTxGrid::new(16, 4).unwrap();
        apply_tx_partition(&mut wrong, 0, 0, TX_16X64, TX_PARTITION_HORZ).unwrap();
        assert_eq!(wrong.records_for_region(0, 0, 16, 4).unwrap().len(), 2);
        assert_eq!(tx_size_from_dimensions(16, 32), Some(TX_16X32));
    }
}
