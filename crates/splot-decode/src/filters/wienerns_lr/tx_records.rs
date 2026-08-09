// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE, SIZE_TO_TX_PART_GROUP_LOOKUP,
    SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ, SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_HEIGHT, TX_WIDTH,
};
use splot_recon::{BitDepth, max_quantizer_index};
use std::ops::Range;

use crate::bitstream::tile_payload::{
    BlockSymbolTraceReadError, DecodeBlockFrontier, DecodeTileWorkUnit, IntraIstSyntax,
    TileCdfSelector,
};
use crate::error::Result;
use crate::filters::cdef::CdefUnitGrid;

use super::{
    intra_capped_seq_sb_size, selectable_missing_quantization_error, selectable_symbol_read_error,
};

pub(crate) mod ccso;
pub(crate) mod gdf;

const BLOCK_4X4: usize = 0;
const MI_SIZE: usize = 4;
const CDEF_UNIT_MI: usize = 16;
const TX_INVALID: usize = 255;
const TX_PARTITION_NONE: usize = 0;
const TX_PARTITION_SPLIT: usize = 1;
const TX_PARTITION_HORZ: usize = 2;
const TX_PARTITION_VERT: usize = 3;
const TX_PARTITION_HORZ4: usize = 4;
const TX_PARTITION_VERT4: usize = 5;
const TX_PARTITION_HORZ5: usize = 6;
const TX_PARTITION_VERT5: usize = 7;
const DELTA_Q_SMALL: usize = 7;
const DELTA_Q_REM_BITS_WIDTH: u32 = 3;
const DELTA_Q_SIGN_BIT_WIDTH: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Block4x4Extent {
    cols: usize,
    rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrTxSkipTransformRecord {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) skip_flag: bool,
    pub(crate) eob: usize,
    pub(crate) intra_ist: Option<IntraIstSyntax>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectableLumaTxRecord {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) tx_size: usize,
    pub(crate) middle: bool,
    pub(crate) scan_order: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectableLumaTxCell {
    tx_size: usize,
    middle: bool,
    scan_order: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectableLumaTxGrid {
    rows: usize,
    cols: usize,
    cells: Vec<Option<SelectableLumaTxCell>>,
    records: Vec<SelectableLumaTxRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeltaQState {
    present: bool,
    delta_q_res: u8,
    sb_size4: usize,
    current_q_index: i32,
    max_q: i32,
    current_sb: Option<(usize, usize)>,
    read_deltas: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefState {
    row_start: usize,
    col_start: usize,
    rows: usize,
    cols: usize,
    values: Vec<Option<usize>>,
    sb_size4: usize,
}

pub(crate) const CCSO_PLANES: usize = 3;
pub(crate) const CCSO_SYMBOL_VALUES: usize = 2;
pub(crate) const MI_SIZE_LOG2: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum SelectableTransformRecordError {
    #[error("selectable transform dimensions must be nonzero, got h4={h4}, w4={w4}")]
    EmptyTransform { h4: usize, w4: usize },
    #[error("selectable transform dimensions {width}x{height} do not map to a valid TxSize")]
    InvalidTxSize { width: usize, height: usize },
    #[error("selectable transform row {row} col {col} is outside {rows}x{cols}")]
    OutOfBounds {
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    },
    #[error("selectable transform cell row {row} col {col} was already populated")]
    Overlap { row: usize, col: usize },
    #[error("selectable transform region expected {expected} populated cells, got {actual}")]
    Incomplete { expected: usize, actual: usize },
    #[error("{table}[{index}] is outside the supported table shape")]
    TableIndex { table: &'static str, index: usize },
    #[error("{table}[{index}] value {value} is not a supported unsigned context")]
    TableValue {
        table: &'static str,
        index: usize,
        value: i32,
    },
    #[error("selectable transform branch is outside the supported subset: {reason}")]
    Unsupported { reason: &'static str },
}

#[allow(clippy::needless_pass_by_value)]
fn selectable_transform_record_error(
    error: SelectableTransformRecordError,
) -> crate::error::DecodeError {
    match error {
        SelectableTransformRecordError::Unsupported {
            reason: "record-allocation",
        } => selectable_allocation_error(),
        _ => crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into(),
    }
}

fn selectable_allocation_error() -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane: splot_recon::PlaneId::Y,
        context: "selectable transform-record storage",
    }
    .into()
}

fn selectable_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into()
}

fn selectable_read_error(
    tile_offset: ByteOffset,
    spec_section: &'static str,
) -> crate::error::DecodeError {
    selectable_symbol_read_error(tile_offset, spec_section)
}

impl DeltaQState {
    pub(crate) fn new(
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let delta_q = core.delta_q_params.as_ref();
        let present = delta_q.is_some_and(|params| params.delta_q_present);
        let delta_q_res = delta_q.map_or(0, |params| params.delta_q_res);
        let base_q_idx = core
            .quantization_params
            .as_ref()
            .ok_or_else(|| selectable_missing_quantization_error(tile_offset))?
            .base_q_idx;
        let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
            .map_err(|_| selectable_state_error())?;
        let sb_size4 = intra_delta_q_sb_size4(sequence, tile_offset)?;
        Ok(Self {
            present,
            delta_q_res,
            sb_size4,
            current_q_index: i32::try_from(base_q_idx).map_err(|_| selectable_state_error())?,
            max_q: i32::try_from(max_quantizer_index(bit_depth))
                .map_err(|_| selectable_state_error())?,
            current_sb: None,
            read_deltas: present,
        })
    }

    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
        skip: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.present {
            return Ok(());
        }
        let sb = self.superblock(frontier);
        if self.current_sb != Some(sb) {
            self.current_sb = Some(sb);
            self.read_deltas = true;
        }
        let delta_q_is_coded =
            delta_q_is_coded_for_block(frontier.b_size.index(), self.sb_size4, skip)
                .map_err(selectable_transform_record_error)?;
        if self.read_deltas && delta_q_is_coded {
            let delta_q_abs = read_delta_q_abs(work_unit, symbols, tile_offset)?;
            if delta_q_abs != 0 {
                let sign_bit = symbols
                    .read_literal(DELTA_Q_SIGN_BIT_WIDTH)
                    .map_err(|_| selectable_read_error(tile_offset, "5.20.5.11"))?
                    != 0;
                let delta_q_abs =
                    i32::try_from(delta_q_abs).map_err(|_| selectable_state_error())?;
                let reduced_delta_q_index = if sign_bit { -delta_q_abs } else { delta_q_abs };
                self.current_q_index = updated_current_q_index(
                    self.current_q_index,
                    reduced_delta_q_index,
                    self.delta_q_res,
                    self.max_q,
                );
            }
        }
        self.current_q_index = self.current_q_index.clamp(1, self.max_q);
        self.read_deltas = false;
        Ok(())
    }

    const fn superblock(&self, frontier: &DecodeBlockFrontier) -> (usize, usize) {
        (frontier.r / self.sb_size4, frontier.c / self.sb_size4)
    }

    pub(crate) fn qindex_u32(&self) -> u32 {
        self.current_q_index.max(0) as u32
    }
}

impl CdefState {
    pub(crate) fn try_for_tile(
        &self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        _tile_offset: ByteOffset,
    ) -> Result<Self> {
        let row_start = mi_rows.start / CDEF_UNIT_MI;
        let col_start = mi_cols.start / CDEF_UNIT_MI;
        let row_end = mi_rows
            .end
            .div_ceil(CDEF_UNIT_MI)
            .min(self.row_start + self.rows);
        let col_end = mi_cols
            .end
            .div_ceil(CDEF_UNIT_MI)
            .min(self.col_start + self.cols);
        let rows = row_end.saturating_sub(row_start);
        let cols = col_end.saturating_sub(col_start);
        let len = rows.checked_mul(cols).ok_or_else(selectable_state_error)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| selectable_allocation_error())?;
        values.resize(len, None);
        Ok(Self {
            row_start,
            col_start,
            rows,
            cols,
            values,
            sb_size4: self.sb_size4,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let rows = mi_rows.div_ceil(CDEF_UNIT_MI);
        let cols = mi_cols.div_ceil(CDEF_UNIT_MI);
        let values_len = rows.checked_mul(cols).ok_or_else(selectable_state_error)?;
        Ok(Self {
            row_start: 0,
            col_start: 0,
            rows,
            cols,
            values: vec![None; values_len],
            sb_size4: intra_delta_q_sb_size4(sequence, tile_offset)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        n4w: usize,
        n4h: usize,
        skip_txfm: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if frontier.is_chroma_part() {
            return Ok(());
        }
        let Some(cdef) = core.cdef_params.as_ref() else {
            return Err(selectable_state_error());
        };
        if !cdef.cdef_frame_enable {
            return Ok(());
        }
        if skip_txfm && cdef.cdef_on_skip_txfm_frame_enable == Some(false) {
            return Ok(());
        }
        let strengths = cdef.cdef_strengths.ok_or_else(selectable_state_error)? as usize;
        if !(1..=8).contains(&strengths) {
            return Err(selectable_state_error());
        }

        let unit_row = frontier.r / CDEF_UNIT_MI;
        let unit_col = frontier.c / CDEF_UNIT_MI;
        if unit_row < self.row_start
            || unit_col < self.col_start
            || unit_row >= self.row_start + self.rows
            || unit_col >= self.col_start + self.cols
        {
            return Err(selectable_state_error());
        }
        if self.value(unit_row, unit_col)?.is_some() {
            return Ok(());
        }

        let strength = if strengths == 1 {
            0
        } else {
            let tile_row_start = work_unit.mi_row_range().start as usize;
            let tile_col_start = work_unit.mi_col_range().start as usize;
            let ctx =
                self.cdef_index0_ctx_at(frontier.r, frontier.c, tile_row_start, tile_col_start)?;
            let cdef_index0 = read_tx_symbol_cdef(
                work_unit,
                symbols,
                TileCdfSelector::CdefIndex0 { ctx },
                tile_offset,
            )?;
            if cdef_index0 != 0 {
                0
            } else if strengths == 2 {
                1
            } else {
                read_tx_symbol_cdef(
                    work_unit,
                    symbols,
                    TileCdfSelector::CdefIndexMinus1 { strengths },
                    tile_offset,
                )?
                .checked_add(1)
                .ok_or_else(selectable_state_error)?
            }
        };
        self.fill_units(frontier.r, frontier.c, n4w, n4h, strength)
    }

    fn cdef_index0_ctx_at(
        &self,
        mi_row: usize,
        mi_col: usize,
        tile_row_start: usize,
        tile_col_start: usize,
    ) -> Result<usize> {
        let unit_row = mi_row / CDEF_UNIT_MI;
        let unit_col = mi_col / CDEF_UNIT_MI;
        let mut ctx = 0usize;
        let mut cnt = 0usize;

        if mi_col
            .checked_sub(CDEF_UNIT_MI)
            .is_some_and(|left_col| left_col >= tile_col_start)
        {
            if self.value(unit_row, unit_col - 1)? == Some(0) {
                ctx += 1;
            }
            cnt += 1;
        }
        if let Some(above_row) = mi_row.checked_sub(CDEF_UNIT_MI)
            && above_row >= tile_row_start
            && mi_row / self.sb_size4 == above_row / self.sb_size4
        {
            if self.value(unit_row - 1, unit_col)? == Some(0) {
                ctx += 1;
            }
            cnt += 1;
        }
        if ctx != 0 && cnt == ctx {
            ctx += 1;
        }
        Ok(ctx)
    }

    fn fill_units(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        strength: usize,
    ) -> Result<()> {
        let start_unit_row = mi_row / CDEF_UNIT_MI;
        let start_unit_col = mi_col / CDEF_UNIT_MI;
        let unit_rows = n4h.div_ceil(CDEF_UNIT_MI).max(1);
        let unit_cols = n4w.div_ceil(CDEF_UNIT_MI).max(1);
        let row_end = (self.row_start + self.rows).min(start_unit_row.saturating_add(unit_rows));
        let col_end = (self.col_start + self.cols).min(start_unit_col.saturating_add(unit_cols));
        for row in start_unit_row.max(self.row_start)..row_end {
            for col in start_unit_col.max(self.col_start)..col_end {
                let index = self.index(row, col)?;
                self.values[index] = Some(strength);
            }
        }
        Ok(())
    }

    fn value(&self, row: usize, col: usize) -> Result<Option<usize>> {
        let index = self.index(row, col)?;
        Ok(self.values[index])
    }

    fn index(&self, row: usize, col: usize) -> Result<usize> {
        let local_row = row
            .checked_sub(self.row_start)
            .ok_or_else(selectable_state_error)?;
        let local_col = col
            .checked_sub(self.col_start)
            .ok_or_else(selectable_state_error)?;
        if local_row >= self.rows || local_col >= self.cols {
            return Err(selectable_state_error());
        }
        local_row
            .checked_mul(self.cols)
            .and_then(|start| start.checked_add(local_col))
            .ok_or_else(selectable_state_error)
    }

    pub(crate) fn merge_tile(
        &mut self,
        tile: &Self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        _tile_offset: ByteOffset,
    ) -> Result<()> {
        let expected_row_start = mi_rows.start / CDEF_UNIT_MI;
        let expected_col_start = mi_cols.start / CDEF_UNIT_MI;
        let row_end = mi_rows.end.div_ceil(CDEF_UNIT_MI).min(self.rows);
        let col_end = mi_cols.end.div_ceil(CDEF_UNIT_MI).min(self.cols);
        if self.row_start != 0
            || self.col_start != 0
            || self.sb_size4 != tile.sb_size4
            || tile.row_start != expected_row_start
            || tile.col_start != expected_col_start
            || tile.rows != row_end.saturating_sub(expected_row_start)
            || tile.cols != col_end.saturating_sub(expected_col_start)
        {
            return Err(selectable_state_error());
        }
        for row in mi_rows.start / CDEF_UNIT_MI..row_end {
            for col in mi_cols.start / CDEF_UNIT_MI..col_end {
                let index = self.index(row, col)?;
                self.values[index] = tile.value(row, col)?;
            }
        }
        Ok(())
    }

    pub(crate) fn into_grid(self, _tile_offset: ByteOffset) -> Result<CdefUnitGrid> {
        if self.row_start != 0 || self.col_start != 0 {
            return Err(selectable_state_error());
        }
        CdefUnitGrid::new(self.rows, self.cols, self.values).map_err(|_| selectable_state_error())
    }
}

std::thread_local! {
    static SELECTABLE_TX_GRID_SCRATCH: std::cell::RefCell<Option<SelectableLumaTxGrid>> =
        const { std::cell::RefCell::new(None) };
}

fn with_selectable_tx_grid<R>(
    rows: usize,
    cols: usize,
    f: impl FnOnce(&mut SelectableLumaTxGrid) -> R,
) -> std::result::Result<R, SelectableTransformRecordError> {
    SELECTABLE_TX_GRID_SCRATCH.with(|slot| {
        let taken = slot.try_borrow_mut().ok().and_then(|mut slot| slot.take());
        let mut grid = match taken {
            Some(mut grid) if grid.rows == rows && grid.cols == cols => {
                grid.reset();
                grid
            }
            _ => SelectableLumaTxGrid::new(rows, cols)?,
        };
        let result = f(&mut grid);
        if let Ok(mut slot) = slot.try_borrow_mut() {
            *slot = Some(grid);
        }
        Ok(result)
    })
}

impl SelectableLumaTxGrid {
    fn new(rows: usize, cols: usize) -> std::result::Result<Self, SelectableTransformRecordError> {
        let cells = rows
            .checked_mul(cols)
            .ok_or(SelectableTransformRecordError::Unsupported {
                reason: "grid-size-overflow",
            })?;
        Ok(Self {
            rows,
            cols,
            cells: vec![None; cells],
            records: Vec::new(),
        })
    }

    fn reset(&mut self) {
        for record in &self.records {
            let row_end = record.row.saturating_add(record.rows).min(self.rows);
            let col_end = record.col.saturating_add(record.cols).min(self.cols);
            for row in record.row..row_end {
                let start = row.saturating_mul(self.cols).saturating_add(record.col);
                let end = row.saturating_mul(self.cols).saturating_add(col_end);
                if let Some(cells) = self.cells.get_mut(start..end) {
                    cells.fill(None);
                }
            }
        }
        self.records.clear();
    }

    fn set_tx_size(
        &mut self,
        row: usize,
        col: usize,
        h4: usize,
        w4: usize,
        middle: bool,
        scan_order: bool,
    ) -> std::result::Result<usize, SelectableTransformRecordError> {
        if h4 == 0 || w4 == 0 {
            return Err(SelectableTransformRecordError::EmptyTransform { h4, w4 });
        }
        let width = w4
            .checked_mul(MI_SIZE)
            .ok_or(SelectableTransformRecordError::Unsupported {
                reason: "tx-width-overflow",
            })?;
        let height =
            h4.checked_mul(MI_SIZE)
                .ok_or(SelectableTransformRecordError::Unsupported {
                    reason: "tx-height-overflow",
                })?;
        let tx_size = tx_size_from_dimensions(width, height)
            .ok_or(SelectableTransformRecordError::InvalidTxSize { width, height })?;

        if row >= self.rows || col >= self.cols {
            return Ok(tx_size);
        }

        self.records
            .try_reserve(1)
            .map_err(|_| SelectableTransformRecordError::Unsupported {
                reason: "record-allocation",
            })?;
        for r in row..row.saturating_add(h4) {
            if r >= self.rows {
                break;
            }
            for c in col..col.saturating_add(w4) {
                if c >= self.cols {
                    break;
                }
                let index = self.index(r, c)?;
                if self.cells[index].is_some() {
                    return Err(SelectableTransformRecordError::Overlap { row: r, col: c });
                }
            }
        }
        let cell = SelectableLumaTxCell {
            tx_size,
            middle,
            scan_order,
        };
        for r in row..row.saturating_add(h4) {
            if r >= self.rows {
                break;
            }
            for c in col..col.saturating_add(w4) {
                if c >= self.cols {
                    break;
                }
                let index = self.index(r, c)?;
                self.cells[index] = Some(cell);
            }
        }
        self.records.push(SelectableLumaTxRecord {
            row,
            col,
            rows: h4,
            cols: w4,
            tx_size,
            middle,
            scan_order,
        });
        Ok(tx_size)
    }

    fn records_for_region_into(
        &self,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
        records: &mut Vec<SelectableLumaTxRecord>,
    ) -> std::result::Result<(), SelectableTransformRecordError> {
        let region_rows = self.rows.saturating_sub(row).min(rows);
        let region_cols = self.cols.saturating_sub(col).min(cols);
        let expected = region_rows.checked_mul(region_cols).ok_or(
            SelectableTransformRecordError::Unsupported {
                reason: "region-size-overflow",
            },
        )?;
        let mut actual = 0usize;
        for r in row..row.saturating_add(region_rows) {
            for c in col..col.saturating_add(region_cols) {
                let index = self.index(r, c)?;
                if self.cells[index].is_some() {
                    actual += 1;
                }
            }
        }
        if actual != expected {
            return Err(SelectableTransformRecordError::Incomplete { expected, actual });
        }

        records.clear();
        records.try_reserve(self.records.len()).map_err(|_| {
            SelectableTransformRecordError::Unsupported {
                reason: "record-allocation",
            }
        })?;
        records.extend(self.records.iter().copied().filter(|record| {
            record.row >= row
                && record.col >= col
                && record.row < row + rows
                && record.col < col + cols
        }));
        let scan_order = self.cell(row, col)?.scan_order;
        if scan_order {
            records.sort_by_key(|record| (record.col, record.row));
        } else {
            records.sort_by_key(|record| (record.row, record.col));
        }
        Ok(())
    }

    #[cfg(test)]
    fn records_for_region(
        &self,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    ) -> std::result::Result<Vec<SelectableLumaTxRecord>, SelectableTransformRecordError> {
        let mut records = Vec::new();
        self.records_for_region_into(row, col, rows, cols, &mut records)?;
        Ok(records)
    }

    fn cell(
        &self,
        row: usize,
        col: usize,
    ) -> std::result::Result<SelectableLumaTxCell, SelectableTransformRecordError> {
        let index = self.index(row, col)?;
        self.cells[index].ok_or(SelectableTransformRecordError::Incomplete {
            expected: 1,
            actual: 0,
        })
    }

    fn index(
        &self,
        row: usize,
        col: usize,
    ) -> std::result::Result<usize, SelectableTransformRecordError> {
        if row >= self.rows || col >= self.cols {
            return Err(SelectableTransformRecordError::OutOfBounds {
                row,
                col,
                rows: self.rows,
                cols: self.cols,
            });
        }
        row.checked_mul(self.cols)
            .and_then(|start| start.checked_add(col))
            .ok_or(SelectableTransformRecordError::Unsupported {
                reason: "grid-index-overflow",
            })
    }
}

pub(crate) fn derive_inter_luma_tx_records_for_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    grid_size: (usize, usize),
    fsc_mode: u8,
    tile_offset: ByteOffset,
    records: &mut Vec<SelectableLumaTxRecord>,
) -> Result<()> {
    with_selectable_tx_grid(grid_size.0, grid_size.1, |grid| {
        let b_size = frontier.b_size.index();
        if b_size == BLOCK_4X4 {
            grid.set_tx_size(frontier.r, frontier.c, 1, 1, false, false)
                .map_err(selectable_transform_record_error)?;
        } else {
            let max_tx_size = table_usize("Max_Tx_Size_Rect", &MAX_TX_SIZE_RECT, b_size)
                .map_err(selectable_transform_record_error)?;
            let tx_w4 = tx_dimension("Tx_Width", &TX_WIDTH, max_tx_size)? / MI_SIZE;
            let tx_h4 = tx_dimension("Tx_Height", &TX_HEIGHT, max_tx_size)? / MI_SIZE;
            let extent = frontier_4x4_extent(frontier)?;
            let row_end = frontier
                .r
                .checked_add(extent.rows)
                .ok_or_else(selectable_state_error)?;
            let col_end = frontier
                .c
                .checked_add(extent.cols)
                .ok_or_else(selectable_state_error)?;
            for row in (frontier.r..row_end).step_by(tx_h4) {
                for col in (frontier.c..col_end).step_by(tx_w4) {
                    let Some(tx_partition) = read_tx_partition_symbols(
                        work_unit,
                        symbols,
                        grid,
                        row,
                        col,
                        max_tx_size,
                        b_size,
                        fsc_mode,
                        true,
                        tile_offset,
                    )?
                    else {
                        continue;
                    };
                    apply_tx_partition(grid, row, col, max_tx_size, tx_partition)
                        .map_err(selectable_transform_record_error)?;
                }
            }
        }
        let extent = frontier_4x4_extent(frontier)?;
        grid.records_for_region_into(frontier.r, frontier.c, extent.rows, extent.cols, records)
            .map_err(selectable_transform_record_error)?;
        Ok(())
    })
    .map_err(selectable_transform_record_error)?
}

fn frontier_4x4_extent(frontier: &DecodeBlockFrontier) -> Result<Block4x4Extent> {
    let n4w = frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| selectable_state_error())?;
    let n4h = frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| selectable_state_error())?;
    Ok(Block4x4Extent {
        cols: n4w,
        rows: n4h,
    })
}

#[allow(clippy::too_many_arguments)]
fn read_tx_partition_symbols(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    grid: &SelectableLumaTxGrid,
    row: usize,
    col: usize,
    tx_size: usize,
    mi_size: usize,
    fsc_mode: u8,
    is_inter: bool,
    tile_offset: ByteOffset,
) -> Result<Option<usize>> {
    if row >= grid.rows || col >= grid.cols {
        return Ok(None);
    }
    let tx_width = tx_dimension("Tx_Width", &TX_WIDTH, tx_size)?;
    let tx_height = tx_dimension("Tx_Height", &TX_HEIGHT, tx_size)?;
    let horz_tx = tx_size_from_dimensions(tx_width, tx_height >> 1);
    let vert_tx = tx_size_from_dimensions(tx_width >> 1, tx_height);
    let allow_horz = horz_tx.is_some();
    let allow_vert = vert_tx.is_some();
    let mut tx_partition = TX_PARTITION_NONE;

    let block_width = block_dimension("Block_Width", mi_size, true)?;
    let block_height = block_dimension("Block_Height", mi_size, false)?;
    let tx_fsc_mode = usize::from(fsc_mode != 0);
    let tx_is_inter = usize::from(is_inter);
    if block_width <= 64 && block_height <= 64 {
        let txfm_split_group = table_usize(
            "Size_To_Tx_Part_Group_Lookup",
            &SIZE_TO_TX_PART_GROUP_LOOKUP,
            mi_size,
        )
        .map_err(selectable_transform_record_error)?;
        let do_partition = read_tx_symbol(
            work_unit,
            symbols,
            TileCdfSelector::TxDoPartition {
                fsc_mode: tx_fsc_mode,
                is_inter: tx_is_inter,
                txfm_split_group,
            },
            tile_offset,
            "5.20.6.3",
        )? != 0;
        if do_partition {
            if allow_horz && allow_vert {
                let ctx = table_usize(
                    "Size_To_Tx_Type_Group_Vert_And_Horz",
                    &SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
                    mi_size,
                )
                .map_err(selectable_transform_record_error)?;
                let symbol = read_tx_symbol(
                    work_unit,
                    symbols,
                    TileCdfSelector::TxPartitionType {
                        fsc_mode: tx_fsc_mode,
                        is_inter: tx_is_inter,
                        ctx,
                        reduced: false,
                    },
                    tile_offset,
                    "5.20.6.3",
                )?;
                tx_partition = symbol + 1;
            } else {
                let vert_or_horz_group = table_usize(
                    "Size_To_Tx_Type_Group_Vert_Or_Horz",
                    &SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ,
                    mi_size,
                )
                .map_err(selectable_transform_record_error)?;
                if let Some(vert_or_horz_ctx) = vert_or_horz_group.checked_sub(1) {
                    let tx_2or3 = if work_unit.coeff_frame_facts().reduced_tx_set() != 0 {
                        0
                    } else {
                        read_tx_symbol(
                            work_unit,
                            symbols,
                            TileCdfSelector::Tx2Or3PartitionType {
                                fsc_mode: tx_fsc_mode,
                                is_inter: tx_is_inter,
                                ctx: vert_or_horz_ctx,
                            },
                            tile_offset,
                            "5.20.6.3",
                        )?
                    };
                    tx_partition = if allow_horz {
                        if tx_2or3 != 0 {
                            TX_PARTITION_HORZ4
                        } else {
                            TX_PARTITION_HORZ
                        }
                    } else if tx_2or3 != 0 {
                        TX_PARTITION_VERT4
                    } else {
                        TX_PARTITION_VERT
                    };
                } else {
                    tx_partition = if allow_horz {
                        TX_PARTITION_HORZ
                    } else {
                        TX_PARTITION_VERT
                    };
                }
            }
        }
    }

    Ok(Some(tx_partition))
}

fn apply_tx_partition(
    grid: &mut SelectableLumaTxGrid,
    mut row: usize,
    mut col: usize,
    tx_size: usize,
    tx_partition: usize,
) -> std::result::Result<usize, SelectableTransformRecordError> {
    let tx_width = tx_dimension_for_grid("Tx_Width", &TX_WIDTH, tx_size)?;
    let tx_height = tx_dimension_for_grid("Tx_Height", &TX_HEIGHT, tx_size)?;
    let mut w4 = tx_width / MI_SIZE;
    let mut h4 = tx_height / MI_SIZE;
    match tx_partition {
        TX_PARTITION_NONE => grid.set_tx_size(row, col, h4, w4, false, false),
        TX_PARTITION_HORZ => {
            h4 >>= 1;
            grid.set_tx_size(row, col, h4, w4, false, false)?;
            row += h4;
            grid.set_tx_size(row, col, h4, w4, false, false)
        }
        TX_PARTITION_VERT => {
            w4 >>= 1;
            grid.set_tx_size(row, col, h4, w4, false, false)?;
            col += w4;
            grid.set_tx_size(row, col, h4, w4, false, false)
        }
        TX_PARTITION_HORZ4 => {
            h4 >>= 2;
            for _ in 0..3 {
                grid.set_tx_size(row, col, h4, w4, false, false)?;
                row += h4;
            }
            grid.set_tx_size(row, col, h4, w4, false, false)
        }
        TX_PARTITION_VERT4 => {
            w4 >>= 2;
            for _ in 0..3 {
                grid.set_tx_size(row, col, h4, w4, false, false)?;
                col += w4;
            }
            grid.set_tx_size(row, col, h4, w4, false, false)
        }
        TX_PARTITION_HORZ5 => {
            h4 >>= 2;
            w4 >>= 1;
            grid.set_tx_size(row, col, h4, w4, false, false)?;
            col += w4;
            grid.set_tx_size(row, col, h4, w4, true, false)?;
            col -= w4;
            row += h4;
            h4 <<= 1;
            w4 <<= 1;
            grid.set_tx_size(row, col, h4, w4, true, false)?;
            row += h4;
            h4 >>= 1;
            w4 >>= 1;
            grid.set_tx_size(row, col, h4, w4, true, false)?;
            col += w4;
            grid.set_tx_size(row, col, h4, w4, true, false)
        }
        TX_PARTITION_VERT5 => {
            h4 >>= 1;
            w4 >>= 2;
            grid.set_tx_size(row, col, h4, w4, false, true)?;
            row += h4;
            grid.set_tx_size(row, col, h4, w4, true, true)?;
            col += w4;
            row -= h4;
            h4 <<= 1;
            w4 <<= 1;
            grid.set_tx_size(row, col, h4, w4, true, true)?;
            col += w4;
            h4 >>= 1;
            w4 >>= 1;
            grid.set_tx_size(row, col, h4, w4, true, true)?;
            row += h4;
            grid.set_tx_size(row, col, h4, w4, true, true)
        }
        TX_PARTITION_SPLIT => {
            w4 >>= 1;
            h4 >>= 1;
            grid.set_tx_size(row, col + w4, h4, w4, false, false)?;
            grid.set_tx_size(row, col, h4, w4, false, false)?;
            grid.set_tx_size(row + h4, col, h4, w4, false, false)?;
            grid.set_tx_size(row + h4, col + w4, h4, w4, false, false)
        }
        _ => Err(SelectableTransformRecordError::Unsupported {
            reason: "tx-partition-type",
        }),
    }
}

fn read_tx_symbol_cdef(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    read_tx_symbol(work_unit, symbols, selector, tile_offset, "5.20.10.1")
}

fn read_tx_symbol(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
    spec_section: &'static str,
) -> Result<usize> {
    let value = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|error| match error {
            BlockSymbolTraceReadError::Cdf(_) => selectable_state_error(),
            BlockSymbolTraceReadError::Symbol(_) => {
                selectable_symbol_read_error(tile_offset, spec_section)
            }
        })?;
    Ok(value)
}

fn read_delta_q_abs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let delta_q_abs = read_tx_symbol(
        work_unit,
        symbols,
        TileCdfSelector::DeltaQ,
        tile_offset,
        "5.20.5.11",
    )?;
    if delta_q_abs != DELTA_Q_SMALL {
        return Ok(delta_q_abs);
    }
    let delta_q_rem_bits = symbols
        .read_literal(DELTA_Q_REM_BITS_WIDTH)
        .map_err(|_| selectable_read_error(tile_offset, "5.20.5.11"))?
        + 1;
    let delta_q_abs_bits = symbols
        .read_literal(delta_q_rem_bits)
        .map_err(|_| selectable_read_error(tile_offset, "5.20.5.11"))?;
    let delta_q_large_base = 1usize << delta_q_rem_bits;
    Ok(delta_q_abs_bits as usize + delta_q_large_base + (DELTA_Q_SMALL - 2))
}

