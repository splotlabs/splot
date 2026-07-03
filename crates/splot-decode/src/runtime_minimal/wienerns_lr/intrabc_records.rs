// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::ops::Range;

use splot_core::headers::frame::{FrameHeaderCore, MvPrecision};
use splot_core::headers::sequence::{DrlReorder, SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::{PlaneRect, PlaneSize};

use crate::error::Result;
use crate::runtime_minimal::inter::mv_scaling::{PlaneScaling, derive_plane_scaling};
use crate::tile_payload::{DecodeBlockFrontier, TileCdfSelector, TileCdfSubset};

use super::super::inter::{
    Mv,
    read_mv::{
        MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL, MvReadConfig, mv_clamp_to_integer,
        read_newmv_block_mvd_with_config,
    },
};
use super::intrabc_ref_mv_stack::{
    DrlReorderMode, IntrabcRefMvBank, IntrabcStackAdmission, IntrabcStackGeometry,
    SpatialIntrabcScan, SpatialScanGeometry, build_intrabc_ref_mv_stack,
    intrabc_ref_stack_admission, spatial_intrabc_scan,
};
use super::recon::WienerNsLrReconSink;
use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;

/// Result of the §5.20.5.3 `use_intrabc` / `read_skip` prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct IntrabcUseSkip {
    pub(in crate::runtime_minimal) use_intrabc: bool,
    pub(in crate::runtime_minimal) skip_flag: bool,
}

/// Luma/shared mode-info facts retained by the transform-record handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct IntrabcBlockPrelude {
    pub(in crate::runtime_minimal) use_intrabc: bool,
    pub(in crate::runtime_minimal) is_inter: bool,
    pub(in crate::runtime_minimal) skip_flag: bool,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "retained IntrABC mode-info facts are part of the parse/prediction handoff"
        )
    )]
    pub(in crate::runtime_minimal) intrabc: Option<IntrabcInfo>,
}

impl IntrabcBlockPrelude {
    pub(in crate::runtime_minimal) const fn from_use_skip(
        use_skip: IntrabcUseSkip,
        intrabc: Option<IntrabcInfo>,
    ) -> Self {
        Self {
            use_intrabc: use_skip.use_intrabc,
            is_inter: use_skip.use_intrabc,
            skip_flag: use_skip.skip_flag,
            intrabc,
        }
    }
}

/// Minimal block facts needed by the local IntrABC syntax handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockContext {
    row: usize,
    col: usize,
    b_size: usize,
    is_chroma_part: bool,
    mixed_region: bool,
}

impl IntrabcBlockContext {
    pub(super) fn from_frontier(frontier: &DecodeBlockFrontier) -> Self {
        Self {
            row: frontier.r,
            col: frontier.c,
            b_size: frontier.b_size.index(),
            is_chroma_part: frontier.is_chroma_part(),
            mixed_region: frontier.is_mixed_region(),
        }
    }

    #[cfg(test)]
    const fn new(row: usize, col: usize, b_size: usize, is_chroma_part: bool) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
            mixed_region: true,
        }
    }

    #[cfg(test)]
    const fn new_with_mixed_region(
        row: usize,
        col: usize,
        b_size: usize,
        is_chroma_part: bool,
        mixed_region: bool,
    ) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
            mixed_region,
        }
    }
}

/// Current block geometry for §5.20.5.3 IntrABC context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct IntrabcBlockGeometry {
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
}

impl IntrabcBlockGeometry {
    pub(in crate::runtime_minimal) fn from_frontier(
        frontier: &DecodeBlockFrontier,
        n4w: usize,
        n4h: usize,
    ) -> Self {
        Self {
            block: IntrabcBlockContext::from_frontier(frontier),
            n4w,
            n4h,
        }
    }

    #[cfg(test)]
    const fn new(block: IntrabcBlockContext, n4w: usize, n4h: usize) -> Self {
        Self { block, n4w, n4h }
    }
}

