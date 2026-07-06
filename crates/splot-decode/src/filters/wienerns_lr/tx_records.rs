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

use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, IntraIstSyntax, TileCdfSelector,
};
use crate::error::Result;
use crate::filters::cdef::CdefUnitGrid;

use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

pub(crate) mod ccso;

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

macro_rules! selectable_reason {
    ($suffix:literal) => {
        concat!(
            "unsupported_wienerns_lr_selectable_transform_records_",
            $suffix
        )
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Block4x4Extent {
    cols: usize,
    rows: usize,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "crate-visible handoff record crosses transform-record module boundary"
    )
)]
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
    current_q_index: i64,
    max_q: i64,
    current_sb: Option<(usize, usize)>,
    read_deltas: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefState {
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

impl SelectableTransformRecordError {
    fn unsupported_reason(&self) -> &'static str {
        match self {
            Self::EmptyTransform { .. } => selectable_reason!("empty_transform"),
            Self::InvalidTxSize { .. } => selectable_reason!("invalid_tx_size"),
            Self::OutOfBounds { .. } => selectable_reason!("out_of_bounds"),
            Self::Overlap { .. } => selectable_reason!("overlap"),
            Self::Incomplete { .. } => selectable_reason!("incomplete_grid"),
            Self::TableIndex { .. } => selectable_reason!("table_index"),
            Self::TableValue { .. } => selectable_reason!("table_value"),
            Self::Unsupported { reason } => selectable_unsupported_reason(reason),
        }
    }
}

fn selectable_unsupported_reason(reason: &'static str) -> &'static str {
    match reason {
        "grid-size-overflow" => selectable_reason!("grid_size_overflow"),
        "tx-width-overflow" => selectable_reason!("tx_width_overflow"),
        "tx-height-overflow" => selectable_reason!("tx_height_overflow"),
        "record-allocation" => selectable_reason!("record_allocation"),
        "region-size-overflow" => selectable_reason!("region_size_overflow"),
        "grid-index-overflow" => selectable_reason!("grid_index_overflow"),
        "horz4-loop" => selectable_reason!("horz4_loop"),
        "vert4-loop" => selectable_reason!("vert4_loop"),
        "tx-partition-type" => selectable_reason!("tx_partition_type"),
        _ => selectable_reason!("unsupported_branch"),
    }
}

fn selectable_decode_error(
    tile_offset: ByteOffset,
    reason: &'static str,
) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(tile_offset, reason)
}

