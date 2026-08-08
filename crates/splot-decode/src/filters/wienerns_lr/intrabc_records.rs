// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::ops::Range;

use splot_core::headers::frame::{FrameHeaderCore, MvPrecision};
#[cfg(test)]
use splot_core::headers::sequence::DrlReorder;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::{PlaneRect, PlaneSize};

use crate::bitstream::tile_payload::{
    BlockSize, DecodeBlockFrontier, TileCdfSelector, TileCdfSubset,
};
use crate::error::{DecodeError, Result};
use crate::prediction::inter::mv_scaling::{PlaneScaling, derive_plane_scaling};
use crate::prediction::inter::{
    Mv,
    read_mv::{
        MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL, MvReadConfig, lower_mv_precision,
        mv_clamp_to_integer, read_newmv_block_mvd_with_config,
    },
};

#[cfg(test)]
use super::intrabc_ref_mv_stack::{
    BANK_SB_ABOVE_ROW_MAX_HITS, DrlReorderMode, IntrabcRefMvBank, IntrabcStackAdmission,
    IntrabcStackGeometry, SpatialIntrabcScan, intrabc_ref_stack_admission,
};
use super::intrabc_ref_mv_stack::{
    SpatialIntrabcProbes, SpatialScanGeometry, capture_spatial_intrabc_probes,
};
use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;
const MORPH_PRED_CONTEXT_MAX: usize = 2;
const INTRABC_GEOMETRY_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry";

