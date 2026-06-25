// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selectable transform-record handoff for the ac0ej3 Wiener NS LR frontier.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameHeaderParseStatus, TxMode};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE, SIZE_TO_TX_PART_GROUP_LOOKUP,
    SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ, SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_HEIGHT, TX_WIDTH,
};
use splot_recon::{BitDepth, max_quantizer_index};

use crate::error::Result;
use crate::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, DecodeBlockFrontier,
    DecodeTileWorkUnit, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    GeneralIntraLeafMode, IntraIstSyntax, LumaTransformTypeContext, TileCdfSelector,
    TileCoeffContextState, TransformToolResidualPolicy, decode_general_intra_block_modes,
    decode_general_intra_chroma_block_mode, decode_general_intra_luma_block_mode,
    decode_general_intra_multiblock_tree, decode_general_intra_plane_coeffs, frame_mi_dimensions,
};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use super::{
    WienerNsLrTransformRecordDiagnosticScope, derive_tile_plan,
    fixed_largest_420_chroma_tx_size_from_luma_4x4,
    map_wienerns_lr_transform_record_multiblock_error,
    wienerns_lr_live_transform_record_mode_error, wienerns_lr_live_transform_record_residual_error,
    wienerns_lr_selectable_transform_record_error_reason,
};

const BLOCK_4X4: usize = 0;
const BLOCK_64X64: usize = 12;
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
const DEFAULT_SEGMENT_ID: usize = 0;
const DELTA_Q_SMALL: usize = 7;
const DELTA_Q_REM_BITS_WIDTH: u32 = 3;
const DELTA_Q_SIGN_BIT_WIDTH: u32 = 1;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrLiveTransformRecordHandoff {
    pub(super) tx_skip_rows: usize,
    pub(super) tx_skip_cols: usize,
    pub(super) records: Vec<WienerNsLrTxSkipTransformRecord>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private tx-skip retention proof waits for live transform-record handoff"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct WienerNsLrTxSkipTransformRecord {
    pub(in crate::runtime_minimal) row: usize,
    pub(in crate::runtime_minimal) col: usize,
    pub(in crate::runtime_minimal) rows: usize,
    pub(in crate::runtime_minimal) cols: usize,
    pub(in crate::runtime_minimal) skip_flag: bool,
    pub(in crate::runtime_minimal) eob: usize,
    // TODO(spec: DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS): feed retained
    // IST syntax into the next transform-record residual parser frontier.
    pub(in crate::runtime_minimal) intra_ist: Option<IntraIstSyntax>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SelectableLumaTxRecord {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
    tx_size: usize,
    middle: bool,
    scan_order: bool,
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
struct DeltaQState {
    present: bool,
    delta_q_res: u8,
    sb_size4: usize,
    current_q_index: i64,
    max_q: i64,
    current_sb: Option<(usize, usize)>,
    read_deltas: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CdefState {
    rows: usize,
    cols: usize,
    values: Vec<Option<usize>>,
    sb_size4: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum SelectableTransformRecordError {
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
            Self::EmptyTransform { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_empty_transform"
            }
            Self::InvalidTxSize { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_invalid_tx_size"
            }
            Self::OutOfBounds { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_out_of_bounds"
            }
            Self::Overlap { .. } => "unsupported_wienerns_lr_selectable_transform_records_overlap",
            Self::Incomplete { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_incomplete_grid"
            }
            Self::TableIndex { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_table_index"
            }
            Self::TableValue { .. } => {
                "unsupported_wienerns_lr_selectable_transform_records_table_value"
            }
            Self::Unsupported { reason } => selectable_unsupported_reason(reason),
        }
    }
}

fn selectable_unsupported_reason(reason: &'static str) -> &'static str {
    match reason {
        "grid-size-overflow" => {
            "unsupported_wienerns_lr_selectable_transform_records_grid_size_overflow"
        }
        "tx-width-overflow" => {
            "unsupported_wienerns_lr_selectable_transform_records_tx_width_overflow"
        }
        "tx-height-overflow" => {
            "unsupported_wienerns_lr_selectable_transform_records_tx_height_overflow"
        }
        "record-allocation" => {
            "unsupported_wienerns_lr_selectable_transform_records_record_allocation"
        }
        "region-size-overflow" => {
            "unsupported_wienerns_lr_selectable_transform_records_region_size_overflow"
        }
        "grid-index-overflow" => {
            "unsupported_wienerns_lr_selectable_transform_records_grid_index_overflow"
        }
        "horz4-loop" => "unsupported_wienerns_lr_selectable_transform_records_horz4_loop",
        "vert4-loop" => "unsupported_wienerns_lr_selectable_transform_records_vert4_loop",
        "tx-partition-type" => {
            "unsupported_wienerns_lr_selectable_transform_records_tx_partition_type"
        }
        _ => "unsupported_wienerns_lr_selectable_transform_records_unsupported_branch",
    }
}

fn selectable_transform_record_error(
    error: SelectableTransformRecordError,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(tile_offset, error.unsupported_reason())
}

impl DeltaQState {
    fn new(
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
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_missing_quantization",
                )
            })?
            .base_q_idx;
        let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_bit_depth",
                )
            })?;
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

    fn read_for_block(
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
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_delta_q_sign_read",
                    )
                })? != 0;
                let delta_q_abs = i64::try_from(delta_q_abs).map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_delta_q_abs_width",
                    )
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
}