#[allow(clippy::needless_pass_by_value)]
fn selectable_transform_record_error(
    error: SelectableTransformRecordError,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    selectable_decode_error(tile_offset, error.unsupported_reason())
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
            .ok_or_else(|| {
                selectable_decode_error(tile_offset, selectable_reason!("missing_quantization"))
            })?
            .base_q_idx;
        let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
            .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("bit_depth")))?;
        let sb_size4 = intra_delta_q_sb_size4(sequence, tile_offset)?;
        Ok(Self {
            present,
            delta_q_res,
            sb_size4,
            current_q_index: i64::from(base_q_idx),
            max_q: i64::from(max_quantizer_index(bit_depth)),
            current_sb: None,
            read_deltas: present,
        })
    }

    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
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
        if self.read_deltas {
            let delta_q_abs = read_delta_q_abs(work_unit, symbols, tile_offset)?;
            if delta_q_abs != 0 {
                let sign_bit = symbols.read_literal(DELTA_Q_SIGN_BIT_WIDTH).map_err(|_| {
                    selectable_decode_error(tile_offset, selectable_reason!("delta_q_sign_read"))
                })? != 0;
                let delta_q_abs = i64::try_from(delta_q_abs).map_err(|_| {
                    selectable_decode_error(tile_offset, selectable_reason!("delta_q_abs_width"))
                })?;
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
        u32::try_from(self.current_q_index.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
    }
}

impl CdefState {
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let rows = mi_rows.div_ceil(CDEF_UNIT_MI);
        let cols = mi_cols.div_ceil(CDEF_UNIT_MI);
        let values_len = rows.checked_mul(cols).ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("cdef_grid_overflow"))
        })?;
        Ok(Self {
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
            return Err(selectable_decode_error(
                tile_offset,
                selectable_reason!("missing_cdef_params"),
            ));
        };
        if !cdef.cdef_frame_enable {
            return Ok(());
        }
        if skip_txfm && cdef.cdef_on_skip_txfm_frame_enable == Some(false) {
            return Ok(());
        }
        let strengths = cdef.cdef_strengths.ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("missing_cdef_strengths"))
        })? as usize;
        if !(1..=8).contains(&strengths) {
            return Err(selectable_decode_error(
                tile_offset,
                selectable_reason!("cdef_strengths"),
            ));
        }

        let unit_row = frontier.r / CDEF_UNIT_MI;
        let unit_col = frontier.c / CDEF_UNIT_MI;
        if unit_row >= self.rows || unit_col >= self.cols {
            return Err(selectable_decode_error(
                tile_offset,
                selectable_reason!("cdef_bounds"),
            ));
        }
        if self.value(unit_row, unit_col, tile_offset)?.is_some() {
            return Ok(());
        }

        let strength = if strengths == 1 {
            0
        } else {
            let tile_row_start = work_unit.mi_row_range().start as usize;
            let tile_col_start = work_unit.mi_col_range().start as usize;
            let ctx = self.cdef_index0_ctx_at(
                frontier.r,
                frontier.c,
                tile_row_start,
                tile_col_start,
                tile_offset,
            )?;
            let cdef_index0 = read_tx_symbol(
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
                read_tx_symbol(
                    work_unit,
                    symbols,
                    TileCdfSelector::CdefIndexMinus1 { strengths },
                    tile_offset,
                )?
                .checked_add(1)
                .ok_or_else(|| {
                    selectable_decode_error(tile_offset, selectable_reason!("cdef_index_overflow"))
                })?
            }
        };
        self.fill_units(frontier.r, frontier.c, n4w, n4h, strength, tile_offset)
    }

    fn cdef_index0_ctx_at(
        &self,
        mi_row: usize,
        mi_col: usize,
        tile_row_start: usize,
        tile_col_start: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let unit_row = mi_row / CDEF_UNIT_MI;
        let unit_col = mi_col / CDEF_UNIT_MI;
        let mut ctx = 0usize;
        let mut cnt = 0usize;

        if mi_col
            .checked_sub(CDEF_UNIT_MI)
            .is_some_and(|left_col| left_col >= tile_col_start)
        {
            if self.value(unit_row, unit_col - 1, tile_offset)? == Some(0) {
                ctx += 1;
            }
            cnt += 1;
        }
        if let Some(above_row) = mi_row.checked_sub(CDEF_UNIT_MI)
            && above_row >= tile_row_start
            && mi_row / self.sb_size4 == above_row / self.sb_size4
        {
            if self.value(unit_row - 1, unit_col, tile_offset)? == Some(0) {
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
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let start_unit_row = mi_row / CDEF_UNIT_MI;
        let start_unit_col = mi_col / CDEF_UNIT_MI;
        let unit_rows = n4h.div_ceil(CDEF_UNIT_MI).max(1);
        let unit_cols = n4w.div_ceil(CDEF_UNIT_MI).max(1);
        for row in start_unit_row..start_unit_row.saturating_add(unit_rows).min(self.rows) {
            for col in start_unit_col..start_unit_col.saturating_add(unit_cols).min(self.cols) {
                let index = self.index(row, col, tile_offset)?;
                self.values[index] = Some(strength);
            }
        }
        Ok(())
    }

    fn value(&self, row: usize, col: usize, tile_offset: ByteOffset) -> Result<Option<usize>> {
        let index = self.index(row, col, tile_offset)?;
        Ok(self.values[index])
    }

    fn index(&self, row: usize, col: usize, tile_offset: ByteOffset) -> Result<usize> {
        if row >= self.rows || col >= self.cols {
            return Err(selectable_decode_error(
                tile_offset,
                selectable_reason!("cdef_index_bounds"),
            ));
        }
        row.checked_mul(self.cols)
            .and_then(|start| start.checked_add(col))
            .ok_or_else(|| {
                selectable_decode_error(tile_offset, selectable_reason!("cdef_index_overflow"))
            })
    }

    pub(crate) fn into_grid(self, tile_offset: ByteOffset) -> Result<CdefUnitGrid> {
        CdefUnitGrid::new(self.rows, self.cols, self.values).map_err(|_| {
            selectable_decode_error(tile_offset, selectable_reason!("cdef_grid_shape"))
        })
    }
}

std::thread_local! {
    static SELECTABLE_TX_GRID_SCRATCH: std::cell::RefCell<Option<SelectableLumaTxGrid>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `f` with a per-thread reusable frame-sized grid reset to all-`None`
/// cells, so each coding block avoids reallocating and refilling the grid.
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

    /// Restores the all-`None` cell state. `set_tx_size` writes cells only
    /// inside rects it also records, so clearing the recorded rects (with the
    /// same edge clipping) covers every set cell.
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

    fn records_for_region(
        &self,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    ) -> std::result::Result<Vec<SelectableLumaTxRecord>, SelectableTransformRecordError> {
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

        let mut records: Vec<_> = self
            .records
            .iter()
            .copied()
            .filter(|record| {
                record.row >= row
                    && record.col >= col
                    && record.row < row + rows
                    && record.col < col + cols
            })
            .collect();
        let scan_order = self.cell(row, col)?.scan_order;
        if scan_order {
            records.sort_by_key(|record| (record.col, record.row));
        } else {
            records.sort_by_key(|record| (record.row, record.col));
        }
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
) -> Result<Vec<SelectableLumaTxRecord>> {
    with_selectable_tx_grid(grid_size.0, grid_size.1, |grid| {
    let b_size = frontier.b_size.index();
    if b_size == BLOCK_4X4 {
        grid.set_tx_size(frontier.r, frontier.c, 1, 1, false, false)
            .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
    } else {
        let max_tx_size = table_usize("Max_Tx_Size_Rect", &MAX_TX_SIZE_RECT, b_size)
            .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
        let tx_w4 = tx_dimension("Tx_Width", &TX_WIDTH, max_tx_size, tile_offset)? / MI_SIZE;
        let tx_h4 = tx_dimension("Tx_Height", &TX_HEIGHT, max_tx_size, tile_offset)? / MI_SIZE;
        let extent = frontier_4x4_extent(
            frontier,
            tile_offset,
            selectable_reason!("inter_block_width"),
            selectable_reason!("inter_block_height"),
        )?;
        let row_end = frontier.r.checked_add(extent.rows).ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("inter_row_end_overflow"))
        })?;
        let col_end = frontier.c.checked_add(extent.cols).ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("inter_col_end_overflow"))
        })?;
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
                if trace_tx_partition_for(row, col) {
                    eprintln!(
                        "tx partition inter row={row} col={col} max_tx_size={max_tx_size} b_size={b_size} partition={tx_partition} checkpoint={:?}",
                        symbols.checkpoint(),
                    );
                }
                apply_tx_partition(grid, row, col, max_tx_size, tx_partition)
                    .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
            }
        }
    }
    let extent = frontier_4x4_extent(
        frontier,
        tile_offset,
        selectable_reason!("inter_region_width"),
        selectable_reason!("inter_region_height"),
    )?;
    let records = grid
        .records_for_region(frontier.r, frontier.c, extent.rows, extent.cols)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
    if trace_tx_partition_for(frontier.r, frontier.c) {
        eprintln!(
            "tx records inter r={} c={} b={} records={records:?}",
            frontier.r,
            frontier.c,
            frontier.b_size.index(),
        );
    }
    Ok(records)
    })
    .map_err(|error| selectable_transform_record_error(error, tile_offset))?
}

