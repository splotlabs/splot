// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{FrameHeaderCore, MAX_CDEF_STRENGTH_SETS};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE, TX_HEIGHT, TX_WIDTH,
};
use splot_recon::{BitDepth, max_quantizer_index};
use std::ops::Range;

use crate::bitstream::tile_payload::{
    BlockSymbolTraceReadError, DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraResidualError,
    LumaTxPartition, TileCdfSelector, TileCdfSubset, luma_transform_records_for_partition,
};
use crate::error::Result;
use crate::filters::cdef::CdefUnitGrid;

use super::{frame_sb_size4, selectable_missing_quantization_error, selectable_symbol_read_error};

pub(crate) mod ccso;
pub(crate) mod gdf;

const MI_SIZE: usize = 4;
const CDEF_UNIT_MI: usize = 16;
pub(crate) const DELTA_Q_SMALL: usize = 7;
const DELTA_Q_REM_BITS_WIDTH: u32 = 3;
const DELTA_Q_SIGN_BIT_WIDTH: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrTxSkipTransformRecord {
    pub(crate) row: u16,
    pub(crate) col: u16,
    pub(crate) rows: u8,
    pub(crate) cols: u8,
    pub(crate) skip_flag: bool,
    pub(crate) empty: bool,
}

impl WienerNsLrTxSkipTransformRecord {
    pub(crate) const fn from_luma(
        block: &crate::filters::deblock::DeblockBlock,
        eob: usize,
    ) -> Self {
        Self {
            row: block.r,
            col: block.c,
            rows: block.n4h,
            cols: block.n4w,
            skip_flag: block.skip,
            empty: eob == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectableLumaTxRecord {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) tx_size: usize,
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
    base_q_index: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefState {
    window: UnitWindow,
    values: Vec<Option<usize>>,
    sb_size4: usize,
}

/// The units of a frame's filter-unit grid that one state holds: all of them
/// for the frame state, one tile's for a tile state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UnitWindow {
    row_start: usize,
    col_start: usize,
    rows: usize,
    cols: usize,
}

impl UnitWindow {
    const fn frame(rows: usize, cols: usize) -> Self {
        Self {
            row_start: 0,
            col_start: 0,
            rows,
            cols,
        }
    }

    const fn is_frame(self) -> bool {
        self.row_start == 0 && self.col_start == 0
    }

    /// The units of this frame window that a tile of `mi_rows` x `mi_cols`
    /// covers, with `unit_mi` MI rows and columns per unit.
    fn tile(self, mi_rows: &Range<usize>, mi_cols: &Range<usize>, unit_mi: usize) -> Self {
        let row_start = mi_rows.start / unit_mi;
        let col_start = mi_cols.start / unit_mi;
        Self {
            row_start,
            col_start,
            rows: mi_rows
                .end
                .div_ceil(unit_mi)
                .min(self.rows)
                .saturating_sub(row_start),
            cols: mi_cols
                .end
                .div_ceil(unit_mi)
                .min(self.cols)
                .saturating_sub(col_start),
        }
    }

    fn len(self) -> Option<usize> {
        self.rows.checked_mul(self.cols)
    }

    #[allow(clippy::inline_always, reason = "measured: per-leaf guards read it")]
    #[inline(always)]
    fn index(self, row: usize, col: usize) -> Option<usize> {
        crate::tile::local_grid_index(
            row,
            col,
            self.row_start,
            self.col_start,
            self.rows,
            self.cols,
        )
    }

    /// The units in raster order, which is their coding order.
    pub(crate) fn units(self) -> impl Iterator<Item = (usize, usize)> {
        (self.row_start..self.row_start + self.rows).flat_map(move |row| {
            (self.col_start..self.col_start + self.cols).map(move |col| (row, col))
        })
    }

    /// The indices in this frame window's values of row `row` of `tile`.
    fn tile_row(self, tile: Self, row: usize) -> Option<Range<usize>> {
        let start = self.index(tile.row_start.checked_add(row)?, tile.col_start)?;
        Some(start..start.checked_add(tile.cols)?)
    }

    /// Copies the values of `tile` into this frame window's `frame` values.
    fn merge<T: Copy>(self, tile: Self, values: &[T], frame: &mut [T]) -> Option<()> {
        for row in 0..tile.rows {
            let source = values
                .get(row.checked_mul(tile.cols)?..)?
                .get(..tile.cols)?;
            frame
                .get_mut(self.tile_row(tile, row)?)?
                .copy_from_slice(source);
        }
        Some(())
    }
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
    #[error("{table}[{index}] is outside the supported table shape")]
    TableIndex { table: &'static str, index: usize },
    #[error("{table}[{index}] value {value} is not a supported unsigned context")]
    TableValue {
        table: &'static str,
        index: usize,
        value: i32,
    },
}

#[allow(clippy::needless_pass_by_value)]
fn selectable_transform_record_error(
    _error: SelectableTransformRecordError,
) -> crate::error::DecodeError {
    selectable_state_error()
}

pub(crate) fn selectable_transform_partition_error(
    error: SelectableTransformRecordError,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    match error {
        SelectableTransformRecordError::EmptyTransform { .. }
        | SelectableTransformRecordError::InvalidTxSize { .. } => {
            crate::pipeline::malformed_tile_payload(tile_offset, "6.19.6.3", error)
        }
        _ => selectable_transform_record_error(error),
    }
}

fn selectable_allocation_error() -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane: splot_recon::PlaneId::Y,
        context: "selectable transform-record storage",
    }
    .into()
}

pub(super) fn selectable_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into()
}