impl CdefState {
    fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let rows = mi_rows.div_ceil(CDEF_UNIT_MI);
        let cols = mi_cols.div_ceil(CDEF_UNIT_MI);
        let values_len = rows.checked_mul(cols).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_grid_overflow",
            )
        })?;
        Ok(Self {
            rows,
            cols,
            values: vec![None; values_len],
            sb_size4: intra_delta_q_sb_size4(sequence, tile_offset)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if frontier.is_chroma_part() {
            return Ok(());
        }
        let Some(cdef) = core.cdef_params.as_ref() else {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_missing_cdef_params",
            ));
        };
        if !cdef.cdef_frame_enable {
            return Ok(());
        }
        let strengths = cdef.cdef_strengths.ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_missing_cdef_strengths",
            )
        })? as usize;
        if !(1..=8).contains(&strengths) {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_strengths",
            ));
        }

        let unit_row = frontier.r / CDEF_UNIT_MI;
        let unit_col = frontier.c / CDEF_UNIT_MI;
        if unit_row >= self.rows || unit_col >= self.cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_bounds",
            ));
        }
        if self.value(unit_row, unit_col, tile_offset)?.is_some() {
            return Ok(());
        }

        let strength = if strengths == 1 {
            0
        } else {
            let ctx = self.cdef_index0_ctx_at(frontier.r, frontier.c, tile_offset)?;
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
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_cdef_index_overflow",
                    )
                })?
            }
        };
        self.fill_units(frontier.r, frontier.c, n4w, n4h, strength, tile_offset)
    }

    fn cdef_index0_ctx_at(
        &self,
        mi_row: usize,
        mi_col: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let unit_row = mi_row / CDEF_UNIT_MI;
        let unit_col = mi_col / CDEF_UNIT_MI;
        let mut ctx = 0usize;
        let mut cnt = 0usize;

        if unit_col > 0 {
            if self.value(unit_row, unit_col - 1, tile_offset)? == Some(0) {
                ctx += 1;
            }
            cnt += 1;
        }
        if unit_row > 0
            && unit_row / self.sb_size4_units() == (unit_row - 1) / self.sb_size4_units()
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
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_index_bounds",
            ));
        }
        row.checked_mul(self.cols)
            .and_then(|start| start.checked_add(col))
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_cdef_index_overflow",
                )
            })
    }

    const fn sb_size4_units(&self) -> usize {
        self.sb_size4 / CDEF_UNIT_MI
    }
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

        self.records
            .try_reserve(1)
            .map_err(|_| SelectableTransformRecordError::Unsupported {
                reason: "record-allocation",
            })?;
        for r in row..row.saturating_add(h4) {
            for c in col..col.saturating_add(w4) {
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
        for r in row..row + h4 {
            for c in col..col + w4 {
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
        let expected =
            rows.checked_mul(cols)
                .ok_or(SelectableTransformRecordError::Unsupported {
                    reason: "region-size-overflow",
                })?;
        let mut actual = 0usize;
        for r in row..row.saturating_add(rows) {
            for c in col..col.saturating_add(cols) {
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

#[allow(clippy::too_many_arguments)]
pub(super) fn derive_wienerns_lr_selectable_transform_record_handoff(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    key_candidate: &DecodePlannedObu,
    key_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<WienerNsLrLiveTransformRecordHandoff> {
    ensure_selectable_transform_record_tool_gates(sequence, core, key_envelope.offset)?;
    let mut tile_plan = derive_tile_plan(
        plan,
        key_candidate,
        bytes,
        key_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                key_envelope.offset,
                "unsupported_wienerns_lr_selectable_transform_records_no_tile",
            ));
        }
        work_units => {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                work_units
                    .first()
                    .map_or(key_envelope.offset, |tile| tile.tile_byte_span().start),
                "unsupported_wienerns_lr_selectable_transform_records_multi_tile",
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;
    let frame_facts = tile.coeff_frame_facts();
    if frame_facts.lossless_for_segment(DEFAULT_SEGMENT_ID) != Some(false) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_lossless",
        ));
    }

    let (tx_skip_rows, tx_skip_cols) = frame_mi_dimensions(core).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_frame_dimensions",
        )
    })?;
    let mut coeff_ctx = TileCoeffContextState::new(tx_skip_rows, tx_skip_cols).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_coeff_context",
        )
    })?;
    let mut records = Vec::new();
    let limits = options.limits();
    let chroma_tools = sequence
        .intra
        .as_ref()
        .map_or(GeneralIntraChromaToolConfig::disabled(), |intra| {
            GeneralIntraChromaToolConfig::new(intra.enable_cfl_intra, intra.enable_mhccp)
                .with_enable_mrls(intra.enable_mrls)
        })
        .with_enable_idtx_intra(
            sequence
                .transform_quant_entropy
                .as_ref()
                .is_some_and(|tq| tq.enable_idtx_intra),
        );
    let transform_tool_residual_policy = sequence.transform_quant_entropy.as_ref().map_or(
        TransformToolResidualPolicy::Allow,
        |tq| {
            if tq.enable_fsc
                || tq.enable_cctx
                || tq.enable_idtx_intra
                || tq.enable_intra_ist
                || tq.enable_inter_ist
            {
                TransformToolResidualPolicy::AdmitTransformToolSubset {
                    luma: None,
                    active_intra_ist: ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
                    active_chroma: ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
                }
            } else {
                TransformToolResidualPolicy::Allow
            }
        },
    );
    let mut delta_q_state = DeltaQState::new(sequence, core, tile_offset)?;
    let mut cdef_state = CdefState::new(tx_skip_rows, tx_skip_cols, sequence, tile_offset)?;

    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, joint_modes, uses_mrls, _block_decoded| {
            let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_block_width",
                )
            })?;
            let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_block_height",
                )
            })?;
            if frontier.chroma_offset
                && !selectable_chroma_offset_leaf_supported(
                    frontier.is_luma_part(),
                    frontier.has_chroma,
                )
            {
                return Err(wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_chroma_offset_leaf",
                ));
            }
            if !selectable_transform_leaf_shape_supported(
                frontier.is_luma_part(),
                frontier.has_chroma,
                n4w,
                n4h,
            ) {
                return Err(wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_block_shape",
                ));
            }
            if frontier.is_chroma_part() {
                let y_mode = frontier.stored_luma_y_mode().ok_or_else(|| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_missing_sdp_luma_mode",
                    )
                })?;
                let uv_mode = decode_general_intra_chroma_block_mode(
                    work_unit,
                    symbols,
                    chroma_tools,
                    GeneralIntraChromaModeContext::sdp_chroma_part(
                        frontier.cfl_allowed_in_sdp(),
                    ),
                    y_mode,
                    frontier.b_size.index(),
                    n4w,
                    n4h,
                )
                .map_err(|error| {
                    wienerns_lr_live_transform_record_mode_error(
                        error,
                        tile_offset,
                        WienerNsLrTransformRecordDiagnosticScope::Selectable,
                    )
                })?;
                decode_chroma_residual_chunks(
                    work_unit,
                    symbols,
                    &mut coeff_ctx,
                    frontier,
                    n4w,
                    n4h,
                    uv_mode.coeff_uv_mode(),
                    transform_tool_residual_policy,
                    tile_offset,
                )?;
                return Ok(GeneralIntraLeafMode::no_luma_mode());
            }
            read_luma_shared_mode_info_prelude(
                work_unit,
                symbols,
                core,
                frontier,
                n4w,
                n4h,
                &mut cdef_state,
                &mut delta_q_state,
                tile_offset,
            )?;
            let (uv_mode, leaf_mode, luma_transform_type_context) = if frontier.is_luma_part() {
                let luma = decode_general_intra_luma_block_mode(
                    work_unit,
                    symbols,
                    chroma_tools,
                    joint_modes,
                    uses_mrls,
                    frontier.b_size.index(),
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                )
                .map_err(|error| {
                    wienerns_lr_live_transform_record_mode_error(
                        error,
                        tile_offset,
                        WienerNsLrTransformRecordDiagnosticScope::Selectable,
                    )
                })?;
                (
                    0,
                    GeneralIntraLeafMode::luma(luma.intra_joint_mode, luma.y_mode, luma.uses_mrls),
                    LumaTransformTypeContext::with_mrl_index(
                        luma.y_mode,
                        luma.angle_delta_y,
                        luma.mrl_index,
                    ),
                )
            } else {
                let modes = decode_general_intra_block_modes(
                    work_unit,
                    symbols,
                    chroma_tools,
                    joint_modes,
                    uses_mrls,
                    frontier.b_size.index(),
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                )
                .map_err(|error| {
                    wienerns_lr_live_transform_record_mode_error(
                        error,
                        tile_offset,
                        WienerNsLrTransformRecordDiagnosticScope::Selectable,
                    )
                })?;
                (
                    modes.coeff_uv_mode(),
                    GeneralIntraLeafMode::luma(
                        modes.intra_joint_mode,
                        modes.y_mode,
                        modes.uses_mrls,
                    ),
                    LumaTransformTypeContext::with_mrl_index(
                        modes.y_mode,
                        modes.angle_delta_y,
                        modes.mrl_index,
                    ),
                )
            };

            let luma_records = derive_selectable_luma_tx_records_for_block(
                work_unit,
                symbols,
                frontier,
                tx_skip_rows,
                tx_skip_cols,
                tile_offset,
            )?;
            records.try_reserve(luma_records.len()).map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_output_allocation",
                )
            })?;
            decode_selectable_residual_chunks(
                work_unit,
                symbols,
                &mut coeff_ctx,
                frontier,
                n4w,
                n4h,
                uv_mode,
                &luma_records,
                &mut records,
                luma_transform_type_context,
                transform_tool_residual_policy,
                tile_offset,
            )?;
            Ok(leaf_mode)
        },
    )
    .map_err(|error| {
        map_wienerns_lr_transform_record_multiblock_error(
            error,
            tile_offset,
            WienerNsLrTransformRecordDiagnosticScope::Selectable,
        )
    })?;

    symbols.exit_symbol().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_exit_symbol",
        )
    })?;

    Ok(WienerNsLrLiveTransformRecordHandoff {
        tx_skip_rows,
        tx_skip_cols,
        records,
    })
}