fn intrabc_geometry_error(tile_offset: ByteOffset) -> DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(tile_offset, INTRABC_GEOMETRY_REASON)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcUseSkip {
    pub(crate) use_intrabc: bool,
    pub(crate) skip_flag: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcBlockPrelude {
    pub(crate) use_intrabc: bool,
    pub(crate) is_inter: bool,
    pub(crate) skip_flag: bool,
    pub(crate) morph_pred: bool,
    pub(crate) intrabc: Option<IntrabcInfo>,
}

impl IntrabcBlockPrelude {
    pub(crate) const fn from_use_skip(
        use_skip: IntrabcUseSkip,
        intrabc: Option<IntrabcInfo>,
    ) -> Self {
        Self {
            use_intrabc: use_skip.use_intrabc,
            is_inter: use_skip.use_intrabc,
            skip_flag: use_skip.skip_flag,
            morph_pred: match intrabc {
                Some(info) => info.morph_pred,
                None => false,
            },
            intrabc,
        }
    }

    pub(crate) const fn mark_inter(mut self) -> Self {
        self.is_inter = true;
        self
    }

    pub(crate) const fn with_morph_pred(mut self, morph_pred: bool) -> Self {
        self.morph_pred = morph_pred;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcBlockContext {
    row: usize,
    col: usize,
    b_size: usize,
    is_chroma_part: bool,
    mixed_region: bool,
}

impl IntrabcBlockContext {
    pub(crate) fn from_frontier(frontier: &DecodeBlockFrontier) -> Self {
        Self {
            row: frontier.r,
            col: frontier.c,
            b_size: frontier.b_size.index(),
            is_chroma_part: frontier.is_chroma_part(),
            mixed_region: frontier.is_mixed_region(),
        }
    }

    const fn from_chroma_ref(row: usize, col: usize, b_size: BlockSize) -> Self {
        Self {
            row,
            col,
            b_size: b_size.index(),
            is_chroma_part: false,
            mixed_region: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcBlockGeometry {
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
}

impl IntrabcBlockGeometry {
    pub(crate) fn from_frontier(frontier: &DecodeBlockFrontier, n4w: usize, n4h: usize) -> Self {
        Self {
            block: IntrabcBlockContext::from_frontier(frontier),
            n4w,
            n4h,
        }
    }

    pub(crate) fn from_chroma_ref(
        row: usize,
        col: usize,
        b_size: BlockSize,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let n4w = b_size
            .num_4x4_wide()
            .map_err(|_| intrabc_geometry_error(tile_offset))?;
        let n4h = b_size
            .num_4x4_high()
            .map_err(|_| intrabc_geometry_error(tile_offset))?;
        Ok(Self {
            block: IntrabcBlockContext::from_chroma_ref(row, col, b_size),
            n4w,
            n4h,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcInfo {
    pub(crate) intrabc_mode: u8,
    pub(crate) ref_mv_idx: usize,
    pub(crate) mv_precision: u8,
    pub(crate) morph_pred: bool,
    pub(crate) block_mv: IntrabcBlockVector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcBlockVector {
    pub(crate) row: i32,
    pub(crate) col: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcPredictionGeometry {
    pub(crate) scaling: PlaneScaling,
    pub(crate) fractional: bool,
    pub(crate) source: PlaneRect,
    pub(crate) target: PlaneRect,
    pub(crate) ref_mi_cols: i32,
    pub(crate) ref_mi_rows: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcInfoSyntax {
    intrabc_mode: usize,
    ref_mv_idx: usize,
    mv_precision: u8,
    max_bvp_drl_bits_minus_1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingIntrabcInfo {
    syntax: IntrabcInfoSyntax,
    mvd: Option<Mv>,
    morph_pred: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcLumaPredictionDomain {
    storage: PlaneSize,
    tile_bounds: PlaneRect,
    ref_mi_cols: i32,
    ref_mi_rows: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockPixels {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl IntrabcBlockPixels {
    fn from_geometry(geometry: IntrabcBlockGeometry, tile_offset: ByteOffset) -> Result<Self> {
        Ok(Self {
            x: checked_mi_to_luma(geometry.block.col, tile_offset)?,
            y: checked_mi_to_luma(geometry.block.row, tile_offset)?,
            width: checked_mi_to_luma(geometry.n4w, tile_offset)?,
            height: checked_mi_to_luma(geometry.n4h, tile_offset)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntrabcNeighborContext {
    UseIntrabc,
    Skip,
    MorphPred,
}

impl IntrabcNeighborContext {
    const fn same_sb_row(self) -> bool {
        matches!(self, Self::UseIntrabc | Self::MorphPred)
    }

    const fn max(self) -> usize {
        match self {
            Self::UseIntrabc => INTRABC_CONTEXT_MAX,
            Self::Skip => SKIP_CONTEXT_MAX,
            Self::MorphPred => MORPH_PRED_CONTEXT_MAX,
        }
    }

    const fn matches(self, facts: IntrabcBlockFacts) -> bool {
        match self {
            Self::UseIntrabc => facts.use_intrabc,
            Self::Skip => facts.skip_flag,
            Self::MorphPred => facts.morph_pred,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IntrabcMiArea {
    rows: Range<usize>,
    cols: Range<usize>,
}

impl IntrabcMiArea {
    fn clipped(
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        mi_rows: usize,
        mi_cols: usize,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        Ok(Self {
            rows: clipped_mi_range(
                row,
                n4h,
                mi_rows,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_row_overflow",
                tile_offset,
            )?,
            cols: clipped_mi_range(
                col,
                n4w,
                mi_cols,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_col_overflow",
                tile_offset,
            )?,
        })
    }

    fn from_tile_starts(
        row_starts: &[u32],
        col_starts: &[u32],
        geometry: IntrabcBlockGeometry,
        bounds_reason: &'static str,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let (col_start, col_end) = tile_interval_for_block(
            col_starts,
            geometry.block.col,
            geometry.n4w,
            bounds_reason,
            tile_offset,
        )?;
        let (row_start, row_end) = tile_interval_for_block(
            row_starts,
            geometry.block.row,
            geometry.n4h,
            bounds_reason,
            tile_offset,
        )?;
        Ok(Self {
            rows: row_start..row_end,
            cols: col_start..col_end,
        })
    }

    fn luma_rect(&self, tile_offset: ByteOffset) -> Result<PlaneRect> {
        let x = checked_mi_to_luma(self.cols.start, tile_offset)?;
        let y = checked_mi_to_luma(self.rows.start, tile_offset)?;
        let width = checked_mi_to_luma(self.cols.len(), tile_offset)?;
        let height = checked_mi_to_luma(self.rows.len(), tile_offset)?;
        PlaneRect::new(x, y, width, height).map_err(|_| intrabc_geometry_error(tile_offset))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcNeighborScan {
    row: usize,
    col: usize,
    n4w: usize,
    n4h: usize,
    same_sb_row: bool,
    tile_offset: ByteOffset,
}

impl From<Mv> for IntrabcBlockVector {
    fn from(value: Mv) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

impl From<IntrabcBlockVector> for Mv {
    fn from(value: IntrabcBlockVector) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileIntrabcPreludeState {
    enabled: bool,
    mi_rows: usize,
    mi_cols: usize,
    origin_row: usize,
    origin_col: usize,
    tile_rows: usize,
    tile_cols: usize,
    sb_size4: usize,
    values: Vec<IntrabcGridCell>,
    #[cfg(test)]
    test_values: Vec<Option<IntrabcTestFacts>>,
    #[cfg(test)]
    bank: IntrabcRefMvBank,
    #[cfg(test)]
    enable_refmvbank: bool,
    #[cfg(test)]
    seed_bank_from_above: bool,
    #[cfg(test)]
    drl_reorder: DrlReorderMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockFacts {
    use_intrabc: bool,
    is_inter: bool,
    skip_flag: bool,
    morph_pred: bool,
    #[cfg(test)]
    block_mv: Option<IntrabcBlockVector>,
    base_col: usize,
    #[cfg(test)]
    n4w: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IntrabcGridCell {
    base_col: u32,
    flags: u8,
}

impl IntrabcGridCell {
    const CODED: u8 = 1 << 0;
    const USE_INTRABC: u8 = 1 << 1;
    const IS_INTER: u8 = 1 << 2;
    const SKIP: u8 = 1 << 3;
    const MORPH_PRED: u8 = 1 << 4;

    fn new(facts: IntrabcBlockFacts, tile_offset: ByteOffset) -> Result<Self> {
        let base_col = u32::try_from(facts.base_col).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_overflow",
            )
        })?;
        let flags = Self::CODED
            | (u8::from(facts.use_intrabc) * Self::USE_INTRABC)
            | (u8::from(facts.is_inter) * Self::IS_INTER)
            | (u8::from(facts.skip_flag) * Self::SKIP)
            | (u8::from(facts.morph_pred) * Self::MORPH_PRED);
        Ok(Self { base_col, flags })
    }

    const fn contains(self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcTestFacts {
    block_mv: Option<IntrabcBlockVector>,
    n4w: usize,
}

impl TileIntrabcPreludeState {
    #[cfg(test)]
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        frame_is_intra_only: bool,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        Self::new_for_tile(
            (mi_rows, mi_cols),
            0..mi_rows,
            0..mi_cols,
            sequence,
            frame_is_intra_only,
            true,
            tile_offset,
        )
    }

    pub(crate) fn new_for_tile(
        frame_mi_size: (usize, usize),
        tile_rows: Range<usize>,
        tile_cols: Range<usize>,
        sequence: &SequenceHeader,
        frame_is_intra_only: bool,
        enabled: bool,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let (mi_rows, mi_cols) = frame_mi_size;
        let rows = tile_rows.end.saturating_sub(tile_rows.start);
        let cols = tile_cols.end.saturating_sub(tile_cols.start);
        let values_len = if enabled {
            rows.checked_mul(cols).ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_intrabc_grid_overflow",
                )
            })?
        } else {
            0
        };
        let sb_size4 = intrabc_sb_size4(sequence, frame_is_intra_only, tile_offset)?;
        #[cfg(test)]
        let enable_refmvbank = enabled
            && sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_refmvbank);
        #[cfg(test)]
        let drl_reorder = match sequence.inter.as_ref().map(|inter| inter.drl_reorder) {
            Some(DrlReorder::Always) => DrlReorderMode::Always,
            Some(DrlReorder::Constraint) => DrlReorderMode::Constraint,
            Some(DrlReorder::Disabled) | None => DrlReorderMode::Disabled,
        };
        Ok(Self {
            enabled,
            mi_rows,
            mi_cols,
            origin_row: tile_rows.start,
            origin_col: tile_cols.start,
            tile_rows: rows,
            tile_cols: cols,
            sb_size4,
            values: vec![IntrabcGridCell::default(); values_len],
            #[cfg(test)]
            test_values: vec![None; values_len],
            #[cfg(test)]
            bank: IntrabcRefMvBank::new(sb_size4),
            #[cfg(test)]
            enable_refmvbank,
            #[cfg(test)]
            seed_bank_from_above: !frame_is_intra_only,
            #[cfg(test)]
            drl_reorder,
        })
    }

    pub(crate) fn record_block(
        &mut self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        prelude: IntrabcBlockPrelude,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        #[cfg(test)]
        let block_mv = prelude.intrabc.map(|info| info.block_mv);
        let facts = IntrabcBlockFacts {
            use_intrabc: prelude.use_intrabc,
            is_inter: prelude.is_inter,
            skip_flag: prelude.skip_flag,
            morph_pred: prelude.morph_pred,
            #[cfg(test)]
            block_mv,
            base_col: col,
            #[cfg(test)]
            n4w,
        };
        let value = IntrabcGridCell::new(facts, tile_offset)?;
        #[cfg(test)]
        let test_value = IntrabcTestFacts {
            block_mv: facts.block_mv,
            n4w,
        };
        let area = self.clipped_record_area(row, col, n4w, n4h, tile_offset)?;
        if !area.cols.is_empty() {
            for r in area.rows {
                let start = self.index(r, area.cols.start, tile_offset)?;
                let end = start.checked_add(area.cols.len()).ok_or_else(|| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
                    )
                })?;
                let row_values = self.values.get_mut(start..end).ok_or_else(|| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
                    )
                })?;
                row_values.fill(value);
                #[cfg(test)]
                self.test_values
                    .get_mut(start..end)
                    .ok_or_else(|| {
                        wienerns_lr_selectable_transform_record_error_reason(
                            tile_offset,
                            "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
                        )
                    })?
                    .fill(Some(test_value));
            }
        }
        #[cfg(test)]
        if self.enable_refmvbank {
            self.bank.update_after_block(
                row,
                col,
                n4w,
                n4h,
                prelude.use_intrabc,
                block_mv.map(Mv::from),
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn prepare_for_block(&mut self, row: usize, col: usize) {
        let entered = self.enable_refmvbank && self.bank.enter_block_superblock(row, col);
        if entered && self.seed_bank_from_above {
            self.seed_bank_from_above_row(row, col);
        }
    }

    #[cfg(test)]
    fn seed_bank_from_above_row(&mut self, row: usize, col: usize) {
        if self.sb_size4 == 0 {
            return;
        }
        let sb_row = row / self.sb_size4 * self.sb_size4;
        if sb_row <= self.origin_row {
            return;
        }
        let sb_col = col / self.sb_size4 * self.sb_size4;
        let tile_col_end = self.origin_col.saturating_add(self.tile_cols);
        let sb_width = self.sb_size4.min(tile_col_end.saturating_sub(sb_col));
        let mut offset = 0usize;
        let mut hits = 0usize;
        while offset < sb_width && hits < BANK_SB_ABOVE_ROW_MAX_HITS {
            let aligned = offset / 2 * 2;
            let Some(facts) = self.facts_at(sb_row - 1, sb_col.saturating_add(aligned)) else {
                offset = offset.saturating_add(1);
                continue;
            };
            if facts.is_inter {
                hits += 1;
                let mv = facts
                    .use_intrabc
                    .then_some(facts.block_mv)
                    .flatten()
                    .map(Into::into);
                self.bank.seed_from_above_row(mv);
            }
            offset = offset.saturating_add(facts.n4w.max(1));
        }
    }

    pub(crate) fn intrabc_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        self.neighbor_context(
            row,
            col,
            n4w,
            n4h,
            IntrabcNeighborContext::UseIntrabc,
            tile_offset,
        )
    }

    fn skip_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        self.neighbor_context(
            row,
            col,
            n4w,
            n4h,
            IntrabcNeighborContext::Skip,
            tile_offset,
        )
    }

    fn morph_pred_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        self.neighbor_context(
            row,
            col,
            n4w,
            n4h,
            IntrabcNeighborContext::MorphPred,
            tile_offset,
        )
    }

    fn clipped_record_area(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<IntrabcMiArea> {
        IntrabcMiArea::clipped(
            row,
            col,
            n4w,
            n4h,
            self.origin_row.saturating_add(self.tile_rows),
            self.origin_col.saturating_add(self.tile_cols),
            tile_offset,
        )
    }

    fn neighbor_context(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        context: IntrabcNeighborContext,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let mut ctx = 0usize;
        self.visit_neighbor_positions(
            IntrabcNeighborScan {
                row,
                col,
                n4w,
                n4h,
                same_sb_row: context.same_sb_row(),
                tile_offset,
            },
            |r, c| {
                if self
                    .value(r, c, tile_offset)?
                    .is_some_and(|facts| context.matches(facts))
                {
                    ctx += 1;
                }
                Ok(())
            },
        )?;
        Ok(ctx.min(context.max()))
    }

    fn visit_neighbor_positions<F>(&self, scan: IntrabcNeighborScan, mut visit: F) -> Result<()>
    where
        F: FnMut(usize, usize) -> Result<()>,
    {
        let IntrabcNeighborScan {
            row,
            col,
            n4w,
            n4h,
            same_sb_row,
            tile_offset,
        } = scan;
        if n4w == 0 || n4h == 0 {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_empty_block",
            ));
        }
        let bottom_row = row.checked_add(n4h - 1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_neighbor_row_overflow",
            )
        })?;
        let right_col = col.checked_add(n4w - 1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_neighbor_col_overflow",
            )
        })?;
        let candidates = [
            col.checked_sub(1).map(|left_col| (bottom_row, left_col)),
            row.checked_sub(1).map(|above_row| (above_row, right_col)),
            col.checked_sub(1).map(|left_col| (row, left_col)),
            row.checked_sub(1).map(|above_row| (above_row, col)),
        ];
        let mut accepted = 0usize;
        for (r, c) in candidates.into_iter().flatten() {
            if r < self.origin_row
                || c < self.origin_col
                || r >= self.origin_row.saturating_add(self.tile_rows)
                || c >= self.origin_col.saturating_add(self.tile_cols)
            {
                continue;
            }
            if same_sb_row && r / self.sb_size4 != row / self.sb_size4 {
                continue;
            }
            visit(r, c)?;
            accepted += 1;
            if accepted == 2 {
                break;
            }
        }
        Ok(())
    }

    fn value(
        &self,
        row: usize,
        col: usize,
        tile_offset: ByteOffset,
    ) -> Result<Option<IntrabcBlockFacts>> {
        let index = self.index(row, col, tile_offset)?;
        Ok(self.facts_at_index(index))
    }

    fn index(&self, row: usize, col: usize, tile_offset: ByteOffset) -> Result<usize> {
        if row < self.origin_row
            || col < self.origin_col
            || row >= self.origin_row.saturating_add(self.tile_rows)
            || col >= self.origin_col.saturating_add(self.tile_cols)
        {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
            ));
        }
        row.checked_sub(self.origin_row)
            .and_then(|row| row.checked_mul(self.tile_cols))
            .and_then(|start| {
                col.checked_sub(self.origin_col)
                    .and_then(|col| start.checked_add(col))
            })
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_overflow",
                )
            })
    }

    #[cfg(test)]
    fn spatial_intrabc_scan(&self, geometry: IntrabcBlockGeometry) -> SpatialIntrabcScan {
        self.capture_spatial_intrabc_probes(geometry)
            .resolve(|row, col| self.block_vector_at(row, col))
    }

    pub(crate) fn capture_spatial_intrabc_probes(
        &self,
        geometry: IntrabcBlockGeometry,
    ) -> SpatialIntrabcProbes {
        capture_spatial_intrabc_probes(
            self.spatial_scan_geometry(geometry),
            |row, col| self.is_mi_coded(row, col),
            |row, col| self.block_base_col_at(row, col),
        )
    }

    fn spatial_scan_geometry(&self, geometry: IntrabcBlockGeometry) -> SpatialScanGeometry {
        SpatialScanGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            sb_size4: self.sb_size4,
        }
    }

    #[cfg(test)]
    fn block_vector_at(&self, row: usize, col: usize) -> Option<Mv> {
        let facts = self.facts_at(row, col)?;
        if !facts.use_intrabc {
            return None;
        }
        facts.block_mv.map(Mv::from)
    }

    fn is_mi_coded(&self, row: usize, col: usize) -> bool {
        self.facts_at(row, col).is_some()
    }

    fn block_base_col_at(&self, row: usize, col: usize) -> Option<usize> {
        self.facts_at(row, col).map(|facts| facts.base_col)
    }

    fn facts_at(&self, row: usize, col: usize) -> Option<IntrabcBlockFacts> {
        if row < self.origin_row
            || col < self.origin_col
            || row >= self.origin_row.saturating_add(self.tile_rows)
            || col >= self.origin_col.saturating_add(self.tile_cols)
        {
            return None;
        }
        self.facts_at_index(
            row.checked_sub(self.origin_row)?
                .checked_mul(self.tile_cols)?
                .checked_add(col.checked_sub(self.origin_col)?)?,
        )
    }

    fn facts_at_index(&self, index: usize) -> Option<IntrabcBlockFacts> {
        let value = *self.values.get(index)?;
        if !value.contains(IntrabcGridCell::CODED) {
            return None;
        }
        #[cfg(test)]
        let test = self.test_values.get(index)?.as_ref()?;
        Some(IntrabcBlockFacts {
            use_intrabc: value.contains(IntrabcGridCell::USE_INTRABC),
            is_inter: value.contains(IntrabcGridCell::IS_INTER),
            skip_flag: value.contains(IntrabcGridCell::SKIP),
            morph_pred: value.contains(IntrabcGridCell::MORPH_PRED),
            #[cfg(test)]
            block_mv: test.block_mv,
            base_col: value.base_col as usize,
            #[cfg(test)]
            n4w: test.n4w,
        })
    }

    #[cfg(test)]
    fn bank(&self) -> &IntrabcRefMvBank {
        &self.bank
    }
}

pub(crate) fn read_intrabc_use_and_skip(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcUseSkip> {
    let block = geometry.block;
    let n4w = geometry.n4w;
    let n4h = geometry.n4h;
    if !intrabc_use_is_coded(core, block, n4w, n4h) {
        return Ok(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: false,
        });
    }
    let intrabc_ctx = state.intrabc_ctx(block.row, block.col, n4w, n4h, tile_offset)?;
    let use_intrabc = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::Intrabc { ctx: intrabc_ctx },
        tile_offset,
    )? != 0;
    if !use_intrabc {
        return Ok(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: false,
        });
    }
    let skip_ctx = state.skip_ctx(block.row, block.col, n4w, n4h, tile_offset)?;
    let skip_flag = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::Skip { ctx: skip_ctx },
        tile_offset,
    )? != 0;
    Ok(IntrabcUseSkip {
        use_intrabc: true,
        skip_flag,
    })
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn read_intrabc_info(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let pending =
        read_pending_intrabc_info(cdfs, symbols, state, sequence, core, geometry, tile_offset)?;
    let pred_mv =
        ensure_intrabc_ref_stack_supported(state, sequence, geometry, pending.syntax, tile_offset)?;
    Ok(resolve_pending_intrabc_info(pending, pred_mv))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_pending_intrabc_info(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<PendingIntrabcInfo> {
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    let mvd = if syntax.intrabc_mode == 0 {
        Some(read_intrabc_mvd(
            cdfs,
            symbols,
            syntax.mv_precision,
            tile_offset,
        )?)
    } else {
        None
    };
    let block = geometry.block;
    let morph_pred_ctx = state.morph_pred_ctx(
        block.row,
        block.col,
        geometry.n4w,
        geometry.n4h,
        tile_offset,
    )?;
    let morph_pred =
        read_intrabc_morph_pred(cdfs, symbols, sequence, core, morph_pred_ctx, tile_offset)?;
    Ok(PendingIntrabcInfo {
        syntax,
        mvd,
        morph_pred,
    })
}

pub(crate) fn resolve_pending_intrabc_info(
    pending: PendingIntrabcInfo,
    pred_mv: Mv,
) -> IntrabcInfo {
    let syntax = pending.syntax;
    let block_mv = match pending.mvd {
        Some(diff) => {
            let pred_mv = if syntax.mv_precision == MV_PRECISION_ONE_PEL {
                lower_mv_precision(syntax.mv_precision, pred_mv)
            } else {
                pred_mv
            };
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
        None => pred_mv,
    };
    IntrabcInfo {
        intrabc_mode: u8::try_from(syntax.intrabc_mode).unwrap_or(1),
        ref_mv_idx: syntax.ref_mv_idx,
        mv_precision: syntax.mv_precision,
        morph_pred: pending.morph_pred,
        block_mv: block_mv.into(),
    }
}

impl PendingIntrabcInfo {
    pub(crate) const fn ref_mv_idx(self) -> usize {
        self.syntax.ref_mv_idx
    }

    pub(crate) const fn max_bvp_drl_bits_minus_1(self) -> u32 {
        self.syntax.max_bvp_drl_bits_minus_1
    }

    pub(crate) const fn morph_pred(self) -> bool {
        self.morph_pred
    }
}

fn read_intrabc_info_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfoSyntax> {
    let force_integer_mv = resolve_intrabc_force_integer_mv(core, tile_offset)?;
    let max_bvp_drl_bits_minus_1 = max_bvp_drl_bits_minus_1(sequence, core, tile_offset)?;
    let m = usize::try_from(max_bvp_drl_bits_minus_1)
        .ok()
        .and_then(|bits| bits.checked_add(1))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_drl_count",
            )
        })?;
    let intrabc_mode = read_symbol(cdfs, symbols, TileCdfSelector::IntrabcMode, tile_offset)?;
    let mut ref_mv_idx = 0usize;
    for idx in 0..m {
        let drl_mode = symbols.read_literal(1).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_drl_read",
            )
        })?;
        if drl_mode == 0 {
            ref_mv_idx = idx;
            break;
        }
        ref_mv_idx = idx.checked_add(1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx",
            )
        })?;
    }

    let mut mv_precision = if force_integer_mv {
        MV_PRECISION_ONE_PEL
    } else {
        MV_PRECISION_QUARTER_PEL
    };
    if intrabc_mode == 0 && !force_integer_mv {
        let precision = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IntrabcPrecision,
            tile_offset,
        )?;
        mv_precision = if precision != 0 {
            MV_PRECISION_QUARTER_PEL
        } else {
            MV_PRECISION_ONE_PEL
        };
    }
    Ok(IntrabcInfoSyntax {
        intrabc_mode,
        ref_mv_idx,
        mv_precision,
        max_bvp_drl_bits_minus_1,
    })
}

