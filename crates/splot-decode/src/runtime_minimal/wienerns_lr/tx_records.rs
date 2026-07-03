// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE, SIZE_TO_TX_PART_GROUP_LOOKUP,
    SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ, SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_HEIGHT, TX_WIDTH,
};
use splot_recon::{BitDepth, PlaneId, max_quantizer_index};

use crate::error::Result;
use crate::runtime_minimal::ccso::CcsoUnitGrid;
use crate::runtime_minimal::cdef::CdefUnitGrid;
use crate::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, DecodeBlockFrontier,
    DecodeTileWorkUnit, FrameCdfSubset, GeneralIntraBlockModeError, GeneralIntraChromaModeContext,
    GeneralIntraChromaToolConfig, GeneralIntraLeafMode, GeneralIntraMultiblockError,
    GeneralIntraResidualError, IntraIstSyntax, IntraYMode, LumaCoeffBlock,
    LumaTransformTypeContext, TileBlockDecodedState, TileCdfSelector, TileCoeffContextState,
    TransformToolResidualPolicy, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
    decode_general_intra_block_modes_with_fsc_context, decode_general_intra_chroma_block_mode,
    decode_general_intra_luma_block_mode_with_fsc_context,
    decode_general_intra_multiblock_tree_with_lr_source_blocks, decode_general_intra_plane_coeffs,
    frame_mi_dimensions, read_general_intra_palette_y_mode, supported_chroma_mode,
};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use super::super::effective_allow_screen_content_tools;
use super::recon::{SelectableReconContext, WienerNsLrReconSink};

use super::intrabc_records::{
    IntrabcBlockGeometry, IntrabcBlockPrelude, TileIntrabcPreludeState, read_intrabc_info,
    read_intrabc_use_and_skip,
};
use super::{
    WienerNsLrTransformRecordDiagnosticScope, derive_tile_plan,
    fixed_largest_420_chroma_tx_size_from_luma_4x4, intra_capped_seq_sb_size,
    map_wienerns_lr_transform_record_multiblock_error,
    wienerns_lr_live_transform_record_mode_error, wienerns_lr_live_transform_record_residual_error,
    wienerns_lr_selectable_transform_record_error_reason,
};

pub(in crate::runtime_minimal) mod ccso;
mod max_rect;
mod skip_records;

use ccso::CcsoState;

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
const DEFAULT_SEGMENT_ID: usize = 0;
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

mod tool_gate;

use tool_gate::ensure_selectable_transform_record_tool_gates;

type SelectableTransformGridSize = (usize, usize);