fn derive_selectable_luma_tx_records_for_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    mi_rows: usize,
    mi_cols: usize,
    tile_offset: ByteOffset,
) -> Result<Vec<SelectableLumaTxRecord>> {
    let mut grid = SelectableLumaTxGrid::new(mi_rows, mi_cols)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
    read_tx_size_selectable(work_unit, symbols, frontier, &mut grid, tile_offset)?;
    let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_region_width",
        )
    })?;
    let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_region_height",
        )
    })?;
    grid.records_for_region(frontier.r, frontier.c, n4h, n4w)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))
}

#[allow(clippy::too_many_arguments)]
fn decode_selectable_residual_chunks(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    uv_mode: usize,
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    luma_transform_type_context: LumaTransformTypeContext,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let width_chunks = (n4w / 16).max(1);
    let height_chunks = (n4h / 16).max(1);
    let large_chunks = width_chunks > 1 || height_chunks > 1;
    let double_chroma_w = width_chunks > 1;
    let double_chroma_h = height_chunks > 1;

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    let chunk_row = chunk_origin(frontier.r, chunk_y, tile_offset)?;
                    let chunk_col = chunk_origin(frontier.c, chunk_x, tile_offset)?;
                    let chunk_rows = if large_chunks { 16 } else { n4h };
                    let chunk_cols = if large_chunks { 16 } else { n4w };
                    decode_luma_records_for_chunk(
                        work_unit,
                        symbols,
                        coeff_ctx,
                        uv_mode,
                        luma_records,
                        records,
                        chunk_row,
                        chunk_col,
                        chunk_rows,
                        chunk_cols,
                        n4h,
                        n4w,
                        luma_transform_type_context,
                        transform_tool_residual_policy,
                        tile_offset,
                    )?;

                    let at_start = (!double_chroma_w || chunk_x & 1 == 0)
                        && (!double_chroma_h || chunk_y & 1 == 0);
                    if frontier.has_chroma && at_start {
                        let group_chunks_w = if large_chunks && double_chroma_w {
                            (width_chunks - chunk_x).min(2)
                        } else {
                            1
                        };
                        let group_chunks_h = if large_chunks && double_chroma_h {
                            (height_chunks - chunk_y).min(2)
                        } else {
                            1
                        };
                        let chroma_luma_n4w = if large_chunks {
                            group_chunks_w.checked_mul(16).ok_or_else(|| {
                                wienerns_lr_selectable_transform_record_error_reason(
                                    tile_offset,
                                    "unsupported_wienerns_lr_selectable_transform_records_chroma_group_width_overflow",
                                )
                            })?
                        } else {
                            n4w
                        };
                        let chroma_luma_n4h = if large_chunks {
                            group_chunks_h.checked_mul(16).ok_or_else(|| {
                                wienerns_lr_selectable_transform_record_error_reason(
                                    tile_offset,
                                    "unsupported_wienerns_lr_selectable_transform_records_chroma_group_height_overflow",
                                )
                            })?
                        } else {
                            n4h
                        };
                        decode_chroma_group(
                            work_unit,
                            symbols,
                            coeff_ctx,
                            frontier,
                            chunk_x,
                            chunk_y,
                            chroma_luma_n4w,
                            chroma_luma_n4h,
                            uv_mode,
                            transform_tool_residual_policy,
                            tile_offset,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma_residual_chunks(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    uv_mode: usize,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let width_chunks = (n4w / 16).max(1);
    let height_chunks = (n4h / 16).max(1);
    let large_chunks = width_chunks > 1 || height_chunks > 1;
    let double_chroma_w = width_chunks > 1;
    let double_chroma_h = height_chunks > 1;

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    let at_start = (!double_chroma_w || chunk_x & 1 == 0)
                        && (!double_chroma_h || chunk_y & 1 == 0);
                    if !at_start {
                        continue;
                    }
                    let group_chunks_w = if large_chunks && double_chroma_w {
                        (width_chunks - chunk_x).min(2)
                    } else {
                        1
                    };
                    let group_chunks_h = if large_chunks && double_chroma_h {
                        (height_chunks - chunk_y).min(2)
                    } else {
                        1
                    };
                    let chroma_luma_n4w = if large_chunks {
                        group_chunks_w.checked_mul(16).ok_or_else(|| {
                            wienerns_lr_selectable_transform_record_error_reason(
                                tile_offset,
                                "unsupported_wienerns_lr_selectable_transform_records_chroma_group_width_overflow",
                            )
                        })?
                    } else {
                        n4w
                    };
                    let chroma_luma_n4h = if large_chunks {
                        group_chunks_h.checked_mul(16).ok_or_else(|| {
                            wienerns_lr_selectable_transform_record_error_reason(
                                tile_offset,
                                "unsupported_wienerns_lr_selectable_transform_records_chroma_group_height_overflow",
                            )
                        })?
                    } else {
                        n4h
                    };
                    decode_chroma_group(
                        work_unit,
                        symbols,
                        coeff_ctx,
                        frontier,
                        chunk_x,
                        chunk_y,
                        chroma_luma_n4w,
                        chroma_luma_n4h,
                        uv_mode,
                        transform_tool_residual_policy,
                        tile_offset,
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_luma_records_for_chunk(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    uv_mode: usize,
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    chunk_row: usize,
    chunk_col: usize,
    chunk_rows: usize,
    chunk_cols: usize,
    block_rows: usize,
    block_cols: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut decoded_any = false;
    let chunk_row_end = chunk_row.checked_add(chunk_rows).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chunk_row_end_overflow",
        )
    })?;
    let chunk_col_end = chunk_col.checked_add(chunk_cols).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chunk_col_end_overflow",
        )
    })?;
    for record in luma_records.iter().copied().filter(|record| {
        record.row >= chunk_row
            && record.col >= chunk_col
            && record.row < chunk_row_end
            && record.col < chunk_col_end
    }) {
        decoded_any = true;
        let residual_policy =
            luma_transform_tool_policy(transform_tool_residual_policy, luma_transform_type_context);
        // AV2 § 5.20.7.23 uses `miSizeChunk` only for residual traversal bounds;
        // § 8.3.2 still derives luma `all_zero` context from `MiSize`, the full
        // luma residual block. Compare the transform record with that full block
        // for `bw == w && bh == h`, while keeping § 5.20.7.27/§ 5.20.7.30
        // coefficient spans and scan length record-local.
        let luma = decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            0,
            record.tx_size,
            mi_to_sample(record.col, tile_offset)?,
            mi_to_sample(record.row, tile_offset)?,
            selectable_luma_tx_record_fills_block(record, block_rows, block_cols),
            false,
            uv_mode,
            false,
            residual_policy,
        )
        .map_err(|error| {
            wienerns_lr_live_transform_record_residual_error(
                error,
                tile_offset,
                WienerNsLrTransformRecordDiagnosticScope::Selectable,
            )
        })?;
        records.push(WienerNsLrTxSkipTransformRecord {
            row: record.row,
            col: record.col,
            rows: record.rows,
            cols: record.cols,
            skip_flag: luma.all_zero,
            eob: luma.eob,
            intra_ist: luma.intra_ist,
        });
    }
    if decoded_any {
        Ok(())
    } else {
        Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_empty_chunk",
        ))
    }
}