fn frontier_4x4_extent(
    frontier: &DecodeBlockFrontier,
    tile_offset: ByteOffset,
    width_reason: &'static str,
    height_reason: &'static str,
) -> Result<Block4x4Extent> {
    let n4w = frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| selectable_decode_error(tile_offset, width_reason))?;
    let n4h = frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| selectable_decode_error(tile_offset, height_reason))?;
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
    let tx_width = tx_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)?;
    let tx_height = tx_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)?;
    let horz_tx = tx_size_from_dimensions(tx_width, tx_height >> 1);
    let vert_tx = tx_size_from_dimensions(tx_width >> 1, tx_height);
    let allow_horz = horz_tx.is_some();
    let allow_vert = vert_tx.is_some();
    let mut tx_partition = TX_PARTITION_NONE;

    let block_width = block_dimension("Block_Width", mi_size, true, tile_offset)?;
    let block_height = block_dimension("Block_Height", mi_size, false, tile_offset)?;
    let tx_fsc_mode = usize::from(fsc_mode != 0);
    let tx_is_inter = usize::from(is_inter);
    if block_width <= 64 && block_height <= 64 {
        let txfm_split_group = table_usize(
            "Size_To_Tx_Part_Group_Lookup",
            &SIZE_TO_TX_PART_GROUP_LOOKUP,
            mi_size,
        )
        .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
        let do_partition = read_tx_symbol(
            work_unit,
            symbols,
            TileCdfSelector::TxDoPartition {
                fsc_mode: tx_fsc_mode,
                is_inter: tx_is_inter,
                txfm_split_group,
            },
            tile_offset,
        )? != 0;
        if do_partition {
            if allow_horz && allow_vert {
                let ctx = table_usize(
                    "Size_To_Tx_Type_Group_Vert_And_Horz",
                    &SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
                    mi_size,
                )
                .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
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
                )?;
                tx_partition = symbol.checked_add(1).ok_or_else(|| {
                    selectable_decode_error(
                        tile_offset,
                        selectable_reason!("partition_symbol_overflow"),
                    )
                })?;
            } else {
                let vert_or_horz_group = table_usize(
                    "Size_To_Tx_Type_Group_Vert_Or_Horz",
                    &SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ,
                    mi_size,
                )
                .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
                if vert_or_horz_group > 0 {
                    let tx_2or3 = if work_unit.coeff_frame_facts().reduced_tx_set() != 0 {
                        0
                    } else {
                        read_tx_symbol(
                            work_unit,
                            symbols,
                            TileCdfSelector::Tx2Or3PartitionType {
                                fsc_mode: tx_fsc_mode,
                                is_inter: tx_is_inter,
                                ctx: vert_or_horz_group.checked_sub(1).ok_or_else(|| {
                                    selectable_decode_error(
                                        tile_offset,
                                        selectable_reason!("vert_or_horz_context_underflow"),
                                    )
                                })?,
                            },
                            tile_offset,
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
            for part in 0..4 {
                let tx = grid.set_tx_size(row, col, h4, w4, false, false)?;
                if part == 3 {
                    return Ok(tx);
                }
                row += h4;
            }
            Err(SelectableTransformRecordError::Unsupported {
                reason: "horz4-loop",
            })
        }
        TX_PARTITION_VERT4 => {
            w4 >>= 2;
            for part in 0..4 {
                let tx = grid.set_tx_size(row, col, h4, w4, false, false)?;
                if part == 3 {
                    return Ok(tx);
                }
                col += w4;
            }
            Err(SelectableTransformRecordError::Unsupported {
                reason: "vert4-loop",
            })
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

fn read_tx_symbol(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let value = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("symbol_read")))?;
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_TX_PARTITION") {
        eprintln!(
            "tx symbol selector={selector:?} value={value} checkpoint={:?}",
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

fn trace_tx_partition_for(row: usize, col: usize) -> bool {
    crate::trace_flags::trace_flag!("SPLOT_TRACE_TX_PARTITION")
        && ((row == 0 && (128..=144).contains(&col)) || (row == 16 && col == 336))
}

fn read_delta_q_abs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let delta_q_abs = read_tx_symbol(work_unit, symbols, TileCdfSelector::DeltaQ, tile_offset)?;
    if delta_q_abs != DELTA_Q_SMALL {
        return Ok(delta_q_abs);
    }
    let delta_q_rem_bits = read_literal_usize(symbols, DELTA_Q_REM_BITS_WIDTH, tile_offset)?
        .checked_add(1)
        .ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("delta_q_rem_bits_overflow"))
        })?;
    let delta_q_abs_bits = read_literal_usize(
        symbols,
        u32::try_from(delta_q_rem_bits).map_err(|_| {
            selectable_decode_error(tile_offset, selectable_reason!("delta_q_rem_bits_width"))
        })?,
        tile_offset,
    )?;
    let delta_q_large_base = 1usize
        .checked_shl(u32::try_from(delta_q_rem_bits).map_err(|_| {
            selectable_decode_error(tile_offset, selectable_reason!("delta_q_shift_width"))
        })?)
        .ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("delta_q_shift_overflow"))
        })?;
    delta_q_abs_bits
        .checked_add(delta_q_large_base)
        .and_then(|value| value.checked_add(DELTA_Q_SMALL - 2))
        .ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("delta_q_abs_overflow"))
        })
}

fn read_literal_usize(
    symbols: &mut SymbolDecoder<'_>,
    width: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let value = symbols
        .read_literal(width)
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("literal_read")))?;
    usize::try_from(value)
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("literal_width")))
}

fn updated_current_q_index(
    current_q_index: i64,
    reduced_delta_q_index: i64,
    delta_q_res: u8,
    max_q: i64,
) -> i64 {
    let scale = 1_i64 << delta_q_res;
    let delta = reduced_delta_q_index.saturating_mul(scale);
    current_q_index.saturating_add(delta).clamp(1, max_q)
}

fn intra_delta_q_sb_size4(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 32,
    })
}

fn block_dimension(
    table: &'static str,
    block_size: usize,
    width: bool,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let values = if width {
        &NUM_4X4_BLOCKS_WIDE
    } else {
        &NUM_4X4_BLOCKS_HIGH
    };
    let dimension = table_usize(table, values, block_size)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
    dimension.checked_mul(MI_SIZE).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("block_dimension_overflow"))
    })
}

fn tx_dimension(
    table: &'static str,
    values: &[i32],
    tx_size: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    tx_dimension_for_grid(table, values, tx_size)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))
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