const SELECTABLE_DIAGNOSTIC_SCOPE: WienerNsLrTransformRecordDiagnosticScope =
    WienerNsLrTransformRecordDiagnosticScope::Selectable;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectableTxSizeContext {
    grid_size: SelectableTransformGridSize,
    fsc_mode: u8,
    is_inter: bool,
    skip_flag: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Block4x4Extent {
    cols: usize,
    rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResidualDecodeContext {
    uv_mode: usize,
    angle_delta_uv: i32,
    is_inter: bool,
    fsc_mode: u8,
    tool_policy: TransformToolResidualPolicy,
}

impl ResidualDecodeContext {
    fn luma_policy(self, luma: LumaTransformTypeContext) -> TransformToolResidualPolicy {
        match self.tool_policy {
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
}

fn chroma_angle_delta_uv(y_mode: IntraYMode, uv_mode: usize, angle_delta_y: i8) -> i32 {
    if uv_mode == y_mode.value() {
        i32::from(angle_delta_y)
    } else {
        0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrLiveTransformRecordHandoff {
    pub(super) tx_skip_rows: usize,
    pub(super) tx_skip_cols: usize,
    pub(super) records: Vec<WienerNsLrTxSkipTransformRecord>,
    pub(super) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(super) unit_filters: Vec<WienerNsLrUnitFilter>,
    pub(super) frame_cdfs: FrameCdfSubset,
    pub(in crate::runtime_minimal) cdef_grid: Option<CdefUnitGrid>,
    pub(in crate::runtime_minimal) ccso_grid: Option<CcsoUnitGrid>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "crate-visible handoff record crosses runtime_minimal module boundary"
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
    pub(in crate::runtime_minimal) intra_ist: Option<IntraIstSyntax>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct SelectableLumaTxRecord {
    pub(in crate::runtime_minimal) row: usize,
    pub(in crate::runtime_minimal) col: usize,
    pub(in crate::runtime_minimal) rows: usize,
    pub(in crate::runtime_minimal) cols: usize,
    pub(in crate::runtime_minimal) tx_size: usize,
    pub(in crate::runtime_minimal) middle: bool,
    pub(in crate::runtime_minimal) scan_order: bool,
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
pub(in crate::runtime_minimal) struct DeltaQState {
    present: bool,
    delta_q_res: u8,
    sb_size4: usize,
    current_q_index: i64,
    max_q: i64,
    current_sb: Option<(usize, usize)>,
    read_deltas: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct CdefState {
    rows: usize,
    cols: usize,
    values: Vec<Option<usize>>,
    sb_size4: usize,
}

pub(super) const CCSO_PLANES: usize = 3;
pub(super) const CCSO_SYMBOL_VALUES: usize = 2;
pub(super) const MI_SIZE_LOG2: u32 = 2;

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

fn selectable_mode_error_at(
    tile_offset: ByteOffset,
) -> impl FnOnce(GeneralIntraBlockModeError) -> crate::error::DecodeError {
    move |error| {
        wienerns_lr_live_transform_record_mode_error(
            error,
            tile_offset,
            SELECTABLE_DIAGNOSTIC_SCOPE,
        )
    }
}

fn selectable_residual_error_at(
    tile_offset: ByteOffset,
) -> impl FnOnce(GeneralIntraResidualError) -> crate::error::DecodeError {
    move |error| {
        if std::env::var_os("SPLOT_TRACE_SELECTABLE_RESIDUAL_ERROR").is_some() {
            eprintln!(
                "selectable residual error offset={}: {error:?}",
                tile_offset.get()
            );
        }
        wienerns_lr_live_transform_record_residual_error(
            error,
            tile_offset,
            SELECTABLE_DIAGNOSTIC_SCOPE,
        )
    }
}

fn selectable_multiblock_error_at(
    tile_offset: ByteOffset,
) -> impl FnOnce(GeneralIntraMultiblockError<crate::error::DecodeError>) -> crate::error::DecodeError
{
    move |error| {
        map_wienerns_lr_transform_record_multiblock_error(
            error,
            tile_offset,
            SELECTABLE_DIAGNOSTIC_SCOPE,
        )
    }
}

impl DeltaQState {
    pub(in crate::runtime_minimal) fn new(
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

    pub(in crate::runtime_minimal) fn read_for_block(
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

    pub(in crate::runtime_minimal) fn qindex_u32(&self) -> u32 {
        u32::try_from(self.current_q_index.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
    }
}

impl CdefState {
    pub(in crate::runtime_minimal) fn new(
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
    pub(in crate::runtime_minimal) fn read_for_block(
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

    const fn sb_size4_units(&self) -> usize {
        self.sb_size4 / CDEF_UNIT_MI
    }

    pub(in crate::runtime_minimal) fn into_grid(
        self,
        tile_offset: ByteOffset,
    ) -> Result<CdefUnitGrid> {
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

#[allow(clippy::too_many_arguments)]
pub(super) fn derive_wienerns_lr_selectable_transform_record_handoff(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    key_candidate: &DecodePlannedObu,
    key_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    mut sink: Option<&mut WienerNsLrReconSink<u16>>,
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
            return Err(selectable_decode_error(
                key_envelope.offset,
                selectable_reason!("no_tile"),
            ));
        }
        work_units => {
            return Err(selectable_decode_error(
                work_units
                    .first()
                    .map_or(key_envelope.offset, |tile| tile.tile_byte_span().start),
                selectable_reason!("multi_tile"),
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;
    let frame_facts = tile.coeff_frame_facts();
    if frame_facts.lossless_for_segment(DEFAULT_SEGMENT_ID) != Some(false) {
        return Err(selectable_decode_error(
            tile_offset,
            selectable_reason!("lossless"),
        ));
    }
    let luma_use_tcq = frame_facts.allow_tcq();

    let (tx_skip_rows, tx_skip_cols) = frame_mi_dimensions(core).map_err(|_| {
        selectable_decode_error(tile_offset, selectable_reason!("frame_dimensions"))
    })?;
    let mut coeff_ctx = TileCoeffContextState::new(tx_skip_rows, tx_skip_cols)
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("coeff_context")))?;
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
        )
        .with_allow_screen_content_tools(effective_allow_screen_content_tools(core));
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
    let mut ccso_state = CcsoState::new(tx_skip_rows, tx_skip_cols, sequence, core, tile_offset)?;
    let mut intrabc_state =
        TileIntrabcPreludeState::new(tx_skip_rows, tx_skip_cols, sequence, tile_offset)?;

    let tree_output = decode_general_intra_multiblock_tree_with_lr_source_blocks(
        tile,
        sequence,
        core,
        limits,
        |work_unit,
         symbols,
         frontier,
         joint_modes,
         uses_mrls,
         fsc_modes,
         palette_state,
         is_cfl_ctx,
         block_decoded| {
            let block_extent = frontier_4x4_extent(
                frontier,
                tile_offset,
                selectable_reason!("block_width"),
                selectable_reason!("block_height"),
            )?;
            let (n4w, n4h) = (block_extent.cols, block_extent.rows);
            if frontier.chroma_offset
                && !selectable_chroma_offset_leaf_supported(
                    frontier.is_luma_part(),
                    frontier.has_chroma,
                )
            {
                return Err(selectable_decode_error(
                    tile_offset,
                    selectable_reason!("chroma_offset_leaf"),
                ));
            }
            if !selectable_transform_leaf_shape_supported(
                frontier.is_luma_part(),
                frontier.has_chroma,
                block_extent.cols,
                block_extent.rows,
            ) {
                return Err(selectable_decode_error(
                    tile_offset,
                    selectable_reason!("block_shape"),
                ));
            }
            if frontier.is_chroma_part() {
                let y_mode = frontier.stored_luma_y_mode().ok_or_else(|| {
                    selectable_decode_error(
                        tile_offset,
                        selectable_reason!("missing_sdp_luma_mode"),
                    )
                })?;
                let angle_delta_y = frontier.stored_luma_angle_delta_y().ok_or_else(|| {
                    selectable_decode_error(
                        tile_offset,
                        selectable_reason!("missing_sdp_luma_angle_delta"),
                    )
                })?;
                let uv_mode = decode_general_intra_chroma_block_mode(
                    work_unit,
                    symbols,
                    chroma_tools,
                    GeneralIntraChromaModeContext::sdp_chroma_part(
                        frontier.cfl_allowed_in_sdp(),
                        is_cfl_ctx.get(),
                    ),
                    y_mode,
                    frontier.b_size.index(),
                    n4w,
                    n4h,
                )
                .map_err(selectable_mode_error_at(tile_offset))?;
                let sdp_recon = SelectableReconContext {
                    leaf_y_mode: Some(y_mode),
                    directional_luma: None,
                    mrl_index: 0,
                    mrl_sec_index: None,
                    angle_delta_y,
                    chroma_mode: supported_chroma_mode(y_mode, uv_mode.uv_mode()),
                    cfl_params: uv_mode.cfl_params(),
                    qindex: delta_q_state.qindex_u32(),
                    luma_use_tcq,
                    fsc_mode: false,
                    is_intrabc: false,
                };
                decode_chroma_residual_chunks(
                    work_unit,
                    symbols,
                    &mut coeff_ctx,
                    frontier,
                    block_extent,
                    block_decoded,
                    ResidualDecodeContext {
                        uv_mode: uv_mode.coeff_uv_mode(),
                        angle_delta_uv: chroma_angle_delta_uv(
                            y_mode,
                            uv_mode.coeff_uv_mode(),
                            angle_delta_y,
                        ),
                        is_inter: false,
                        fsc_mode: 0,
                        tool_policy: transform_tool_residual_policy,
                    },
                    sink.as_deref_mut(),
                    sdp_recon,
                    tile_offset,
                )?;
                return Ok(GeneralIntraLeafMode::chroma(uv_mode.is_cfl()));
            }
            let prelude = read_luma_shared_mode_info_prelude(
                work_unit,
                symbols,
                sequence,
                core,
                frontier,
                n4w,
                n4h,
                &mut cdef_state,
                &mut ccso_state,
                &mut delta_q_state,
                &mut intrabc_state,
                sink.as_deref_mut(),
                tile_offset,
            )?;
            let (
                uv_mode,
                leaf_mode,
                luma_transform_type_context,
                fsc_mode,
                chroma_mode,
                cfl_params,
                directional_luma,
            ) = if prelude.use_intrabc {
                (
                    0,
                    GeneralIntraLeafMode::luma(0, IntraYMode::DC_PRED, 0, 0, 0),
                    LumaTransformTypeContext::with_mrl_index(IntraYMode::DC_PRED, 0, 0),
                    0,
                    None,
                    None,
                    None,
                )
            } else if frontier.is_luma_part() {
                let use_neighbor_fsc_context =
                    core.frame_is_intra == Some(true) || !frontier.is_mixed_region();
                let luma = decode_general_intra_luma_block_mode_with_fsc_context(
                    work_unit,
                    symbols,
                    chroma_tools,
                    joint_modes,
                    uses_mrls,
                    fsc_modes,
                    use_neighbor_fsc_context,
                    frontier.b_size.index(),
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                )
                .map_err(selectable_mode_error_at(tile_offset))?;
                let palette_y = read_general_intra_palette_y_mode(
                    work_unit,
                    symbols,
                    chroma_tools,
                    palette_state,
                    luma.y_mode,
                    frontier.b_size.index(),
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                    bit_depth_bits(sequence),
                )
                .map_err(selectable_mode_error_at(tile_offset))?;
                (
                    0,
                    GeneralIntraLeafMode::luma(
                        luma.intra_joint_mode,
                        luma.y_mode,
                        luma.angle_delta_y,
                        luma.fsc_mode,
                        luma.uses_mrls,
                    )
                    .with_palette_y(palette_y),
                    LumaTransformTypeContext::with_mrl_indices(
                        luma.y_mode,
                        luma.angle_delta_y,
                        luma.mrl_index,
                        luma.mrl_sec_index,
                    ),
                    luma.fsc_mode,
                    None,
                    None,
                    luma.supported_directional_luma(),
                )
            } else {
                let use_neighbor_fsc_context =
                    core.frame_is_intra == Some(true) || !frontier.is_mixed_region();
                let modes = decode_general_intra_block_modes_with_fsc_context(
                    work_unit,
                    symbols,
                    chroma_tools,
                    joint_modes,
                    uses_mrls,
                    fsc_modes,
                    use_neighbor_fsc_context,
                    palette_state,
                    is_cfl_ctx.get(),
                    frontier.b_size.index(),
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                    frontier.b_size.index(),
                    n4w,
                    n4h,
                    bit_depth_bits(sequence),
                )
                .map_err(selectable_mode_error_at(tile_offset))?;
                let chroma_mode = modes.supported_chroma_mode();
                (
                    modes.coeff_uv_mode(),
                    GeneralIntraLeafMode::luma(
                        modes.intra_joint_mode,
                        modes.y_mode,
                        modes.angle_delta_y,
                        modes.fsc_mode,
                        modes.uses_mrls,
                    )
                    .with_palette_y(modes.palette_y())
                    .with_uv_cfl(modes.is_cfl()),
                    LumaTransformTypeContext::with_mrl_indices(
                        modes.y_mode,
                        modes.angle_delta_y,
                        modes.mrl_index,
                        modes.mrl_sec_index,
                    ),
                    modes.fsc_mode,
                    chroma_mode,
                    modes.cfl_params(),
                    modes.supported_directional_luma(),
                )
            };

            let luma_records = derive_selectable_luma_tx_records_for_block(
                work_unit,
                symbols,
                frontier,
                SelectableTxSizeContext {
                    grid_size: (tx_skip_rows, tx_skip_cols),
                    fsc_mode,
                    is_inter: prelude.is_inter,
                    skip_flag: prelude.skip_flag,
                },
                tile_offset,
            )?;
            records.try_reserve(luma_records.len()).map_err(|_| {
                selectable_decode_error(tile_offset, selectable_reason!("output_allocation"))
            })?;
            if let Some(sink) = sink.as_deref_mut() {
                record_per_transform_far_edge(
                    sink,
                    block_decoded,
                    &luma_records,
                    LumaCodingBlockExtent::new(frontier.r, frontier.c, block_extent),
                    tile_offset,
                )?;
            }
            let recon_context = SelectableReconContext {
                leaf_y_mode: leaf_mode.luma_y_mode(),
                directional_luma,
                mrl_index: luma_transform_type_context.mrl_index(),
                mrl_sec_index: luma_transform_type_context.mrl_sec_index(),
                angle_delta_y: luma_transform_type_context.angle_delta_y(),
                chroma_mode,
                cfl_params,
                qindex: delta_q_state.qindex_u32(),
                luma_use_tcq,
                fsc_mode: fsc_mode != 0,
                is_intrabc: prelude.use_intrabc,
            };
            decode_selectable_residual_chunks(
                work_unit,
                symbols,
                &mut coeff_ctx,
                frontier,
                block_extent,
                &luma_records,
                &mut records,
                luma_transform_type_context,
                prelude.skip_flag,
                ResidualDecodeContext {
                    uv_mode,
                    angle_delta_uv: leaf_mode.luma_y_mode().map_or(0, |y_mode| {
                        chroma_angle_delta_uv(
                            y_mode,
                            uv_mode,
                            luma_transform_type_context.angle_delta_y(),
                        )
                    }),
                    is_inter: prelude.is_inter,
                    fsc_mode,
                    tool_policy: transform_tool_residual_policy,
                },
                block_decoded,
                sink.as_deref_mut(),
                recon_context,
                tile_offset,
            )?;
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            if symbols.is_past_payload_end() {
                return Err(selectable_decode_error(
                    tile_offset,
                    selectable_reason!("bitstream_desync"),
                ));
            }
            Ok(leaf_mode)
        },
    )
    .map_err(selectable_multiblock_error_at(tile_offset))?;
    let symbols = tree_output.symbols;
    let active_source_blocks = tree_output.active_source_blocks;
    let unit_filters = tree_output.unit_filters;

    symbols
        .exit_symbol()
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("exit_symbol")))?;
    tile.apply_frame_end_cdf_update();

    Ok(WienerNsLrLiveTransformRecordHandoff {
        tx_skip_rows,
        tx_skip_cols,
        records,
        active_source_blocks,
        unit_filters,
        frame_cdfs: tile.frame_cdfs(),
        cdef_grid: Some(cdef_state.into_grid(tile_offset)?),
        ccso_grid: ccso_state.into_grid(tile_offset)?,
    })
}

fn derive_selectable_luma_tx_records_for_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    context: SelectableTxSizeContext,
    tile_offset: ByteOffset,
) -> Result<Vec<SelectableLumaTxRecord>> {
    with_selectable_tx_grid(context.grid_size.0, context.grid_size.1, |grid| {
        read_tx_size_selectable(work_unit, symbols, frontier, grid, context, tile_offset)?;
        let extent = frontier_4x4_extent(
            frontier,
            tile_offset,
            selectable_reason!("region_width"),
            selectable_reason!("region_height"),
        )?;
        grid.records_for_region(frontier.r, frontier.c, extent.rows, extent.cols)
            .map_err(|error| selectable_transform_record_error(error, tile_offset))
    })
    .map_err(|error| selectable_transform_record_error(error, tile_offset))?
}

pub(in crate::runtime_minimal) fn derive_inter_luma_tx_records_for_block(
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
                    &grid,
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

#[allow(clippy::too_many_arguments)]
fn decode_selectable_residual_chunks(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    block_extent: Block4x4Extent,
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    luma_transform_type_context: LumaTransformTypeContext,
    skip_flag: bool,
    residual_context: ResidualDecodeContext,
    block_decoded: &TileBlockDecodedState,
    mut sink: Option<&mut WienerNsLrReconSink<u16>>,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    if skip_flag {
        return skip_records::record_skipped_selectable_residuals(
            coeff_ctx,
            frontier,
            block_extent.cols,
            block_extent.rows,
            luma_records,
            records,
            block_decoded,
            sink,
            recon,
            tile_offset,
        );
    }
    visit_residual_chunks(block_extent, tile_offset, |chunk| {
        let chunk_geometry = chunk.luma_geometry(frontier.r, frontier.c, tile_offset)?;
        decode_luma_records_for_chunk(
            work_unit,
            symbols,
            coeff_ctx,
            luma_records,
            records,
            chunk_geometry,
            block_extent,
            luma_transform_type_context,
            residual_context,
            sink.as_deref_mut(),
            recon,
            tile_offset,
        )?;

        if frontier.has_chroma
            && let Some(chroma_group) = chunk.chroma
        {
            decode_chroma_group(
                work_unit,
                symbols,
                coeff_ctx,
                frontier,
                chroma_group,
                residual_context,
                block_decoded,
                sink.as_deref_mut(),
                recon,
                tile_offset,
            )?;
        }
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma_residual_chunks(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    block_extent: Block4x4Extent,
    block_decoded: &TileBlockDecodedState,
    residual_context: ResidualDecodeContext,
    mut sink: Option<&mut WienerNsLrReconSink<u16>>,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    visit_residual_chunks(block_extent, tile_offset, |chunk| {
        if let Some(chroma_group) = chunk.chroma {
            decode_chroma_group(
                work_unit,
                symbols,
                coeff_ctx,
                frontier,
                chroma_group,
                residual_context,
                block_decoded,
                sink.as_deref_mut(),
                recon,
                tile_offset,
            )?;
        }
        Ok(())
    })
}

#[derive(Clone, Copy)]
struct ResidualChunk {
    x: usize,
    y: usize,
    rows: usize,
    cols: usize,
    chroma: Option<ChromaGroup>,
}

impl ResidualChunk {
    fn luma_geometry(
        self,
        block_row: usize,
        block_col: usize,
        tile_offset: ByteOffset,
    ) -> Result<ResidualChunkGeometry> {
        Ok(ResidualChunkGeometry {
            row: chunk_origin(block_row, self.y, tile_offset)?,
            col: chunk_origin(block_col, self.x, tile_offset)?,
            rows: self.rows,
            cols: self.cols,
        })
    }
}

#[derive(Clone, Copy)]
struct ResidualChunkGeometry {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}

impl ResidualChunkGeometry {
    fn row_end(self, tile_offset: ByteOffset) -> Result<usize> {
        self.row.checked_add(self.rows).ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("chunk_row_end_overflow"))
        })
    }

    fn col_end(self, tile_offset: ByteOffset) -> Result<usize> {
        self.col.checked_add(self.cols).ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("chunk_col_end_overflow"))
        })
    }
}

#[derive(Clone, Copy)]
struct ChromaGroup {
    chunk_x: usize,
    chunk_y: usize,
    luma_n4w: usize,
    luma_n4h: usize,
}

fn visit_residual_chunks(
    block: Block4x4Extent,
    tile_offset: ByteOffset,
    mut visit: impl FnMut(ResidualChunk) -> Result<()>,
) -> Result<()> {
    let (n4w, n4h) = (block.cols, block.rows);
    let width_chunks = (n4w / 16).max(1);
    let height_chunks = (n4h / 16).max(1);
    let large_chunks = width_chunks > 1 || height_chunks > 1;
    let double_chroma_w = width_chunks > 1;
    let double_chroma_h = height_chunks > 1;

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    let at_chroma_start = (!double_chroma_w || chunk_x & 1 == 0)
                        && (!double_chroma_h || chunk_y & 1 == 0);
                    let chroma = if at_chroma_start {
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
                        let luma_n4w = if large_chunks {
                            group_chunks_w.checked_mul(16).ok_or_else(|| {
                                selectable_decode_error(
                                    tile_offset,
                                    selectable_reason!("chroma_group_width_overflow"),
                                )
                            })?
                        } else {
                            n4w
                        };
                        let luma_n4h = if large_chunks {
                            group_chunks_h.checked_mul(16).ok_or_else(|| {
                                selectable_decode_error(
                                    tile_offset,
                                    selectable_reason!("chroma_group_height_overflow"),
                                )
                            })?
                        } else {
                            n4h
                        };
                        Some(ChromaGroup {
                            chunk_x,
                            chunk_y,
                            luma_n4w,
                            luma_n4h,
                        })
                    } else {
                        None
                    };
                    visit(ResidualChunk {
                        x: chunk_x,
                        y: chunk_y,
                        rows: if large_chunks { 16 } else { n4h },
                        cols: if large_chunks { 16 } else { n4w },
                        chroma,
                    })?;
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
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    chunk: ResidualChunkGeometry,
    block: Block4x4Extent,
    luma_transform_type_context: LumaTransformTypeContext,
    residual_context: ResidualDecodeContext,
    mut sink: Option<&mut WienerNsLrReconSink<u16>>,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut decoded_any = false;
    let chunk_row_end = chunk.row_end(tile_offset)?;
    let chunk_col_end = chunk.col_end(tile_offset)?;
    for record in luma_records.iter().copied().filter(|record| {
        record.row >= chunk.row
            && record.col >= chunk.col
            && record.row < chunk_row_end
            && record.col < chunk_col_end
    }) {
        decoded_any = true;
        if std::env::var_os("SPLOT_TRACE_SELECTABLE_RESIDUAL_ERROR").is_some() {
            eprintln!(
                "selectable luma residual record row={} col={} rows={} cols={} tx_size={} chunk=({}, {}) {}x{} block={}x{} leaf_mode={:?} is_inter={} is_intrabc={} fsc={} checkpoint={:?}",
                record.row,
                record.col,
                record.rows,
                record.cols,
                record.tx_size,
                chunk.row,
                chunk.col,
                chunk.rows,
                chunk.cols,
                block.rows,
                block.cols,
                recon.leaf_y_mode,
                residual_context.is_inter,
                recon.is_intrabc,
                recon.fsc_mode,
                symbols.checkpoint(),
            );
        }
        let luma = decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            0,
            record.tx_size,
            mi_to_sample(record.col, tile_offset)?,
            mi_to_sample(record.row, tile_offset)?,
            selectable_luma_tx_record_fills_block(record, block.rows, block.cols),
            None,
            false,
            residual_context.uv_mode,
            0,
            residual_context.is_inter,
            residual_context.fsc_mode != 0,
            residual_context.fsc_mode != 0,
            residual_context.luma_policy(luma_transform_type_context),
        )
        .map_err(selectable_residual_error_at(tile_offset))?;
        if let Some(sink) = sink.as_deref_mut() {
            sink.record_deblock_block(
                record.col,
                record.row,
                record.cols,
                record.rows,
                record.tx_size,
                fixed_largest_420_chroma_tx_size_from_luma_4x4(record.cols, record.rows),
                recon.qindex,
                false,
            );
            sink.reconstruct_luma_transform(
                record.col,
                record.row,
                record.tx_size,
                &luma,
                recon.leaf_y_mode,
                recon.directional_luma,
                recon.mrl_index,
                recon.mrl_sec_index,
                recon.angle_delta_y,
                recon.qindex,
                recon.luma_use_tcq,
                recon.fsc_mode,
                recon.is_intrabc,
                tile_offset,
            )?;
        }
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
        Err(selectable_decode_error(
            tile_offset,
            selectable_reason!("empty_chunk"),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma_group(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    group: ChromaGroup,
    residual_context: ResidualDecodeContext,
    block_decoded: &TileBlockDecodedState,
    sink: Option<&mut WienerNsLrReconSink<u16>>,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chroma_tx = fixed_largest_420_chroma_tx_size_from_luma_4x4(group.luma_n4w, group.luma_n4h)
        .ok_or_else(|| {
            selectable_decode_error(tile_offset, selectable_reason!("chroma_tx_size"))
        })?;
    let chroma_x = chroma_420_sample(frontier.c, group.chunk_x, tile_offset)?;
    let chroma_y = chroma_420_sample(frontier.r, group.chunk_y, tile_offset)?;
    let u = decode_chroma_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        1,
        chroma_tx,
        chroma_x,
        chroma_y,
        false,
        residual_context,
        residual_context.fsc_mode != 0,
        tile_offset,
    )?;
    let v = decode_chroma_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        2,
        chroma_tx,
        chroma_x,
        chroma_y,
        !u.all_zero,
        residual_context,
        false,
        tile_offset,
    )?;
    if let Some(sink) = sink {
        let (num4_above_right, num4_below_left) =
            chroma_group_far_edge_avail(frontier, group, block_decoded);
        for (plane, block) in [(PlaneId::U, &u), (PlaneId::V, &v)] {
            sink.reconstruct_chroma_transform(
                plane,
                chroma_tx,
                chroma_x,
                chroma_y,
                block,
                recon.chroma_mode,
                recon.angle_delta_y,
                recon.cfl_params,
                num4_above_right,
                num4_below_left,
                recon.qindex,
                tile_offset,
            )?;
        }
    }
    Ok(())
}

fn chroma_group_far_edge_avail(
    frontier: &DecodeBlockFrontier,
    group: ChromaGroup,
    block_decoded: &TileBlockDecodedState,
) -> (usize, usize) {
    let sb_mask = block_decoded.sb_size4().saturating_sub(1);
    let x4 = ((frontier.c & sb_mask) >> 1).saturating_add(group.chunk_x.saturating_mul(8));
    let y4 = ((frontier.r & sb_mask) >> 1).saturating_add(group.chunk_y.saturating_mul(8));
    let w4 = group.luma_n4w >> 1;
    let h4 = group.luma_n4h >> 1;
    (
        block_decoded.count_top_right_avail(PlaneId::U.index(), x4, y4, w4),
        block_decoded.count_bottom_left_avail(PlaneId::U.index(), x4, y4, h4),
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma_plane_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    plane: usize,
    chroma_tx: usize,
    chroma_x: usize,
    chroma_y: usize,
    chroma_context: bool,
    residual_context: ResidualDecodeContext,
    fsc_mode: bool,
    tile_offset: ByteOffset,
) -> Result<LumaCoeffBlock> {
    decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        plane,
        chroma_tx,
        chroma_x,
        chroma_y,
        true,
        None,
        chroma_context,
        residual_context.uv_mode,
        residual_context.angle_delta_uv,
        residual_context.is_inter,
        fsc_mode,
        fsc_mode,
        residual_context.tool_policy,
    )
    .map_err(selectable_residual_error_at(tile_offset))
}

fn chunk_origin(base: usize, chunk: usize, tile_offset: ByteOffset) -> Result<usize> {
    let chunk_offset = chunk.checked_mul(16).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("chunk_origin_overflow"))
    })?;
    base.checked_add(chunk_offset).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("chunk_origin_add_overflow"))
    })
}

#[derive(Clone, Copy)]
struct LumaCodingBlockExtent {
    block_row: usize,
    block_col: usize,
    n4w: usize,
    n4h: usize,
}

impl LumaCodingBlockExtent {
    const fn new(block_row: usize, block_col: usize, extent: Block4x4Extent) -> Self {
        Self {
            block_row,
            block_col,
            n4w: extent.cols,
            n4h: extent.rows,
        }
    }
}

fn transform_luma_far_edge_avail(
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    tx_row: usize,
    tx_col: usize,
    tx_w4: usize,
    tx_h4: usize,
    suppress_uneven5_far_edges: bool,
    block: LumaCodingBlockExtent,
) -> (usize, usize) {
    if suppress_uneven5_far_edges {
        return (0, 0);
    }

    let sb_mask = block_decoded.sb_size4().saturating_sub(1);
    let x4 = tx_col & sb_mask;
    let y4 = tx_row & sb_mask;
    let col_off = tx_col.saturating_sub(block.block_col);
    let row_off = tx_row.saturating_sub(block.block_row);
    let above_right = if col_off + tx_w4 < block.n4w {
        tx_w4
    } else if row_off == 0 {
        block_decoded.count_top_right_avail(0, x4, y4, tx_w4)
    } else {
        0
    };
    let below_left = if col_off > 0 {
        0
    } else if row_off + tx_h4 < block.n4h {
        tx_h4
    } else {
        block_decoded.count_bottom_left_avail(0, x4, y4, tx_h4)
    };
    (above_right, below_left)
}

fn record_per_transform_far_edge(
    sink: &mut WienerNsLrReconSink<u16>,
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    luma_records: &[SelectableLumaTxRecord],
    block: LumaCodingBlockExtent,
    tile_offset: ByteOffset,
) -> Result<()> {
    for record in luma_records {
        let tx_w4 = tx_dimension("Tx_Width", &TX_WIDTH, record.tx_size, tile_offset)? / MI_SIZE;
        let tx_h4 = tx_dimension("Tx_Height", &TX_HEIGHT, record.tx_size, tile_offset)? / MI_SIZE;
        let (above_right, below_left) = transform_luma_far_edge_avail(
            block_decoded,
            record.row,
            record.col,
            tx_w4,
            tx_h4,
            record.middle,
            block,
        );
        sink.record_block_decoded_far_edge(
            record.col,
            record.row,
            record.tx_size,
            above_right,
            below_left,
        );
    }
    Ok(())
}

fn mi_to_sample(mi: usize, tile_offset: ByteOffset) -> Result<usize> {
    mi.checked_mul(MI_SIZE).ok_or_else(|| {
        selectable_decode_error(
            tile_offset,
            selectable_reason!("sample_coordinate_overflow"),
        )
    })
}

fn chroma_420_sample(base_mi: usize, chunk: usize, tile_offset: ByteOffset) -> Result<usize> {
    let base = base_mi.checked_mul(2).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("chroma_base_overflow"))
    })?;
    let chunk_luma_mi = chunk.checked_mul(16).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("chroma_chunk_overflow"))
    })?;
    let chunk_chroma_samples = (chunk_luma_mi >> 1).checked_mul(MI_SIZE).ok_or_else(|| {
        selectable_decode_error(tile_offset, selectable_reason!("chroma_offset_overflow"))
    })?;
    base.checked_add(chunk_chroma_samples).ok_or_else(|| {
        selectable_decode_error(
            tile_offset,
            selectable_reason!("chroma_coordinate_overflow"),
        )
    })
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