fn resolve_intrabc_force_integer_mv(
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if let Some(force_integer_mv) = core.force_integer_mv {
        return Ok(force_integer_mv);
    }
    let Some(inter) = core.inter.as_ref() else {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_mv_precision",
        ));
    };
    if let Some(force_integer_mv) = inter.force_integer_mv {
        return Ok(force_integer_mv);
    }
    match inter.mv_precision {
        Some(MvPrecision::OnePel) => Ok(true),
        Some(MvPrecision::HalfPel | MvPrecision::QuarterPel | MvPrecision::EighthPel) => Ok(false),
        Some(_) | None => Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_mv_precision",
        )),
    }
}

#[cfg(test)]
fn ensure_intrabc_ref_stack_supported(
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    syntax: IntrabcInfoSyntax,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    let stack_geometry = IntrabcStackGeometry {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        n4w: geometry.n4w,
        n4h: geometry.n4h,
        sb_samples: superblock_samples(sequence, tile_offset)?,
        frame_w: i32::try_from(state.mi_cols.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        frame_h: i32::try_from(state.mi_rows.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        max_bvp_drl_bits_minus_1: syntax.max_bvp_drl_bits_minus_1,
    };
    let spatial = state.spatial_intrabc_scan(geometry);
    let admission = intrabc_ref_stack_admission(
        state.bank(),
        stack_geometry,
        &spatial,
        state.enable_refmvbank,
        state.drl_reorder,
        syntax.ref_mv_idx,
    );
    match admission {
        IntrabcStackAdmission::Admit { selected } => Ok(selected),
        IntrabcStackAdmission::Defer => Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack",
        )),
    }
}