fn updated_current_q_index(
    current_q_index: i32,
    reduced_delta_q_index: i32,
    delta_q_res: u8,
    max_q: i32,
) -> i32 {
    let scale = 1_i32 << delta_q_res;
    let delta = reduced_delta_q_index.saturating_mul(scale);
    current_q_index.saturating_add(delta).clamp(1, max_q)
}

fn delta_q_is_coded_for_block(
    block_size: usize,
    sb_size4: usize,
    skip: bool,
) -> std::result::Result<bool, SelectableTransformRecordError> {
    let width4 = table_usize("Num_4x4_Blocks_Wide", &NUM_4X4_BLOCKS_WIDE, block_size)?;
    let height4 = table_usize("Num_4x4_Blocks_High", &NUM_4X4_BLOCKS_HIGH, block_size)?;
    Ok(width4 != sb_size4 || height4 != sb_size4 || !skip)
}

fn intra_delta_q_sb_size4(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 32,
    })
}

fn block_dimension(table: &'static str, block_size: usize, width: bool) -> Result<usize> {
    let values = if width {
        &NUM_4X4_BLOCKS_WIDE
    } else {
        &NUM_4X4_BLOCKS_HIGH
    };
    let dimension =
        table_usize(table, values, block_size).map_err(selectable_transform_record_error)?;
    Ok(dimension * MI_SIZE)
}