fn read_tx_size_selectable(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    grid: &mut SelectableLumaTxGrid,
    context: SelectableTxSizeContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    let b_size = frontier.b_size.index();
    let extent = frontier_4x4_extent(
        frontier,
        tile_offset,
        selectable_reason!("4x4_width"),
        selectable_reason!("4x4_height"),
    )?;
    let (n4w, n4h) = (extent.cols, extent.rows);
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
    let allow_select = !context.skip_flag || !context.is_inter;

    let width = frontier
        .b_size
        .width_samples()
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("sample_width")))?;
    let height = frontier
        .b_size
        .height_samples()
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("sample_height")))?;
    let width_chunks = width >> 6;
    let height_chunks = height >> 6;
    if !allow_select {
        return max_rect::set_max_rect_tx_records(
            grid,
            frontier.r,
            frontier.c,
            n4h,
            n4w,
            max_tx_size,
            tile_offset,
        );
    }
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

    let Some(tx_partition) = read_tx_partition_symbols(
        work_unit,
        symbols,
        grid,
        frontier.r,
        frontier.c,
        max_tx_size,
        b_size,
        context.fsc_mode,
        context.is_inter,
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

#[allow(clippy::too_many_arguments)]
fn read_luma_shared_mode_info_prelude(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    cdef_state: &mut CdefState,
    ccso_state: &mut CcsoState,
    delta_q_state: &mut DeltaQState,
    intrabc_state: &mut TileIntrabcPreludeState,
    sink: Option<&mut WienerNsLrReconSink<u16>>,
    tile_offset: ByteOffset,
) -> Result<IntrabcBlockPrelude> {
    intrabc_state.prepare_for_block(frontier.r, frontier.c);
    let use_skip = read_intrabc_use_and_skip(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        intrabc_state,
        core,
        IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
        tile_offset,
    )?;
    cdef_state.read_for_block(
        work_unit,
        symbols,
        core,
        frontier,
        n4w,
        n4h,
        use_skip.skip_flag,
        tile_offset,
    )?;
    ccso_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    let intrabc = if use_skip.use_intrabc {
        Some(read_intrabc_info(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            intrabc_state,
            sequence,
            core,
            IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
            use_skip.skip_flag,
            sink,
            tile_offset,
        )?)
    } else {
        None
    };
    Ok(IntrabcBlockPrelude::from_use_skip(use_skip, intrabc))
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
    let value = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| selectable_decode_error(tile_offset, selectable_reason!("symbol_read")))?;
    if std::env::var_os("SPLOT_TRACE_TX_PARTITION").is_some() {
        eprintln!(
            "tx symbol selector={selector:?} value={value} checkpoint={:?}",
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

fn trace_tx_partition_for(row: usize, col: usize) -> bool {
    std::env::var_os("SPLOT_TRACE_TX_PARTITION").is_some()
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
    is_luma_part && !has_chroma && matches!((n4w, n4h), (1 | 2, 8) | (2, 16))
}

fn selectable_chroma_offset_leaf_supported(is_luma_part: bool, has_chroma: bool) -> bool {
    is_luma_part && !has_chroma
}

fn bit_depth_bits(sequence: &SequenceHeader) -> u32 {
    match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => 8,
        BitDepthIdc::Ten => 10,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_actual_extent_tests.rs"]
mod actual_extent_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_tool_gate_tests.rs"]
mod tool_gate_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_cdef_tests.rs"]
mod cdef_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tx_records_max_rect_tests.rs"]
mod max_rect_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "tx_records_grid_tests.rs"]
mod grid_tests;