fn read_intrabc_mvd(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mv_precision: u8,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    read_newmv_block_mvd_with_config(
        cdfs,
        symbols,
        tile_offset,
        MvReadConfig::intrabc(mv_precision),
    )
    .map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv",
        )
    })
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn finish_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    syntax: IntrabcInfoSyntax,
    pred_mv: Mv,
    morph_pred_ctx: usize,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let mvd = if syntax.intrabc_mode == 0 {
        Some(read_intrabc_mvd(
            cdfs,
            symbols,
            syntax.mv_precision,
            tile_offset,
        )?)
    } else {
        None
    };
    let morph_pred =
        read_intrabc_morph_pred(cdfs, symbols, sequence, core, morph_pred_ctx, tile_offset)?;
    Ok(resolve_pending_intrabc_info(
        PendingIntrabcInfo {
            syntax,
            mvd,
            morph_pred,
        },
        pred_mv,
    ))
}

#[cfg(test)]
fn assign_intrabc_mv(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    intrabc_mode: usize,
    mv_precision: u8,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcBlockVector> {
    let mvd = if intrabc_mode == 0 {
        Some(read_intrabc_mvd(cdfs, symbols, mv_precision, tile_offset)?)
    } else {
        None
    };
    Ok(resolve_pending_intrabc_info(
        PendingIntrabcInfo {
            syntax: IntrabcInfoSyntax {
                intrabc_mode,
                ref_mv_idx: 0,
                mv_precision,
                max_bvp_drl_bits_minus_1: 0,
            },
            mvd,
            morph_pred: false,
        },
        pred_mv,
    )
    .block_mv)
}

fn read_intrabc_morph_pred(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    morph_pred_ctx: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if core.frame_is_intra != Some(true)
        || core.allow_screen_content_tools != Some(true)
        || !sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_bawp)
    {
        return Ok(false);
    }
    Ok(read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::MorphPred {
            ctx: morph_pred_ctx,
        },
        tile_offset,
    )? != 0)
}

