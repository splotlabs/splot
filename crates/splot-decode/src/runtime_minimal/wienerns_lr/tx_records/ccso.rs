// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-block CCSO `ccso_blk` reader for the selectable transform-record handoff.
//!
//! AV2 § 5.20.10.2 `read_ccso` is called per block between `read_cdef` and
//! `read_delta_qindex` (AV2 § 5.20.5.3 intra_frame_mode_info order). On the intra path
//! `sb_reuse_ccso[plane] == 0`, so each CCSO-unit-aligned block reads one `ccso_blk`
//! symbol per enabled plane. Omitting this read desynchronises every later symbol in
//! the superblock (it was the root cause of the BLOCK_16X64 transform-partition
//! mismatch versus the AVM reference for the ac0ej3 stream).

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use crate::error::Result;
use crate::tile_payload::{DecodeBlockFrontier, DecodeTileWorkUnit, TileCdfSelector};

use super::super::{
    intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason,
};
use super::{CCSO_PLANES, CCSO_SYMBOL_VALUES, MI_SIZE_LOG2, read_tx_symbol};

/// Per-block CCSO `ccso_blk` reader (AV2 § 5.20.10.2, `read_ccso`).
///
/// On the intra path `sb_reuse_ccso[plane] == 0`, so every CCSO-unit-aligned block
/// with `ccso_planes[plane]` set reads one `ccso_blk` symbol per enabled plane. The
/// decoded value feeds the § 8.3.2 left-neighbour context for later CCSO units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CcsoState {
    /// `enable_ccso && ccso_frame_flag`: whether any per-block `ccso_blk` is read.
    pub(super) active: bool,
    /// `CcsoLumaSizeLog2 - MI_SIZE_LOG2` (CCSO unit size in 4x4 luma units, log2).
    pub(super) shift: u32,
    /// `ccso_planes[plane]` per plane (luma + chroma).
    pub(super) plane_enabled: [bool; CCSO_PLANES],
    /// `CcsoBlks[plane]` grid in CCSO units (`0`/`1`), row-major over `cols` columns.
    pub(super) blocks: [Vec<u8>; CCSO_PLANES],
    /// CCSO-unit grid rows.
    pub(super) grid_rows: usize,
    /// CCSO-unit grid columns.
    pub(super) grid_cols: usize,
}

impl CcsoState {
    /// Builds the CCSO reader from the parsed sequence/frame headers.
    ///
    /// `mi_rows`/`mi_cols` are the frame MI dimensions; the gate already guarantees a
    /// single tile, so `MiColStart == 0` and the § 5.18.7.12 tile-alignment scan yields
    /// `a == 0` (`(a & 63) == 0`).
    pub(super) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let filter = sequence.filter.as_ref();
        let enable_ccso = filter.is_some_and(|f| f.enable_ccso);
        let ccso = core.ccso_params.as_ref();
        let frame_flag = ccso.and_then(|c| c.ccso_frame_flag).unwrap_or(false);
        let active = enable_ccso && frame_flag;
        if !active {
            return Ok(Self::inactive());
        }
        // AV2 § 5.18.7.12: CcsoLumaSizeLog2. Single tile -> a == 0, so the `(a & 63) == 0`
        // branch (value 8) is taken unless ccso_unit_matches_sb_size. Intentional
        // simplification (ceiling): the multi-tile value-7 (`(a & 63) == 32`) and value-6
        // branches are not modelled — only single-tile streams reach this path (the route
        // gate guarantees one tile); a multi-tile CCSO stream is the upgrade path.
        let matches_sb_size = filter.is_some_and(|f| f.ccso_unit_matches_sb_size);
        let luma_size_log2 = if matches_sb_size {
            ccso_mi_width_log2(sequence, tile_offset)? + MI_SIZE_LOG2
        } else {
            8
        };
        let shift = luma_size_log2 - MI_SIZE_LOG2;
        let unit_mi = 1usize << shift;
        let grid_rows = mi_rows.div_ceil(unit_mi);
        let grid_cols = mi_cols.div_ceil(unit_mi);
        let cells = grid_rows.checked_mul(grid_cols).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_ccso_grid_overflow",
            )
        })?;
        let mut plane_enabled = [false; CCSO_PLANES];
        if let Some(ccso) = ccso {
            for (plane, params) in ccso.planes.iter().enumerate().take(CCSO_PLANES) {
                plane_enabled[plane] = params.ccso_planes;
            }
        }
        Ok(Self {
            active: true,
            shift,
            plane_enabled,
            blocks: [vec![0u8; cells], vec![0u8; cells], vec![0u8; cells]],
            grid_rows,
            grid_cols,
        })
    }

    /// An inactive reader that consumes no `ccso_blk` symbols.
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

    /// Reads the per-block `ccso_blk` symbols (AV2 § 5.20.10.2) when the block is at a
    /// CCSO-unit-aligned origin. No-op for chroma-part blocks (`TreeType == CHROMA_PART`),
    /// non-aligned origins, or when CCSO is inactive.
    pub(super) fn read_for_block(
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
        if unit_row >= self.grid_rows || unit_col >= self.grid_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds",
            ));
        }
        for plane in 0..CCSO_PLANES {
            if !self.plane_enabled[plane] {
                continue;
            }
            // § 8.3.2: ctx = 2 * CcsoBlks[plane][row][colLeft] when a left CCSO unit
            // exists within the tile (MiColStart == 0), else 0.
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
                return Err(wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range",
                ));
            }
            self.set_block_value(plane, unit_row, unit_col, value as u8, tile_offset)?;
        }
        Ok(())
    }

    /// `CcsoBlks[plane][unit_row][unit_col]` (`0` for out-of-range indices).
    pub(super) fn block_value(&self, plane: usize, unit_row: usize, unit_col: usize) -> u8 {
        if unit_col >= self.grid_cols {
            return 0;
        }
        self.blocks
            .get(plane)
            .and_then(|grid| grid.get(unit_row * self.grid_cols + unit_col))
            .copied()
            .unwrap_or(0)
    }

    /// Stores `CcsoBlks[plane][unit_row][unit_col] = value`.
    pub(super) fn set_block_value(
        &mut self,
        plane: usize,
        unit_row: usize,
        unit_col: usize,
        value: u8,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let cols = self.grid_cols;
        // Guard the column separately: the flattened `row * cols + col` index would
        // otherwise alias a later row for an out-of-range column (e.g. `col == cols`
        // maps to row+1 col 0), silently corrupting a valid cell.
        let cell = if unit_col >= cols {
            None
        } else {
            self.blocks
                .get_mut(plane)
                .and_then(|grid| grid.get_mut(unit_row * cols + unit_col))
        }
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds",
            )
        })?;
        *cell = value;
        Ok(())
    }
}