impl DeltaQState {
    pub(crate) fn new(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        let delta_q = core.delta_q_params.as_ref();
        let present = delta_q.is_some_and(|params| params.delta_q_present);
        let delta_q_res = delta_q.map_or(0, |params| params.delta_q_res);
        let base_q_idx = core
            .quantization_params
            .as_ref()
            .ok_or_else(selectable_missing_quantization_error)?
            .base_q_idx;
        let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
            .map_err(|_| selectable_state_error())?;
        let sb_size4 = frame_sb_size4(sequence, core.frame_is_intra == Some(true))?;
        Ok(Self {
            present,
            delta_q_res,
            sb_size4,
            current_q_index: i32::try_from(base_q_idx).map_err(|_| selectable_state_error())?,
            max_q: i32::try_from(max_quantizer_index(bit_depth))
                .map_err(|_| selectable_state_error())?,
            current_sb: None,
            read_deltas: present,
            base_q_index: i32::try_from(base_q_idx).map_err(|_| selectable_state_error())?,
        })
    }

    /// § 5.20.5.2 `bru_mode_info()`: `CurrentQIndex = base_q_idx`.
    pub(crate) const fn enter_bru_block(&mut self) {
        self.current_q_index = self.base_q_index;
    }

    #[allow(clippy::inline_always, reason = "measured: a per-leaf guard")]
    #[inline(always)]
    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
        skip: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.present {
            return Ok(());
        }
        self.read_present(work_unit, symbols, frontier, skip, tile_offset)
    }

    #[inline(never)]
    fn read_present(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        frontier: &DecodeBlockFrontier,
        skip: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
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
                    .map_err(|_| selectable_symbol_read_error(tile_offset, "5.20.5.11"))?
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
        crate::tile::superblock_of(frontier.r, frontier.c, self.sb_size4)
    }

    pub(crate) fn qindex_u32(&self) -> u32 {
        self.current_q_index.max(0) as u32
    }
}

impl CdefState {
    /// Lays out one tile's state, on the grid a spent tile state left when
    /// `spare` carries one.
    pub(crate) fn try_for_tile(
        &self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        spare: Option<Self>,
    ) -> Result<Self> {
        let window = self.window.tile(&mi_rows, &mi_cols, CDEF_UNIT_MI);
        Self::with_window(
            window,
            spare.map(|spare| spare.values).unwrap_or_default(),
            self.sb_size4,
        )
    }

    fn with_window(
        window: UnitWindow,
        mut values: Vec<Option<usize>>,
        sb_size4: usize,
    ) -> Result<Self> {
        let len = window.len().ok_or_else(selectable_state_error)?;
        values.clear();
        values
            .try_reserve_exact(len)
            .map_err(|_| selectable_allocation_error())?;
        values.resize(len, None);
        Ok(Self {
            window,
            values,
            sb_size4,
        })
    }

    pub(crate) fn new_reusing(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        frame_is_intra: bool,
        values: Vec<Option<usize>>,
    ) -> Result<Self> {
        let window = UnitWindow::frame(
            mi_rows.div_ceil(CDEF_UNIT_MI),
            mi_cols.div_ceil(CDEF_UNIT_MI),
        );
        Self::with_window(window, values, frame_sb_size4(sequence, frame_is_intra)?)
    }