pub(crate) fn derive_intrabc_luma_prediction_geometry(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<IntrabcPredictionGeometry> {
    let domain = intrabc_luma_prediction_domain(core, geometry, tile_offset)?;
    let block = IntrabcBlockPixels::from_geometry(geometry, tile_offset)?;
    let target = intrabc_clamped_target(
        block.x,
        block.y,
        block.width,
        block.height,
        &domain,
        tile_offset,
    )?;
    let fractional = intrabc_block_vector_is_fractional(info.block_mv);
    let source = if fractional {
        target
    } else {
        intrabc_luma_source_envelope(target, info.block_mv, tile_offset)?
    };
    if !fractional
        && (!source.is_within(domain.storage) || !rect_is_within_rect(source, domain.tile_bounds))
    {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    let frame_size = core.frame_size.ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size",
        )
    })?;
    let frame_width =
        i32::try_from(frame_size.width).map_err(|_| intrabc_geometry_error(tile_offset))?;
    let frame_height =
        i32::try_from(frame_size.height).map_err(|_| intrabc_geometry_error(tile_offset))?;
    let scaling = derive_plane_scaling(
        checked_i32_from_usize(block.x, tile_offset)?,
        checked_i32_from_usize(block.y, tile_offset)?,
        info.block_mv.row,
        info.block_mv.col,
        0,
        0,
        frame_width,
        frame_height,
        frame_width,
        frame_height,
    );
    Ok(IntrabcPredictionGeometry {
        scaling,
        fractional,
        source,
        target,
        ref_mi_cols: domain.ref_mi_cols,
        ref_mi_rows: domain.ref_mi_rows,
    })
}