fn luma_transform_tool_policy(
    policy: TransformToolResidualPolicy,
    luma: LumaTransformTypeContext,
) -> TransformToolResidualPolicy {
    match policy {
        TransformToolResidualPolicy::Allow => TransformToolResidualPolicy::Allow,
        TransformToolResidualPolicy::AdmitTransformToolSubset {
            active_intra_ist,
            active_chroma,
            ..
        } => TransformToolResidualPolicy::AdmitTransformToolSubset {
            luma: Some(luma),
            active_intra_ist,
            active_chroma,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma_group(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    chunk_x: usize,
    chunk_y: usize,
    chroma_luma_n4w: usize,
    chroma_luma_n4h: usize,
    uv_mode: usize,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chroma_tx =
        fixed_largest_420_chroma_tx_size_from_luma_4x4(chroma_luma_n4w, chroma_luma_n4h)
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_chroma_tx_size",
                )
            })?;
    let chroma_x = chroma_420_sample(frontier.c, chunk_x, tile_offset)?;
    let chroma_y = chroma_420_sample(frontier.r, chunk_y, tile_offset)?;
    let u = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        1,
        chroma_tx,
        chroma_x,
        chroma_y,
        true,
        false,
        uv_mode,
        false,
        transform_tool_residual_policy,
    )
    .map_err(|error| {
        wienerns_lr_live_transform_record_residual_error(
            error,
            tile_offset,
            WienerNsLrTransformRecordDiagnosticScope::Selectable,
        )
    })?;
    let _v = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        2,
        chroma_tx,
        chroma_x,
        chroma_y,
        true,
        !u.all_zero,
        uv_mode,
        false,
        transform_tool_residual_policy,
    )
    .map_err(|error| {
        wienerns_lr_live_transform_record_residual_error(
            error,
            tile_offset,
            WienerNsLrTransformRecordDiagnosticScope::Selectable,
        )
    })?;
    Ok(())
}

fn chunk_origin(base: usize, chunk: usize, tile_offset: ByteOffset) -> Result<usize> {
    let chunk_offset = chunk.checked_mul(16).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chunk_origin_overflow",
        )
    })?;
    base.checked_add(chunk_offset).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chunk_origin_add_overflow",
        )
    })
}

fn mi_to_sample(mi: usize, tile_offset: ByteOffset) -> Result<usize> {
    mi.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_sample_coordinate_overflow",
        )
    })
}