/// `Mi_Width_Log2[ SbSize ]` for the § 5.18.2 intra-capped superblock (256×256 -> 128×128).
fn ccso_mi_width_log2(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<u32> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 4,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 5,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Builds an active luma-only CCSO reader matching the ac0ej3 first-frame state:
    /// `CcsoLumaSizeLog2 = 8` (256-luma-px units, `shift = 6`).
    fn ac0ej3_luma_ccso_state(mi_rows: usize, mi_cols: usize) -> CcsoState {
        let shift = 8 - MI_SIZE_LOG2;
        let unit_mi = 1usize << shift;
        let grid_rows = mi_rows.div_ceil(unit_mi);
        let grid_cols = mi_cols.div_ceil(unit_mi);
        let cells = grid_rows * grid_cols;
        CcsoState {
            active: true,
            shift,
            plane_enabled: [true, false, false],
            blocks: [vec![0u8; cells], vec![0u8; cells], vec![0u8; cells]],
            grid_rows,
            grid_cols,
        }
    }

    #[test]
    fn ccso_state_reads_only_at_aligned_origins() {
        // ac0ej3: 1920x1080 -> 480x270 MI. CcsoLumaSizeLog2 = 8 -> 64-MI units -> 8x5.
        let state = ac0ej3_luma_ccso_state(270, 480);
        assert_eq!(state.shift, 6);
        assert_eq!((state.grid_rows, state.grid_cols), (5, 8));
        let unit_mask = (1usize << state.shift) - 1;
        let aligned = |mi: usize| mi & unit_mask == 0;
        // Origin (0,0) of the first superblock is CCSO-aligned (read happens).
        assert!(aligned(0));
        // Origins inside the first CCSO unit (e.g. the second SB at MI col 32) are NOT
        // aligned, so no extra ccso_blk symbol is read there.
        assert!(!aligned(32));
        // The next aligned column is MI col 64 (the start of the third 128x128 SB).
        assert!(aligned(64));
    }

    #[test]
    fn ccso_state_left_neighbour_context_matches_spec_8_3_2() {
        // AV2 § 8.3.2 ccso_blk: ctx = 2 * CcsoBlks[plane][row][colLeft] when a left unit
        // exists (MiColStart == 0), else 0.
        let mut state = ac0ej3_luma_ccso_state(270, 480);
        // First unit column has no left neighbour -> ctx 0.
        assert_eq!(state.block_value(0, 0, 0), 0);
        // Set the left unit's luma value to 1; the next unit's context becomes 2.
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
        // AGENTS.md §8 negative case: a CCSO-grid write past the backing storage is
        // a typed error (never a panic), and an out-of-grid read saturates to 0.
        let mut state = ac0ej3_luma_ccso_state(270, 480); // 5x8 unit grid (40 cells)
        assert!(
            state
                .set_block_value(0, state.grid_rows, 0, 1, ByteOffset::new(0))
                .is_err(),
            "a write past the grid storage must error, not panic"
        );
        // An out-of-range column must error too (not silently alias the next row via
        // the flattened `row * cols + col` index).
        assert!(
            state
                .set_block_value(0, 0, state.grid_cols, 1, ByteOffset::new(0))
                .is_err(),
            "a write at unit_col == grid_cols must error, not alias a later row"
        );
        assert_eq!(
            state.block_value(0, 0, state.grid_cols),
            0,
            "an out-of-column read saturates to 0"
        );
        assert_eq!(
            state.block_value(0, 99, 99),
            0,
            "an out-of-grid read saturates to 0"
        );
    }

    // AV2 § 5.20.6.3: for the ac0ej3 first superblock the top-left BLOCK_16X64 (index 23,
    // maxRectTxSize TX_16X64) derives four TX_16X16 leaves via TX_PARTITION_HORZ4
    // (`tx_partition_type` symbol 3). The missing per-block CCSO `blk_idc` read previously
    // desynced this into two TX_16X32 (HORZ); the AVM inspect oracle pins the whole
    // BLOCK_16X64 footprint (MI rows 0..16, cols 0..4) to TX_16X16.
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
        // The previously-derived HORZ split (the desync this fix removes) is two TX_16X32.
        let mut wrong = SelectableLumaTxGrid::new(16, 4).unwrap();
        apply_tx_partition(&mut wrong, 0, 0, TX_16X64, TX_PARTITION_HORZ).unwrap();
        assert_eq!(wrong.records_for_region(0, 0, 16, 4).unwrap().len(), 2);
        assert_eq!(tx_size_from_dimensions(16, 32), Some(TX_16X32));
    }
}