    /// Reads the block's CDEF unit index unless the unit already has one, which
    /// only an earlier read of the same frame's parameters can have set.
    #[allow(
        clippy::too_many_arguments,
        clippy::inline_always,
        reason = "measured: a per-leaf guard"
    )]
    #[inline(always)]
    pub(crate) fn read_for_block(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        n4w: usize,
        n4h: usize,
        skip_txfm: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if frontier.is_chroma_part()
            || matches!(
                self.value(frontier.r / CDEF_UNIT_MI, frontier.c / CDEF_UNIT_MI),
                Ok(Some(_))
            )
        {
            return Ok(());
        }
        self.read_unit(
            work_unit,
            symbols,
            core,
            frontier,
            (n4w, n4h),
            skip_txfm,
            tile_offset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn read_unit(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        (n4w, n4h): (usize, usize),
        skip_txfm: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let Some(cdef) = core.cdef_params.as_ref() else {
            return Err(selectable_state_error());
        };
        if !cdef.cdef_frame_enable {
            return Ok(());
        }
        if skip_txfm && cdef.cdef_on_skip_txfm_frame_enable == Some(false) {
            return Ok(());
        }
        let strengths = usize::from(cdef.cdef_strengths.ok_or_else(selectable_state_error)?);
        if !(1..=MAX_CDEF_STRENGTH_SETS).contains(&strengths) {
            return Err(selectable_state_error());
        }

        let unit_row = frontier.r / CDEF_UNIT_MI;
        let unit_col = frontier.c / CDEF_UNIT_MI;
        if self.window.index(unit_row, unit_col).is_none() {
            return Err(selectable_state_error());
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

    pub(crate) fn cdef_index0_ctx_at(
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
            && crate::tile::same_superblock_row(mi_row, above_row, self.sb_size4)
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

    pub(crate) fn fill_units(
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
        let UnitWindow {
            row_start,
            col_start,
            rows,
            cols,
        } = self.window;
        let row_end = (row_start + rows).min(start_unit_row.saturating_add(unit_rows));
        let col_end = (col_start + cols).min(start_unit_col.saturating_add(unit_cols));
        for row in start_unit_row.max(row_start)..row_end {
            for col in start_unit_col.max(col_start)..col_end {
                let index = self.index(row, col)?;
                self.values[index] = Some(strength);
            }
        }
        Ok(())
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: the read_for_block guard reads it"
    )]
    #[inline(always)]
    pub(crate) fn value(&self, row: usize, col: usize) -> Result<Option<usize>> {
        let index = self.index(row, col)?;
        Ok(self.values[index])
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: the read_for_block guard reads it"
    )]
    #[inline(always)]
    fn index(&self, row: usize, col: usize) -> Result<usize> {
        self.window
            .index(row, col)
            .ok_or_else(selectable_state_error)
    }

    pub(crate) fn merge_tile(
        &mut self,
        tile: &Self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
    ) -> Result<()> {
        if !self.window.is_frame()
            || self.sb_size4 != tile.sb_size4
            || tile.window != self.window.tile(&mi_rows, &mi_cols, CDEF_UNIT_MI)
        {
            return Err(selectable_state_error());
        }
        self.window
            .merge(tile.window, &tile.values, &mut self.values)
            .ok_or_else(selectable_state_error)
    }

    pub(crate) fn into_grid(self) -> Result<CdefUnitGrid> {
        if !self.window.is_frame() {
            return Err(selectable_state_error());
        }
        CdefUnitGrid::new(self.window.rows, self.window.cols, self.values)
            .map_err(|_| selectable_state_error())
    }
}

pub(crate) fn inter_luma_transforms(
    (row, col): (usize, usize),
    (row_end, col_end): (usize, usize),
    tx_size: usize,
    partition: LumaTxPartition,
) -> std::result::Result<impl Iterator<Item = SelectableLumaTxRecord>, SelectableTransformRecordError>
{
    let units = luma_transform_records_for_partition(0, 0, tx_size, partition).map_err(
        |error| match error {
            GeneralIntraResidualError::InvalidTransformPartitionDimensions { width, height }
                if width == 0 || height == 0 =>
            {
                SelectableTransformRecordError::EmptyTransform {
                    h4: height / MI_SIZE,
                    w4: width / MI_SIZE,
                }
            }
            GeneralIntraResidualError::InvalidTransformPartitionDimensions { width, height } => {
                SelectableTransformRecordError::InvalidTxSize { width, height }
            }
            _ => SelectableTransformRecordError::TableIndex {
                table: "Tx_Width",
                index: tx_size,
            },
        },
    )?;
    let h4 = TX_HEIGHT[tx_size] as usize / MI_SIZE;
    let w4 = TX_WIDTH[tx_size] as usize / MI_SIZE;
    Ok((row..row_end).step_by(h4).flat_map(move |r| {
        let units = units.clone();
        (col..col_end).step_by(w4).flat_map(move |c| {
            units.clone().filter_map(move |unit| {
                let (row, col) = (r + unit.y / MI_SIZE, c + unit.x / MI_SIZE);
                (row < row_end && col < col_end).then(|| SelectableLumaTxRecord {
                    row,
                    col,
                    rows: TX_HEIGHT[unit.tx_size] as usize / MI_SIZE,
                    cols: TX_WIDTH[unit.tx_size] as usize / MI_SIZE,
                    tx_size: unit.tx_size,
                })
            })
        })
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_tx_partition_symbols(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    tx_size: usize,
    mi_size: usize,
    fsc_mode: u8,
    reduced_tx_part_set: bool,
    tile_offset: ByteOffset,
) -> Result<LumaTxPartition> {
    let tx_width = tx_dimension("Tx_Width", &TX_WIDTH, tx_size)?;
    let tx_height = tx_dimension("Tx_Height", &TX_HEIGHT, tx_size)?;
    crate::bitstream::tile_payload::read_luma_tx_partition_type(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        mi_size,
        fsc_mode != 0,
        true,
        tx_height > MI_SIZE,
        tx_width > MI_SIZE,
        reduced_tx_part_set,
    )
    .map_err(|error| match error {
        GeneralIntraResidualError::TransformPartitionRead {
            source: BlockSymbolTraceReadError::Symbol(_),
        } => selectable_symbol_read_error(tile_offset, "5.20.6.3"),
        _ => selectable_state_error(),
    })
}

fn read_tx_symbol_cdef(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    read_tx_symbol(work_unit, symbols, selector, tile_offset, "5.20.10.1")
}

fn read_tx_symbol(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
    spec_section: &'static str,
) -> Result<usize> {
    read_symbol(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        selector,
        tile_offset,
        spec_section,
    )
}

pub(super) fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
    spec_section: &'static str,
) -> Result<usize> {
    cdfs.read_block_symbol(selector, symbols)
        .map(usize::from)
        .map_err(|error| match error {
            BlockSymbolTraceReadError::Cdf(_) => selectable_state_error(),
            BlockSymbolTraceReadError::Symbol(_) => {
                selectable_symbol_read_error(tile_offset, spec_section)
            }
        })
}

fn read_delta_q_abs(
    work_unit: &mut DecodeTileWorkUnit,
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
        .map_err(|_| selectable_symbol_read_error(tile_offset, "5.20.5.11"))?
        + 1;
    let delta_q_abs_bits = symbols
        .read_literal(delta_q_rem_bits)
        .map_err(|_| selectable_symbol_read_error(tile_offset, "5.20.5.11"))?;
    let delta_q_large_base = 1usize << delta_q_rem_bits;
    Ok(delta_q_abs_bits as usize + delta_q_large_base + (DELTA_Q_SMALL - 2))
}

pub(crate) fn updated_current_q_index(
    current_q_index: i32,
    reduced_delta_q_index: i32,
    delta_q_res: u8,
    max_q: i32,
) -> i32 {
    let scale = 1_i32 << delta_q_res;
    let delta = reduced_delta_q_index.saturating_mul(scale);
    current_q_index.saturating_add(delta).clamp(1, max_q)
}

pub(crate) fn delta_q_is_coded_for_block(
    block_size: usize,
    sb_size4: usize,
    skip: bool,
) -> std::result::Result<bool, SelectableTransformRecordError> {
    let width4 = table_usize("Num_4x4_Blocks_Wide", &NUM_4X4_BLOCKS_WIDE, block_size)?;
    let height4 = table_usize("Num_4x4_Blocks_High", &NUM_4X4_BLOCKS_HIGH, block_size)?;
    Ok(width4 != sb_size4 || height4 != sb_size4 || !skip)
}

fn tx_dimension(table: &'static str, values: &[i32], tx_size: usize) -> Result<usize> {
    table_usize(table, values, tx_size).map_err(selectable_transform_record_error)
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