fn chroma_420_sample(base_mi: usize, chunk: usize, tile_offset: ByteOffset) -> Result<usize> {
    let base = base_mi.checked_mul(2).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chroma_base_overflow",
        )
    })?;
    let chunk_luma_mi = chunk.checked_mul(16).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chroma_chunk_overflow",
        )
    })?;
    let chunk_chroma_samples = (chunk_luma_mi >> 1).checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chroma_offset_overflow",
        )
    })?;
    base.checked_add(chunk_chroma_samples).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_chroma_coordinate_overflow",
        )
    })
}

fn read_tx_size_selectable(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    grid: &mut SelectableLumaTxGrid,
    tile_offset: ByteOffset,
) -> Result<()> {
    let b_size = frontier.b_size.index();
    let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_4x4_width",
        )
    })?;
    let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_4x4_height",
        )
    })?;
    if b_size == BLOCK_4X4 {
        grid.set_tx_size(frontier.r, frontier.c, n4h, n4w, false, false)
            .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
        return Ok(());
    }
    let actual_extent = selectable_luma_leaf_uses_actual_extent(
        frontier.is_luma_part(),
        frontier.has_chroma,
        n4w,
        n4h,
    )
    .then_some((n4h, n4w));
    let max_tx_size = table_usize("Max_Tx_Size_Rect", &MAX_TX_SIZE_RECT, b_size)
        .map_err(|error| selectable_transform_record_error(error, tile_offset))?;

    let width = frontier.b_size.width_samples().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_sample_width",
        )
    })?;
    let height = frontier.b_size.height_samples().map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_sample_height",
        )
    })?;
    let width_chunks = width >> 6;
    let height_chunks = height >> 6;
    if width_chunks > 1 || height_chunks > 1 {
        for chunk_y in 0..height_chunks {
            for chunk_x in 0..width_chunks {
                let row = frontier.r + (chunk_y << 4);
                let col = frontier.c + (chunk_x << 4);
                grid.set_tx_size(row, col, 16, 16, false, false)
                    .map_err(|error| selectable_transform_record_error(error, tile_offset))?;
            }
        }
        return Ok(());
    }

    // AV2 §5.20.6.1 still consumes §5.20.6.3 partition syntax here; the
    // actual-extent fallback below only changes how empty narrow geometry is
    // represented for this syntax-only LR tx-skip handoff.
    let Some(tx_partition) = read_tx_partition_symbols(
        work_unit,
        symbols,
        grid,
        frontier.r,
        frontier.c,
        max_tx_size,
        b_size,
        tile_offset,
    )?
    else {
        return Ok(());
    };

    apply_tx_partition_or_actual_extent(
        grid,
        frontier.r,
        frontier.c,
        max_tx_size,
        tx_partition,
        actual_extent,
    )
    .map(|_| ())
    .map_err(|error| selectable_transform_record_error(error, tile_offset))
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
                fsc_mode: 0,
                is_inter: 0,
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
                        fsc_mode: 0,
                        is_inter: 0,
                        ctx,
                        reduced: false,
                    },
                    tile_offset,
                )?;
                tx_partition = symbol
                    .checked_add(1)
                    .ok_or_else(|| {
                        wienerns_lr_selectable_transform_record_error_reason(
                            tile_offset,
                            "unsupported_wienerns_lr_selectable_transform_records_partition_symbol_overflow",
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
                                fsc_mode: 0,
                                is_inter: 0,
                                ctx: vert_or_horz_group.checked_sub(1).ok_or_else(|| {
                                    wienerns_lr_selectable_transform_record_error_reason(
                                        tile_offset,
                                        "unsupported_wienerns_lr_selectable_transform_records_vert_or_horz_context_underflow",
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

#[allow(clippy::too_many_arguments)]
fn read_luma_shared_mode_info_prelude(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    cdef_state: &mut CdefState,
    delta_q_state: &mut DeltaQState,
    tile_offset: ByteOffset,
) -> Result<()> {
    read_use_intrabc_zero(work_unit, symbols, core, frontier, n4w, n4h, tile_offset)?;
    cdef_state.read_for_block(work_unit, symbols, core, frontier, n4w, n4h, tile_offset)?;
    delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)
}

fn read_use_intrabc_zero(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    if core.allow_intrabc != Some(true)
        || frontier.is_chroma_part()
        || n4w.saturating_mul(MI_SIZE) > 64
        || n4h.saturating_mul(MI_SIZE) > 64
        || frontier.b_size.index() == BLOCK_64X64
    {
        return Ok(());
    }
    let use_intrabc = read_tx_symbol(
        work_unit,
        symbols,
        TileCdfSelector::Intrabc { ctx: 0 },
        tile_offset,
    )?;
    if use_intrabc == 0 {
        Ok(())
    } else {
        Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc",
        ))
    }
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
                if part != 3 {
                    row += h4;
                } else {
                    return Ok(tx);
                }
            }
            Err(SelectableTransformRecordError::Unsupported {
                reason: "horz4-loop",
            })
        }
        TX_PARTITION_VERT4 => {
            w4 >>= 2;
            for part in 0..4 {
                let tx = grid.set_tx_size(row, col, h4, w4, false, false)?;
                if part != 3 {
                    col += w4;
                } else {
                    return Ok(tx);
                }
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

fn apply_tx_partition_or_actual_extent(
    grid: &mut SelectableLumaTxGrid,
    row: usize,
    col: usize,
    tx_size: usize,
    tx_partition: usize,
    actual_extent: Option<(usize, usize)>,
) -> std::result::Result<usize, SelectableTransformRecordError> {
    match apply_tx_partition(grid, row, col, tx_size, tx_partition) {
        Ok(tx_size) => Ok(tx_size),
        Err(error @ SelectableTransformRecordError::EmptyTransform { .. }) => {
            if let Some((h4, w4)) = actual_extent {
                grid.set_tx_size(row, col, h4, w4, false, false)
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

fn selectable_luma_tx_record_fills_block(
    record: SelectableLumaTxRecord,
    block_rows: usize,
    block_cols: usize,
) -> bool {
    record.rows == block_rows && record.cols == block_cols
}

fn read_tx_symbol(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_symbol_read",
            )
        })
}

fn ensure_selectable_transform_record_tool_gates(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    if sequence.general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
        return selectable_tool_gate_error(offset, "chroma_format");
    }
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_is_intra != Some(true)
        || !core.is_key_frame
    {
        return selectable_tool_gate_error(offset, "frame_type");
    }
    if core
        .tile_info
        .as_ref()
        .is_none_or(|tile_info| tile_info.tile_cols != 1 || tile_info.tile_rows != 1)
    {
        return selectable_tool_gate_error(offset, "tile_grid");
    }
    if core
        .intra_tail
        .is_none_or(|tail| tail.tx_mode != TxMode::Select || tail.film_grain.apply_grain)
    {
        return selectable_tool_gate_error(offset, "intra_tail");
    }
    let Some(intra) = sequence.intra.as_ref() else {
        return selectable_tool_gate_error(offset, "missing_intra_config");
    };
    if intra.enable_dip {
        return selectable_tool_gate_error(offset, "unsupported_intra_tool");
    }
    let Some(partition) = sequence.partition.as_ref() else {
        return selectable_tool_gate_error(offset, "missing_partition_config");
    };
    let _ = partition;
    let Some(_tq) = sequence.transform_quant_entropy.as_ref() else {
        return selectable_tool_gate_error(offset, "missing_transform_quant_entropy_config");
    };
    if core.allow_screen_content_tools != Some(false) || core.allow_intrabc.is_none() {
        return selectable_tool_gate_error(offset, "screen_content_tools");
    }
    if core
        .segmentation_params
        .as_ref()
        .is_none_or(|seg| seg.segmentation_enabled)
    {
        return selectable_tool_gate_error(offset, "segmentation");
    }
    if core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix) {
        return selectable_tool_gate_error(offset, "quant_matrix");
    }
    if core
        .lossless_info
        .as_ref()
        .is_none_or(|lossless| lossless.coded_lossless)
    {
        return selectable_tool_gate_error(offset, "lossless");
    }
    if core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable) {
        return selectable_tool_gate_error(offset, "gdf");
    }
    if core
        .cdef_params
        .as_ref()
        .is_none_or(|cdef| cdef.cdef_frame_enable && cdef.cdef_strengths.is_none())
    {
        return selectable_tool_gate_error(offset, "cdef");
    }
    Ok(())
}

fn selectable_tool_gate_error(offset: ByteOffset, tool: &'static str) -> Result<()> {
    let reason = match tool {
        "chroma_format" => "unsupported_wienerns_lr_selectable_transform_records_chroma_format",
        "frame_type" => "unsupported_wienerns_lr_selectable_transform_records_frame_type",
        "tile_grid" => "unsupported_wienerns_lr_selectable_transform_records_tile_grid",
        "intra_tail" => "unsupported_wienerns_lr_selectable_transform_records_intra_tail",
        "missing_intra_config" => {
            "unsupported_wienerns_lr_selectable_transform_records_missing_intra_config"
        }
        "unsupported_intra_tool" => {
            "unsupported_wienerns_lr_selectable_transform_records_unsupported_intra_tool"
        }
        "missing_partition_config" => {
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config"
        }
        "missing_transform_quant_entropy_config" => {
            "unsupported_wienerns_lr_selectable_transform_records_missing_transform_quant_entropy_config"
        }
        "unsupported_transform_tool" => {
            "unsupported_wienerns_lr_selectable_transform_records_unsupported_transform_tool"
        }
        "screen_content_tools" => {
            "unsupported_wienerns_lr_selectable_transform_records_screen_content_tools"
        }
        "segmentation" => "unsupported_wienerns_lr_selectable_transform_records_segmentation",
        "quant_matrix" => "unsupported_wienerns_lr_selectable_transform_records_quant_matrix",
        "lossless" => "unsupported_wienerns_lr_selectable_transform_records_lossless",
        "gdf" => "unsupported_wienerns_lr_selectable_transform_records_gdf",
        "cdef" => "unsupported_wienerns_lr_selectable_transform_records_cdef",
        "ccso" => "unsupported_wienerns_lr_selectable_transform_records_ccso",
        _ => "unsupported_wienerns_lr_selectable_transform_records_unsupported_tool",
    };
    Err(wienerns_lr_selectable_transform_record_error_reason(
        offset, reason,
    ))
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
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_delta_q_rem_bits_overflow",
            )
        })?;
    let delta_q_abs_bits = read_literal_usize(
        symbols,
        u32::try_from(delta_q_rem_bits).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_delta_q_rem_bits_width",
            )
        })?,
        tile_offset,
    )?;
    let delta_q_large_base = 1usize
        .checked_shl(u32::try_from(delta_q_rem_bits).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_delta_q_shift_width",
            )
        })?)
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_delta_q_shift_overflow",
            )
        })?;
    delta_q_abs_bits
        .checked_add(delta_q_large_base)
        .and_then(|value| value.checked_add(DELTA_Q_SMALL - 2))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_delta_q_abs_overflow",
            )
        })
}