const fn intrabc_block_vector_is_fractional(block_mv: IntrabcBlockVector) -> bool {
    block_mv.row & 7 != 0 || block_mv.col & 7 != 0
}

fn intrabc_clamped_target(
    target_x: usize,
    target_y: usize,
    width: usize,
    height: usize,
    domain: &IntrabcLumaPredictionDomain,
    tile_offset: ByteOffset,
) -> Result<PlaneRect> {
    let tile_x = domain.tile_bounds.x();
    let tile_y = domain.tile_bounds.y();
    let tile_right = tile_x
        .checked_add(domain.tile_bounds.width())
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let tile_bottom = tile_y
        .checked_add(domain.tile_bounds.height())
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let nominal_right = target_x
        .checked_add(width)
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let nominal_bottom = target_y
        .checked_add(height)
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let visible_right = nominal_right.min(domain.storage.width()).min(tile_right);
    let visible_bottom = nominal_bottom.min(domain.storage.height()).min(tile_bottom);
    if target_x < tile_x
        || target_y < tile_y
        || target_x >= domain.storage.width()
        || target_y >= domain.storage.height()
        || target_x >= visible_right
        || target_y >= visible_bottom
    {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        ));
    }
    let eff_width = visible_right - target_x;
    let eff_height = visible_bottom - target_y;
    PlaneRect::new(target_x, target_y, eff_width, eff_height)
        .map_err(|_| intrabc_geometry_error(tile_offset))
}