/// Retained §5.20.5.4 IntrABC syntax and vector facts.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained IntrABC mode-info facts are part of the parse/prediction handoff"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct IntrabcInfo {
    pub(super) intrabc_mode: u8,
    pub(super) ref_mv_idx: usize,
    pub(super) mv_precision: u8,
    pub(super) block_mv: IntrabcBlockVector,
}

/// Retained IntrABC block vector in eighth-pel units.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained IntrABC block-vector facts are part of the parse/prediction handoff"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockVector {
    pub(super) row: i32,
    pub(super) col: i32,
}

/// Checked luma current-frame prediction geometry derived from an IntrABC block vector.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained prediction geometry is part of the IntrABC reconstruction handoff"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct IntrabcPredictionGeometry {
    pub(in crate::runtime_minimal) scaling: PlaneScaling,
    pub(in crate::runtime_minimal) fractional: bool,
    /// The integer-copy source rectangle. For fractional vectors, the bilinear
    /// predictor reads through `scaling` and §7.13.3.18 reference clipping instead.
    pub(in crate::runtime_minimal) source: PlaneRect,
    pub(in crate::runtime_minimal) target: PlaneRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcInfoSyntax {
    intrabc_mode: usize,
    ref_mv_idx: usize,
    mv_precision: u8,
    max_bvp_drl_bits_minus_1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcLumaPredictionDomain {
    storage: PlaneSize,
    tile_bounds: PlaneRect,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
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
}

impl IntrabcNeighborContext {
    const fn same_sb_row(self) -> bool {
        matches!(self, Self::UseIntrabc)
    }

    const fn max(self) -> usize {
        match self {
            Self::UseIntrabc => INTRABC_CONTEXT_MAX,
            Self::Skip => SKIP_CONTEXT_MAX,
        }
    }

    const fn matches(self, facts: IntrabcBlockFacts) -> bool {
        match self {
            Self::UseIntrabc => facts.use_intrabc,
            Self::Skip => facts.skip_flag,
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
        PlaneRect::new(x, y, width, height).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })
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

/// Tile-local neighbour state for IntrABC and skip contexts used by §8.3.2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct TileIntrabcPreludeState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    values: Vec<Option<IntrabcBlockFacts>>,
    bank: IntrabcRefMvBank,
    enable_refmvbank: bool,
    drl_reorder: DrlReorderMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockFacts {
    use_intrabc: bool,
    skip_flag: bool,
    block_mv: Option<IntrabcBlockVector>,
}

impl TileIntrabcPreludeState {
    pub(in crate::runtime_minimal) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let values_len = mi_rows.checked_mul(mi_cols).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_grid_overflow",
            )
        })?;
        let sb_size4 = intra_sb_size4(sequence, tile_offset)?;
        let enable_refmvbank = sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank);
        let drl_reorder = match sequence.inter.as_ref().map(|inter| inter.drl_reorder) {
            Some(DrlReorder::Always) => DrlReorderMode::Always,
            Some(DrlReorder::Constraint) => DrlReorderMode::Constraint,
            Some(DrlReorder::Disabled) | None => DrlReorderMode::Disabled,
        };
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4,
            values: vec![None; values_len],
            bank: IntrabcRefMvBank::new(sb_size4),
            enable_refmvbank,
            drl_reorder,
        })
    }

    pub(in crate::runtime_minimal) fn record_block(
        &mut self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        prelude: IntrabcBlockPrelude,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let block_mv = prelude.intrabc.map(|info| info.block_mv);
        let facts = IntrabcBlockFacts {
            use_intrabc: prelude.use_intrabc,
            skip_flag: prelude.skip_flag,
            block_mv,
        };
        let area = self.clipped_record_area(row, col, n4w, n4h, tile_offset)?;
        for r in area.rows {
            for c in area.cols.clone() {
                let index = self.index(r, c, tile_offset)?;
                self.values[index] = Some(facts);
            }
        }
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

    /// Resets the §7.12.2 ref-MV bank when entering a new superblock row.
    pub(in crate::runtime_minimal) fn prepare_for_block(&mut self, row: usize, col: usize) {
        if self.enable_refmvbank {
            self.bank.enter_block_superblock(row, col);
        }
    }

    pub(in crate::runtime_minimal) fn intrabc_ctx(
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

    fn clipped_record_area(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<IntrabcMiArea> {
        IntrabcMiArea::clipped(row, col, n4w, n4h, self.mi_rows, self.mi_cols, tile_offset)
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
            if r >= self.mi_rows || c >= self.mi_cols {
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
        Ok(self.values[index])
    }

    fn index(&self, row: usize, col: usize, tile_offset: ByteOffset) -> Result<usize> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
            ));
        }
        row.checked_mul(self.mi_cols)
            .and_then(|start| start.checked_add(col))
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_overflow",
                )
            })
    }

    fn spatial_intrabc_scan(&self, geometry: IntrabcBlockGeometry) -> SpatialIntrabcScan {
        let scan_geometry = SpatialScanGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            sb_size4: self.sb_size4,
        };
        spatial_intrabc_scan(
            scan_geometry,
            |row, col| self.block_vector_at(row, col),
            |row, col| self.is_mi_coded(row, col),
        )
    }

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

    fn facts_at(&self, row: usize, col: usize) -> Option<IntrabcBlockFacts> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return None;
        }
        self.values
            .get(row.checked_mul(self.mi_cols)?.checked_add(col)?)
            .copied()
            .flatten()
    }

    fn bank(&self) -> &IntrabcRefMvBank {
        &self.bank
    }
}

