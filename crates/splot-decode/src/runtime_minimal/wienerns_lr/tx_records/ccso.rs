// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use crate::error::Result;
use crate::runtime_minimal::ccso::CcsoUnitGrid;
use crate::tile_payload::{DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector};

use super::super::{
    intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason,
};
use super::{CCSO_PLANES, CCSO_SYMBOL_VALUES, MI_SIZE_LOG2, read_tx_symbol};

const CCSO_GRID_OVERFLOW_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_ccso_grid_overflow";
const CCSO_BOUNDS_REASON: &str = "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds";
const CCSO_SYMBOL_RANGE_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct CcsoState {
    pub(super) active: bool,
    pub(super) shift: u32,
    pub(super) plane_enabled: [bool; CCSO_PLANES],
    pub(super) blocks: [Vec<u8>; CCSO_PLANES],
    pub(super) grid_rows: usize,
    pub(super) grid_cols: usize,
}

impl CcsoState {
    pub(in crate::runtime_minimal) fn new(
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
        let shift = if filter.is_some_and(|f| f.ccso_unit_matches_sb_size) {
            ccso_mi_width_log2(sequence, tile_offset)?
        } else {
            8 - MI_SIZE_LOG2
        };
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

    pub(super) fn inactive() -> Self {
        Self {
            active: false,
            shift: 0,
            plane_enabled: [false; CCSO_PLANES],
            blocks: [Vec::new(), Vec::new(), Vec::new()],
            grid_rows: 0,
            grid_cols: 0,
        }
    }

    pub(in crate::runtime_minimal) fn read_for_block(
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
            let ctx = if unit_col > 0 {
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

    pub(super) fn block_value(&self, plane: usize, unit_row: usize, unit_col: usize) -> u8 {
        self.block_index(unit_row, unit_col)
            .and_then(|index| self.blocks.get(plane).and_then(|grid| grid.get(index)))
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn set_block_value(
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

    pub(in crate::runtime_minimal) fn into_grid(
        self,
        tile_offset: ByteOffset,
    ) -> Result<Option<CcsoUnitGrid>> {
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

fn ccso_mi_width_log2(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<u32> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 4,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 5,
    })
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