fn read_literal_usize(
    symbols: &mut SymbolDecoder<'_>,
    width: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let value = symbols.read_literal(width).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_literal_read",
        )
    })?;
    usize::try_from(value).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_literal_width",
        )
    })
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
    let partition = sequence.partition.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config",
        )
    })?;
    // AV2 § 5.18.2 caps intra frames with 256x256 sequence superblocks to
    // 128x128 before tile partition traversal and `ReadDeltas` reset.
    Ok(match partition.seq_sb_size() {
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
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_block_dimension_overflow",
        )
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

fn selectable_transform_leaf_shape_supported(
    is_luma_part: bool,
    has_chroma: bool,
    n4w: usize,
    n4h: usize,
) -> bool {
    if n4w == 0 || n4h == 0 {
        return false;
    }
    if n4w < 2 || n4h < 2 {
        return is_luma_part && !has_chroma;
    }
    is_luma_part || has_chroma
}

fn selectable_luma_leaf_uses_actual_extent(
    is_luma_part: bool,
    has_chroma: bool,
    n4w: usize,
    n4h: usize,
) -> bool {
    is_luma_part && !has_chroma && matches!((n4w, n4h), (1, 8) | (2, 8))
}

fn selectable_chroma_offset_leaf_supported(is_luma_part: bool, has_chroma: bool) -> bool {
    is_luma_part && !has_chroma
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_actual_extent_tests.rs"]
mod actual_extent_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::headers::frame::{
        build_minimal_intra_clk_core, build_minimal_intra_sequence_header,
    };

    use crate::error::DecodeError;

    use super::super::derive_wienerns_lr_tx_skip_grid_retention;
    use super::*;

    const TX_32X32: usize = 3;
    const TX_64X64: usize = 4;
    const TX_8X32: usize = 15;
    const TX_4X32: usize = 19;

    fn cdef_state(rows: usize, cols: usize, sb_size4: usize) -> CdefState {
        CdefState {
            rows,
            cols,
            values: vec![None; rows * cols],
            sb_size4,
        }
    }

    fn selectable_tool_gate_fixture() -> (SequenceHeader, FrameHeaderCore) {
        let mut sequence = build_minimal_intra_sequence_header().unwrap();
        let (mut core, _) = build_minimal_intra_clk_core().unwrap();
        if let Some(intra) = sequence.intra.as_mut() {
            intra.enable_dip = false;
            intra.enable_ibp = false;
            intra.enable_mrls = false;
            intra.enable_intra_edge_filter = false;
        }
        if let Some(tq) = sequence.transform_quant_entropy.as_mut() {
            tq.enable_fsc = false;
            tq.enable_cctx = false;
            tq.enable_idtx_intra = false;
            tq.enable_intra_ist = false;
            tq.enable_chroma_dctonly = false;
        }
        core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Select;
        (sequence, core)
    }

    fn unsupported_reason(error: DecodeError) -> &'static str {
        match error {
            DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
            other => panic!("unexpected decode error: {other:?}"),
        }
    }

    #[test]
    fn selectable_tool_gate_admits_minimal_inert_selectable_intra_header() {
        let (sequence, core) = selectable_tool_gate_fixture();

        ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(0))
            .unwrap();
    }

    #[test]
    fn selectable_tool_gate_rejects_unsupported_intra_tools_before_tile_decode() {
        let (mut sequence, core) = selectable_tool_gate_fixture();
        sequence.intra.as_mut().unwrap().enable_dip = true;

        let error =
            ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(74))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_unsupported_intra_tool"
        );
    }

    #[test]
    fn selectable_tool_gate_admits_enabled_inactive_transform_and_mrl_tools() {
        let (mut sequence, mut core) = selectable_tool_gate_fixture();
        sequence.intra.as_mut().unwrap().enable_mrls = true;
        sequence.intra.as_mut().unwrap().enable_intra_edge_filter = true;
        sequence.intra.as_mut().unwrap().enable_ibp = true;
        let tq = sequence.transform_quant_entropy.as_mut().unwrap();
        tq.enable_fsc = true;
        tq.enable_cctx = true;
        tq.enable_idtx_intra = true;
        tq.enable_intra_ist = true;
        tq.enable_chroma_dctonly = true;
        core.ccso_params.as_mut().unwrap().ccso_frame_flag = Some(true);

        ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(0))
            .unwrap();
    }

    #[test]
    fn cdef_index0_context_uses_zero_strength_neighbours_in_same_superblock() {
        let offset = ByteOffset::new(0);
        let mut state = cdef_state(4, 4, 32);

        assert_eq!(state.cdef_index0_ctx_at(0, 0, offset).unwrap(), 0);

        let left = state.index(0, 0, offset).unwrap();
        state.values[left] = Some(0);
        assert_eq!(state.cdef_index0_ctx_at(0, 16, offset).unwrap(), 2);

        let above = state.index(0, 1, offset).unwrap();
        state.values[above] = Some(0);
        let left = state.index(1, 0, offset).unwrap();
        state.values[left] = Some(0);
        assert_eq!(state.cdef_index0_ctx_at(16, 16, offset).unwrap(), 3);

        state.values[left] = Some(2);
        assert_eq!(state.cdef_index0_ctx_at(16, 16, offset).unwrap(), 1);

        let mut state = cdef_state(4, 4, 32);
        let above = state.index(1, 1, offset).unwrap();
        state.values[above] = Some(0);
        assert_eq!(state.cdef_index0_ctx_at(32, 16, offset).unwrap(), 0);
    }

    #[test]
    fn cdef_fill_units_uses_cdef_aligned_origin_and_block_extent() {
        let offset = ByteOffset::new(0);
        let mut state = cdef_state(4, 4, 32);

        state.fill_units(8, 8, 32, 32, 5, offset).unwrap();

        assert_eq!(state.value(0, 0, offset).unwrap(), Some(5));
        assert_eq!(state.value(0, 1, offset).unwrap(), Some(5));
        assert_eq!(state.value(1, 0, offset).unwrap(), Some(5));
        assert_eq!(state.value(1, 1, offset).unwrap(), Some(5));
        assert_eq!(state.value(2, 0, offset).unwrap(), None);
        assert_eq!(state.value(0, 2, offset).unwrap(), None);
    }

    #[test]
    fn selectable_tx_grid_records_middle_and_scan_order_flags() {
        let mut grid = SelectableLumaTxGrid::new(8, 8).unwrap();
        apply_tx_partition(&mut grid, 0, 0, TX_32X32, TX_PARTITION_VERT5).unwrap();

        let records = grid.records_for_region(0, 0, 8, 8).unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(
            records
                .iter()
                .map(|record| (record.row, record.col, record.rows, record.cols))
                .collect::<Vec<_>>(),
            vec![
                (0, 0, 4, 2),
                (4, 0, 4, 2),
                (0, 2, 8, 4),
                (0, 6, 4, 2),
                (4, 6, 4, 2)
            ]
        );
        assert!(!records[0].middle);
        assert!(records[1..].iter().all(|record| record.middle));
        assert!(records.iter().all(|record| record.scan_order));
    }

    #[test]
    fn selectable_luma_record_fill_context_tracks_full_block_extent() {
        let full_record = SelectableLumaTxRecord {
            row: 0,
            col: 0,
            rows: 16,
            cols: 16,
            tx_size: TX_64X64,
            middle: false,
            scan_order: false,
        };
        assert!(selectable_luma_tx_record_fills_block(full_record, 16, 16));
        assert!(
            !selectable_luma_tx_record_fills_block(full_record, 32, 32),
            "a 64x64 transform record inside a 128x128 luma block must not take the §8.3.2 ctx=0 full-block branch"
        );

        let mut grid = SelectableLumaTxGrid::new(16, 16).unwrap();
        apply_tx_partition(&mut grid, 0, 0, TX_64X64, TX_PARTITION_HORZ5).unwrap();

        let records = grid.records_for_region(0, 0, 16, 16).unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| (record.row, record.col, record.rows, record.cols))
                .collect::<Vec<_>>(),
            vec![
                (0, 0, 4, 8),
                (0, 8, 4, 8),
                (4, 0, 8, 16),
                (12, 0, 4, 8),
                (12, 8, 4, 8),
            ]
        );
        assert!(
            records
                .iter()
                .copied()
                .all(|record| !selectable_luma_tx_record_fills_block(record, 16, 16))
        );
    }

    #[test]
    fn selectable_leaf_shape_admits_luma_only_block_4x32() {
        assert!(selectable_transform_leaf_shape_supported(true, false, 1, 8));
        assert!(selectable_transform_leaf_shape_supported(true, false, 8, 1));
        assert!(selectable_transform_leaf_shape_supported(true, false, 1, 1));
    }

    #[test]
    fn selectable_leaf_shape_preserves_chroma_bearing_narrow_guard() {
        assert!(!selectable_transform_leaf_shape_supported(
            false, true, 1, 8
        ));
        assert!(!selectable_transform_leaf_shape_supported(true, true, 1, 8));
        assert!(!selectable_transform_leaf_shape_supported(
            false, false, 1, 8
        ));
        assert!(!selectable_transform_leaf_shape_supported(
            true, false, 0, 8
        ));
        assert!(!selectable_transform_leaf_shape_supported(
            true, false, 1, 0
        ));
        assert!(selectable_transform_leaf_shape_supported(false, true, 2, 2));
    }

    #[test]
    fn selectable_chroma_offset_leaf_support_is_luma_only() {
        assert!(selectable_chroma_offset_leaf_supported(true, false));
        assert!(!selectable_chroma_offset_leaf_supported(true, true));
        assert!(!selectable_chroma_offset_leaf_supported(false, false));
        assert!(!selectable_chroma_offset_leaf_supported(false, true));
    }

    #[test]
    fn selectable_tx_grid_records_observed_luma_only_block_8x32_region() {
        let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();
        grid.set_tx_size(8, 24, 8, 2, false, false).unwrap();

        let records = grid.records_for_region(8, 24, 8, 2).unwrap();
        assert_eq!(
            records,
            vec![SelectableLumaTxRecord {
                row: 8,
                col: 24,
                rows: 8,
                cols: 2,
                tx_size: TX_8X32,
                middle: false,
                scan_order: false,
            }]
        );
    }

    #[test]
    fn selectable_tx_grid_records_luma_only_block_4x32_region() {
        let mut grid = SelectableLumaTxGrid::new(16, 32).unwrap();
        grid.set_tx_size(8, 24, 8, 1, false, false).unwrap();

        let records = grid.records_for_region(8, 24, 8, 1).unwrap();
        assert_eq!(
            records,
            vec![SelectableLumaTxRecord {
                row: 8,
                col: 24,
                rows: 8,
                cols: 1,
                tx_size: TX_4X32,
                middle: false,
                scan_order: false,
            }]
        );
        assert_eq!(
            grid.records_for_region(8, 24, 8, 2).unwrap_err(),
            SelectableTransformRecordError::Incomplete {
                expected: 16,
                actual: 8,
            }
        );
    }

    #[test]
    fn selectable_tx_grid_rejects_empty_transform_dimensions() {
        let mut grid = SelectableLumaTxGrid::new(4, 4).unwrap();

        assert_eq!(
            grid.set_tx_size(0, 0, 4, 0, false, false).unwrap_err(),
            SelectableTransformRecordError::EmptyTransform { h4: 4, w4: 0 }
        );
        assert_eq!(
            grid.set_tx_size(0, 0, 0, 4, false, false).unwrap_err(),
            SelectableTransformRecordError::EmptyTransform { h4: 0, w4: 4 }
        );
    }

    #[test]
    fn selectable_tx_grid_rejects_incomplete_region() {
        let mut grid = SelectableLumaTxGrid::new(4, 4).unwrap();
        grid.set_tx_size(0, 0, 2, 2, false, false).unwrap();

        assert_eq!(
            grid.records_for_region(0, 0, 4, 4).unwrap_err(),
            SelectableTransformRecordError::Incomplete {
                expected: 16,
                actual: 4,
            }
        );
    }

    #[test]
    fn tx_skip_grid_retention_preserves_skip_flag_for_nonzero_eob_record() {
        let records = [
            WienerNsLrTxSkipTransformRecord {
                row: 0,
                col: 0,
                rows: 1,
                cols: 1,
                skip_flag: true,
                eob: 3,
                intra_ist: None,
            },
            WienerNsLrTxSkipTransformRecord {
                row: 0,
                col: 1,
                rows: 1,
                cols: 1,
                skip_flag: false,
                eob: 3,
                intra_ist: None,
            },
        ];

        let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

        assert_eq!(
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup {
                    x: 0,
                    y: 0,
                    row: 0,
                    col: 0
                })
                .unwrap(),
            1
        );
        assert_eq!(
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup {
                    x: 0,
                    y: 0,
                    row: 0,
                    col: 1
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn selectable_tx_records_populate_live_tx_skip_grid() {
        let mut grid = SelectableLumaTxGrid::new(8, 8).unwrap();
        apply_tx_partition(&mut grid, 0, 0, TX_32X32, TX_PARTITION_SPLIT).unwrap();
        let records = grid.records_for_region(0, 0, 8, 8).unwrap();
        let tx_skip_records = records
            .iter()
            .enumerate()
            .map(|(index, record)| WienerNsLrTxSkipTransformRecord {
                row: record.row,
                col: record.col,
                rows: record.rows,
                cols: record.cols,
                skip_flag: false,
                eob: usize::from(index == 0),
                intra_ist: None,
            })
            .collect::<Vec<_>>();

        let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(8, 8, &tx_skip_records).unwrap();
        assert_eq!(
            (0..8)
                .map(|row| tx_skip
                    .lookup(super::super::WienerNsLrTxSkipLookup {
                        x: 0,
                        y: 0,
                        row,
                        col: 0
                    })
                    .unwrap())
                .collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 1, 1]
        );
    }
}