pub(in crate::runtime_minimal) fn read_intrabc_use_and_skip(
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

/// Reads one §5.20.5.3 `use_intrabc` block's mode info and, when a reconstruction
/// `sink` is attached, reconstructs its §7.13.3.18 displaced predictor.
///
/// The gated sink admits ONLY the §6.19-proven same-superblock INTEGER-DV copy
/// (`intrabc_dv_proven_valid`). The full-recon diagnostic reconstructs every block
/// in decode order over a bounds-checked source (`source.is_within(domain.storage)`),
/// so it trusts the decoded DV: an integer DV copies the (possibly cross-SB) source
/// and a fractional DV runs the §7.13.3.18 bilinear predictor.
#[allow(clippy::too_many_arguments)]
pub(in crate::runtime_minimal) fn read_intrabc_info(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    skip_flag: bool,
    sink: Option<&mut WienerNsLrReconSink<u16>>,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    let pred_mv =
        ensure_intrabc_ref_stack_supported(state, sequence, geometry, syntax, tile_offset)?;
    let info =
        finish_intrabc_info_record(cdfs, symbols, sequence, core, syntax, pred_mv, tile_offset)?;
    let prediction = derive_intrabc_luma_prediction_geometry(core, geometry, info, tile_offset)?;
    if let Some(sink) = sink {
        let admit = if sink.is_full_recon() {
            true
        } else {
            intrabc_dv_proven_valid(sequence, core, geometry, info, tile_offset)?
        };
        if admit {
            sink.reconstruct_intrabc_block(
                prediction.source,
                prediction.target,
                prediction.scaling,
                prediction.fractional,
                skip_flag,
                tile_offset,
            )?;
        }
    }
    Ok(info)
}

#[cfg(test)]
fn read_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    use super::intrabc_ref_mv_stack::build_intrabc_ref_mv_stack;
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    let stack_geometry = IntrabcStackGeometry {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        n4w: geometry.n4w,
        n4h: geometry.n4h,
        sb_samples: superblock_samples(sequence, tile_offset)?,
        frame_w: i32::MAX,
        frame_h: i32::MAX,
        max_bvp_drl_bits_minus_1: syntax.max_bvp_drl_bits_minus_1,
    };
    let stack = build_intrabc_ref_mv_stack(&IntrabcRefMvBank::new(0), stack_geometry, false, &[]);
    let pred_mv = *stack.get(syntax.ref_mv_idx).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx_out_of_range",
        )
    })?;
    finish_intrabc_info_record(cdfs, symbols, sequence, core, syntax, pred_mv, tile_offset)
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
    if intrabc_mode > 1 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mode",
        ));
    }

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
        if precision > 1 {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_precision",
            ));
        }
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
    trace_intrabc_ref_stack(
        state,
        stack_geometry,
        &spatial,
        syntax.ref_mv_idx,
        tile_offset,
    );
    match intrabc_ref_stack_admission(
        state.bank(),
        stack_geometry,
        &spatial,
        state.enable_refmvbank,
        state.drl_reorder,
        syntax.ref_mv_idx,
    ) {
        IntrabcStackAdmission::Admit { selected } => Ok(selected),
        IntrabcStackAdmission::Defer => Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack",
        )),
    }
}