fn intrabc_luma_prediction_domain(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcLumaPredictionDomain> {
    let _frame_size = core.frame_size.ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size",
        )
    })?;
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let mi_cols = tile_info
        .mi_col_starts
        .last()
        .copied()
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let mi_rows = tile_info
        .mi_row_starts
        .last()
        .copied()
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    if mi_cols == 0 || mi_rows == 0 {
        return Err(intrabc_geometry_error(tile_offset));
    }
    let width = checked_mi_u32_to_luma(mi_cols, tile_offset)?;
    let height = checked_mi_u32_to_luma(mi_rows, tile_offset)?;
    let storage = PlaneSize::new(width, height).map_err(|_| intrabc_geometry_error(tile_offset))?;
    let tile_area = IntrabcMiArea::from_tile_starts(
        &tile_info.mi_row_starts,
        &tile_info.mi_col_starts,
        geometry,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let tile_bounds = tile_area.luma_rect(tile_offset)?;
    Ok(IntrabcLumaPredictionDomain {
        storage,
        tile_bounds,
        ref_mi_cols: mi_cols as i32,
        ref_mi_rows: mi_rows as i32,
    })
}

fn tile_interval_for_block(
    starts: &[u32],
    block_start: usize,
    block_len: usize,
    bounds_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    checked_mi_end(block_start, block_len, INTRABC_GEOMETRY_REASON, tile_offset)?;
    for window in starts.windows(2) {
        let start = usize::try_from(window[0]).map_err(|_| intrabc_geometry_error(tile_offset))?;
        let end = usize::try_from(window[1]).map_err(|_| intrabc_geometry_error(tile_offset))?;
        if block_start >= start && block_start < end {
            return Ok((start, end));
        }
    }
    Err(wienerns_lr_selectable_transform_record_error_reason(
        tile_offset,
        bounds_reason,
    ))
}