fn tx_dimension(table: &'static str, values: &[i32], tx_size: usize) -> Result<usize> {
    tx_dimension_for_grid(table, values, tx_size).map_err(selectable_transform_record_error)
}

fn tx_dimension_for_grid(
    table: &'static str,
    values: &[i32],
    tx_size: usize,
) -> std::result::Result<usize, SelectableTransformRecordError> {
    let value = values
        .get(tx_size)
        .copied()
        .ok_or(SelectableTransformRecordError::TableIndex {
            table,
            index: tx_size,
        })?;
    usize::try_from(value).map_err(|_| SelectableTransformRecordError::TableValue {
        table,
        index: tx_size,
        value,
    })
}

fn table_usize(
    table: &'static str,
    values: &[i32],
    index: usize,
) -> std::result::Result<usize, SelectableTransformRecordError> {
    let value = values
        .get(index)
        .copied()
        .ok_or(SelectableTransformRecordError::TableIndex { table, index })?;
    usize::try_from(value).map_err(|_| SelectableTransformRecordError::TableValue {
        table,
        index,
        value,
    })
}

fn tx_size_from_dimensions(width: usize, height: usize) -> Option<usize> {
    TX_WIDTH
        .iter()
        .zip(TX_HEIGHT.iter())
        .position(|(&tx_width, &tx_height)| {
            usize::try_from(tx_width).ok() == Some(width)
                && usize::try_from(tx_height).ok() == Some(height)
        })
        .filter(|&tx_size| tx_size != TX_INVALID)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_cdef_tests.rs"]
mod cdef_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "tx_records_grid_tests.rs"]
mod grid_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_delta_q_tests.rs"]
mod delta_q_tests;