fn trace_intrabc_ref_stack(
    state: &TileIntrabcPreludeState,
    stack_geometry: IntrabcStackGeometry,
    spatial: &SpatialIntrabcScan,
    ref_mv_idx: usize,
    tile_offset: ByteOffset,
) {
    if !crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRABC_REF_STACK") {
        return;
    }
    let mut nearest: Vec<_> = spatial.candidates.clone();
    if state.drl_reorder.use_sort(nearest.len()) && nearest.len() > 1 {
        super::intrabc_ref_mv_stack::sort_nearest_max_weight_to_slot0(&mut nearest);
    }
    let sorted: Vec<Mv> = nearest.iter().map(|entry| entry.mv).collect();
    let stack = build_intrabc_ref_mv_stack(
        state.bank(),
        stack_geometry,
        state.enable_refmvbank,
        &sorted,
    );
    eprintln!(
        "intrabc ref_stack offset={} mi=({}, {}) n4={}x{} ref_mv_idx={} spatial_defer={} spatial={:?} sorted={:?} bank={:?} stack={:?} enable_refmvbank={} drl_reorder={:?}",
        tile_offset.get(),
        stack_geometry.mi_row,
        stack_geometry.mi_col,
        stack_geometry.n4w,
        stack_geometry.n4h,
        ref_mv_idx,
        spatial.defer,
        spatial.candidates,
        sorted,
        state.bank().entries(),
        stack,
        state.enable_refmvbank,
        state.drl_reorder,
    );
}

#[allow(clippy::too_many_arguments)]
fn finish_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    syntax: IntrabcInfoSyntax,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let block_mv = assign_intrabc_mv(
        cdfs,
        symbols,
        syntax.intrabc_mode,
        syntax.mv_precision,
        pred_mv,
        tile_offset,
    )?;
    if core.frame_is_intra == Some(true)
        && core.allow_screen_content_tools == Some(true)
        && sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_bawp)
    {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_morph_pred",
        ));
    }
    Ok(IntrabcInfo {
        intrabc_mode: u8::try_from(syntax.intrabc_mode).unwrap_or(1),
        ref_mv_idx: syntax.ref_mv_idx,
        mv_precision: syntax.mv_precision,
        block_mv,
    })
}

fn assign_intrabc_mv(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    intrabc_mode: usize,
    mv_precision: u8,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcBlockVector> {
    let block_mv = if intrabc_mode == 0 {
        let diff = read_newmv_block_mvd_with_config(
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
        })?;
        Mv {
            row: mv_clamp_to_integer(pred_mv.row + diff.row),
            col: mv_clamp_to_integer(pred_mv.col + diff.col),
        }
    } else {
        pred_mv
    };
    Ok(block_mv.into())
}