fn clipped_mi_range(
    start: usize,
    len: usize,
    limit: usize,
    overflow_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<Range<usize>> {
    let end = checked_mi_end(start, len, overflow_reason, tile_offset)?;
    Ok(start..end.min(limit))
}

fn checked_mi_end(
    start: usize,
    len: usize,
    overflow_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<usize> {
    start.checked_add(len).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(tile_offset, overflow_reason)
    })
}

fn checked_mi_to_luma(mi: usize, tile_offset: ByteOffset) -> Result<usize> {
    mi.checked_mul(MI_SIZE)
        .ok_or_else(|| intrabc_geometry_error(tile_offset))
}

fn checked_mi_u32_to_luma(mi: u32, tile_offset: ByteOffset) -> Result<usize> {
    checked_mi_to_luma(
        usize::try_from(mi).map_err(|_| intrabc_geometry_error(tile_offset))?,
        tile_offset,
    )
}

fn checked_i32_from_usize(value: usize, tile_offset: ByteOffset) -> Result<i32> {
    i32::try_from(value).map_err(|_| intrabc_geometry_error(tile_offset))
}

fn rect_is_within_rect(rect: PlaneRect, bounds: PlaneRect) -> bool {
    let Some(rect_right) = rect.x().checked_add(rect.width()) else {
        return false;
    };
    let Some(rect_bottom) = rect.y().checked_add(rect.height()) else {
        return false;
    };
    let Some(bounds_right) = bounds.x().checked_add(bounds.width()) else {
        return false;
    };
    let Some(bounds_bottom) = bounds.y().checked_add(bounds.height()) else {
        return false;
    };
    rect.x() >= bounds.x()
        && rect.y() >= bounds.y()
        && rect_right <= bounds_right
        && rect_bottom <= bounds_bottom
}

fn intrabc_luma_source_envelope(
    target: PlaneRect,
    block_mv: IntrabcBlockVector,
    tile_offset: ByteOffset,
) -> Result<PlaneRect> {
    let bottom_border = usize::from(block_mv.row & 7 != 0);
    let right_border = usize::from(block_mv.col & 7 != 0);
    let delta_row = block_mv.row >> 3;
    let delta_col = block_mv.col >> 3;
    let source_x = i32::try_from(target.x())
        .ok()
        .and_then(|value| value.checked_add(delta_col))
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let source_y = i32::try_from(target.y())
        .ok()
        .and_then(|value| value.checked_add(delta_row))
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    if source_x < 0 || source_y < 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    let source_width = target
        .width()
        .checked_add(right_border)
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    let source_height = target
        .height()
        .checked_add(bottom_border)
        .ok_or_else(|| intrabc_geometry_error(tile_offset))?;
    PlaneRect::new(
        usize::try_from(source_x).map_err(|_| intrabc_geometry_error(tile_offset))?,
        usize::try_from(source_y).map_err(|_| intrabc_geometry_error(tile_offset))?,
        source_width,
        source_height,
    )
    .map_err(|_| intrabc_geometry_error(tile_offset))
}

#[cfg(test)]
fn superblock_samples(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<i32> {
    let partition = sequence.partition.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config",
        )
    })?;
    Ok(match partition.seq_sb_size() {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 128,
    })
}

fn intrabc_use_is_coded(
    core: &FrameHeaderCore,
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
) -> bool {
    let region_allows_intrabc = core.frame_is_intra != Some(false) || block.mixed_region;
    frame_allows_intrabc(core)
        && region_allows_intrabc
        && !block.is_chroma_part
        && n4w <= 64 / MI_SIZE
        && n4h <= 64 / MI_SIZE
        && block.b_size != BLOCK_64X64
}

pub(crate) fn frame_allows_intrabc(core: &FrameHeaderCore) -> bool {
    core.allow_intrabc == Some(true)
        || core
            .inter
            .as_ref()
            .is_some_and(|inter| inter.allow_intrabc == Some(true))
}

fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| super::selectable_symbol_read_error(tile_offset))
}

fn max_bvp_drl_bits_minus_1(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<u32> {
    if let Some(max) = core
        .intrabc
        .as_ref()
        .and_then(|params| params.max_bvp_drl_bits_minus_1)
    {
        return Ok(max);
    }
    let inter = sequence.inter.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_inter_config",
        )
    })?;
    Ok(inter.seq_max_bvp_drl_bits_minus_1)
}

fn intrabc_sb_size4(
    sequence: &SequenceHeader,
    frame_is_intra_only: bool,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let seq_sb_size = intra_capped_seq_sb_size(sequence, tile_offset)?;
    Ok(match seq_sb_size {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 => 32,
        SuperblockSize::Block256x256 if frame_is_intra_only => 32,
        SuperblockSize::Block256x256 => 64,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "intrabc_records_tests.rs"]
mod tests;