pub(in crate::runtime_minimal) fn derive_intrabc_luma_prediction_geometry(
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
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRABC_GEOMETRY") {
            eprintln!(
                "intrabc geometry source_bounds offset={} mi=({}, {}) n4={}x{} block_px=({}, {}) {}x{} mv=({}, {}) target=({}, {}) {}x{} source=({}, {}) {}x{} storage={}x{} tile=({}, {}) {}x{}",
                tile_offset.get(),
                geometry.block.row,
                geometry.block.col,
                geometry.n4w,
                geometry.n4h,
                block.x,
                block.y,
                block.width,
                block.height,
                info.block_mv.row,
                info.block_mv.col,
                target.x(),
                target.y(),
                target.width(),
                target.height(),
                source.x(),
                source.y(),
                source.width(),
                source.height(),
                domain.storage.width(),
                domain.storage.height(),
                domain.tile_bounds.x(),
                domain.tile_bounds.y(),
                domain.tile_bounds.width(),
                domain.tile_bounds.height(),
            );
        }
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    if !fractional && rects_overlap(source, target) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity",
        ));
    }
    let scaling = derive_plane_scaling(
        checked_i64_from_usize(block.x, tile_offset)?,
        checked_i64_from_usize(block.y, tile_offset)?,
        i64::from(info.block_mv.row),
        i64::from(info.block_mv.col),
        0,
        0,
        domain.ref_mi_cols,
        domain.ref_mi_rows,
        checked_i64_from_usize(block.width, tile_offset)?,
        checked_i64_from_usize(block.height, tile_offset)?,
    );
    Ok(IntrabcPredictionGeometry {
        scaling,
        fractional,
        source,
        target,
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
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_bottom = tile_y
        .checked_add(domain.tile_bounds.height())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let nominal_right = target_x.checked_add(width).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let nominal_bottom = target_y.checked_add(height).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
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
    PlaneRect::new(target_x, target_y, eff_width, eff_height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
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
    let tile_info = core.tile_info.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let mi_cols = tile_info.mi_col_starts.last().copied().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let mi_rows = tile_info.mi_row_starts.last().copied().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    if mi_cols == 0 || mi_rows == 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        ));
    }
    let width = checked_mi_u32_to_luma(mi_cols, tile_offset)?;
    let height = checked_mi_u32_to_luma(mi_rows, tile_offset)?;
    let storage = PlaneSize::new(width, height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
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
        ref_mi_cols: i64::from(mi_cols),
        ref_mi_rows: i64::from(mi_rows),
    })
}

fn resolve_allow_local_intrabc(core: &FrameHeaderCore) -> bool {
    core.intrabc
        .as_ref()
        .is_some_and(|params| params.allow_intrabc && params.allow_local_intrabc != Some(false))
}

fn resolve_allow_global_intrabc(core: &FrameHeaderCore) -> bool {
    core.intrabc
        .as_ref()
        .is_some_and(|params| params.allow_intrabc && params.allow_global_intrabc == Some(true))
}

fn intrabc_dv_proven_valid(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if info.block_mv.row & 7 != 0 || info.block_mv.col & 7 != 0 {
        return Ok(false);
    }
    if !resolve_allow_local_intrabc(core) {
        return Ok(false);
    }
    let sb_samples = superblock_samples(sequence, tile_offset)?;
    let block = IntrabcBlockPixels::from_geometry(geometry, tile_offset)?;
    let sb_size = usize::try_from(sb_samples).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let local_valid = local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w: block.width,
        block_h: block.height,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size,
    });
    if local_valid {
        return Ok(true);
    }
    if core.frame_is_intra != Some(true) || !resolve_allow_global_intrabc(core) {
        return Ok(false);
    }
    let global_sb_size = global_superblock_samples(sequence, tile_offset)?;
    let total_sb64_per_row = intrabc_tile_total_sb64_per_row(core, geometry, tile_offset)?;
    Ok(global_intrabc_range_valid(IntrabcGlobalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w: block.width,
        block_h: block.height,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size: global_sb_size,
        total_sb64_per_row,
    }))
}

fn intrabc_tile_total_sb64_per_row(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let tile_info = core.tile_info.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let (tile_col_start, tile_col_end) = tile_interval_for_block(
        &tile_info.mi_col_starts,
        geometry.block.col,
        geometry.n4w,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let tile_mi_cols =
        i64::try_from(tile_col_end.saturating_sub(tile_col_start)).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    if tile_mi_cols <= 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        ));
    }
    Ok(((tile_mi_cols - 1) >> 4) + 1)
}

fn global_superblock_samples(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 => 128,
        SuperblockSize::Block256x256 => 256,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcLocalRangeInputs {
    mi_row: usize,
    mi_col: usize,
    block_w: usize,
    block_h: usize,
    dv_row: i32,
    dv_col: i32,
    sb_size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcSourceBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

fn intrabc_source_bounds(
    mi_row: usize,
    mi_col: usize,
    block_w: i64,
    block_h: i64,
    dv_row: i64,
    dv_col: i64,
) -> IntrabcSourceBounds {
    let origin_y = mi_row as i64 * MI_SIZE as i64;
    let origin_x = mi_col as i64 * MI_SIZE as i64;
    let top_edge = origin_y * 8 + dv_row;
    let left_edge = origin_x * 8 + dv_col;
    let bottom_edge = (origin_y + block_h) * 8 + dv_row;
    let right_edge = (origin_x + block_w) * 8 + dv_col;

    IntrabcSourceBounds {
        left: left_edge >> 3,
        top: top_edge >> 3,
        right: (right_edge >> 3) - 1,
        bottom: (bottom_edge >> 3) - 1,
    }
}

fn local_intrabc_range_valid(inputs: IntrabcLocalRangeInputs) -> bool {
    let bw = inputs.block_w as i64;
    let bh = inputs.block_h as i64;
    let dv_row = i64::from(inputs.dv_row);
    let dv_col = i64::from(inputs.dv_col);
    let sb_size_log2 = match inputs.sb_size {
        64 => 6,
        128 => 7,
        256 => 8,
        _ => return false,
    };

    let source = intrabc_source_bounds(inputs.mi_row, inputs.mi_col, bw, bh, dv_row, dv_col);
    let act_left_x = inputs.mi_col as i64 * MI_SIZE as i64;
    let act_top_y = inputs.mi_row as i64 * MI_SIZE as i64;

    if ((dv_col >> 3) + bw) > 0 && ((dv_row >> 3) + bh) > 0 {
        return false;
    }
    let act_sb_col = act_left_x >> sb_size_log2;
    let act_sb_row = act_top_y >> sb_size_log2;
    (source.left >> sb_size_log2) == act_sb_col
        && (source.right >> sb_size_log2) == act_sb_col
        && (source.top >> sb_size_log2) == act_sb_row
        && (source.bottom >> sb_size_log2) == act_sb_row
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcGlobalRangeInputs {
    mi_row: usize,
    mi_col: usize,
    block_w: usize,
    block_h: usize,
    dv_row: i32,
    dv_col: i32,
    sb_size: usize,
    total_sb64_per_row: i64,
}

const INTRABC_DELAY_SB64: i64 = 4;
const LOG2_MI_PER_64: i64 = 4;
const MI_SIZE_LOG2: i64 = 2;

fn global_intrabc_range_valid(inputs: IntrabcGlobalRangeInputs) -> bool {
    let bw = inputs.block_w as i64;
    let bh = inputs.block_h as i64;
    let dv_row = i64::from(inputs.dv_row);
    let dv_col = i64::from(inputs.dv_col);
    let mi_row = inputs.mi_row as i64;
    let mi_col = inputs.mi_col as i64;
    let mi = i64::from(MI_SIZE as u32);
    let mib_size_log2: i64 = match inputs.sb_size {
        64 => 4,
        128 => 5,
        256 => 6,
        _ => return false,
    };
    let sb_size = i64::from(inputs.sb_size as u32);

    let source = intrabc_source_bounds(inputs.mi_row, inputs.mi_col, bw, bh, dv_row, dv_col);

    let active_sb_row = mi_row >> mib_size_log2;
    let active_sb64_col = mi_col >> LOG2_MI_PER_64;
    let src_sb_row = source.bottom >> (mib_size_log2 + MI_SIZE_LOG2);
    let src_sb64_col = source.right >> (LOG2_MI_PER_64 + MI_SIZE_LOG2);
    let active_sb64_row = (mi_row * mi) >> (LOG2_MI_PER_64 + MI_SIZE_LOG2);

    let active_sb64 = active_sb_row * inputs.total_sb64_per_row + active_sb64_col;
    let src_sb64 = src_sb_row * inputs.total_sb64_per_row + src_sb64_col;

    let gradient = 1 + INTRABC_DELAY_SB64 + i64::from(sb_size > 64) + 2 * i64::from(sb_size > 128);
    let wf_offset = gradient * (active_sb_row - src_sb_row);

    let is_bottom_left = sb_size == 128 && (active_sb64_col & 1) == 0 && (active_sb64_row & 1) == 1;
    let residuals: &[i64] = if is_bottom_left { &[0, -1] } else { &[0] };
    residuals.iter().all(|&sb_64_residual| {
        if src_sb64 >= active_sb64 - INTRABC_DELAY_SB64 - sb_64_residual {
            return false;
        }
        if src_sb_row > active_sb_row
            || src_sb64_col >= active_sb64_col - INTRABC_DELAY_SB64 - sb_64_residual + wf_offset
        {
            return false;
        }
        true
    })
}

fn tile_interval_for_block(
    starts: &[u32],
    block_start: usize,
    block_len: usize,
    bounds_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    checked_mi_end(
        block_start,
        block_len,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        tile_offset,
    )?;
    for window in starts.windows(2) {
        let start = usize::try_from(window[0]).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
        let end = usize::try_from(window[1]).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
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
    mi.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
}

fn checked_mi_u32_to_luma(mi: u32, tile_offset: ByteOffset) -> Result<usize> {
    checked_mi_to_luma(
        usize::try_from(mi).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        tile_offset,
    )
}

fn checked_i64_from_usize(value: usize, tile_offset: ByteOffset) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
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

fn rects_overlap(first: PlaneRect, second: PlaneRect) -> bool {
    let Some(first_right) = first.x().checked_add(first.width()) else {
        return true;
    };
    let Some(first_bottom) = first.y().checked_add(first.height()) else {
        return true;
    };
    let Some(second_right) = second.x().checked_add(second.width()) else {
        return true;
    };
    let Some(second_bottom) = second.y().checked_add(second.height()) else {
        return true;
    };
    first.x() < second_right
        && first_right > second.x()
        && first.y() < second_bottom
        && first_bottom > second.y()
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
    let source_x = i64::try_from(target.x())
        .ok()
        .and_then(|value| value.checked_add(i64::from(delta_col)))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let source_y = i64::try_from(target.y())
        .ok()
        .and_then(|value| value.checked_add(i64::from(delta_row)))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    if source_x < 0 || source_y < 0 {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRABC_GEOMETRY") {
            eprintln!(
                "intrabc geometry source_negative offset={} target=({}, {}) {}x{} mv=({}, {}) source=({}, {})",
                tile_offset.get(),
                target.x(),
                target.y(),
                target.width(),
                target.height(),
                block_mv.row,
                block_mv.col,
                source_x,
                source_y,
            );
        }
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    let source_width = target.width().checked_add(right_border).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let source_height = target.height().checked_add(bottom_border).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    PlaneRect::new(
        usize::try_from(source_x).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        usize::try_from(source_y).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        source_width,
        source_height,
    )
    .map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
}

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
    let allow_intrabc = core.allow_intrabc == Some(true)
        || core
            .inter
            .as_ref()
            .is_some_and(|inter| inter.allow_intrabc == Some(true));
    let region_allows_intrabc = core.frame_is_intra != Some(false) || block.mixed_region;
    allow_intrabc
        && region_allows_intrabc
        && !block.is_chroma_part
        && n4w <= 64 / MI_SIZE
        && n4h <= 64 / MI_SIZE
        && block.b_size != BLOCK_64X64
}

fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_symbol_read",
            )
        })
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

fn intra_sb_size4(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 32,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "intrabc_records_tests.rs"]
mod tests;
