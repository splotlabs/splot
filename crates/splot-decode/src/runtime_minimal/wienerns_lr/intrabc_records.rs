// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded IntrABC syntax handoff for the ac0ej3 selectable transform-record frontier.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
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
use super::recon::WienerNsLrReconSink;
use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;
const INTRABC_DELAY_PIXELS: i32 = 256;

/// Result of the §5.20.5.3 `use_intrabc` / `read_skip` prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcUseSkip {
    pub(super) use_intrabc: bool,
    pub(super) skip_flag: bool,
}

/// Luma/shared mode-info facts retained by the transform-record handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockPrelude {
    pub(super) use_intrabc: bool,
    pub(super) is_inter: bool,
    pub(super) skip_flag: bool,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "retained IntrABC mode-info facts are consumed by tests until prediction uses them"
        )
    )]
    pub(super) intrabc: Option<IntrabcInfo>,
}

impl IntrabcBlockPrelude {
    pub(super) const fn from_use_skip(
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
}

impl IntrabcBlockContext {
    pub(super) fn from_frontier(frontier: &DecodeBlockFrontier) -> Self {
        Self {
            row: frontier.r,
            col: frontier.c,
            b_size: frontier.b_size.index(),
            is_chroma_part: frontier.is_chroma_part(),
        }
    }

    #[cfg(test)]
    const fn new(row: usize, col: usize, b_size: usize, is_chroma_part: bool) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
        }
    }
}

/// Current block geometry for §5.20.5.3 IntrABC context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockGeometry {
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
}

impl IntrabcBlockGeometry {
    pub(super) fn from_frontier(frontier: &DecodeBlockFrontier, n4w: usize, n4h: usize) -> Self {
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

/// Bounded §5.20.5.4 facts that can be parsed without current-frame prediction.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained IntrABC mode-info facts are consumed by tests until prediction uses them"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcInfo {
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
        reason = "retained IntrABC block-vector facts are consumed by tests until prediction uses them"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockVector {
    pub(super) row: i32,
    pub(super) col: i32,
}

/// Checked luma current-frame copy geometry derived from an IntrABC block vector.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "live ac0ej3 path stops at the missing CurrFrame frontier until samples are populated"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcPredictionGeometry {
    pub(super) scaling: PlaneScaling,
    pub(super) source: PlaneRect,
    pub(super) target: PlaneRect,
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

impl From<Mv> for IntrabcBlockVector {
    fn from(value: Mv) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

/// Tile-local neighbour state for IntrABC and skip contexts used by §8.3.2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TileIntrabcPreludeState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    values: Vec<Option<IntrabcBlockFacts>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockFacts {
    use_intrabc: bool,
    skip_flag: bool,
}

impl TileIntrabcPreludeState {
    pub(super) fn new(
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
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4: intra_sb_size4(sequence, tile_offset)?,
            values: vec![None; values_len],
        })
    }

    pub(super) fn record_block(
        &mut self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        prelude: IntrabcBlockPrelude,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let facts = IntrabcBlockFacts {
            use_intrabc: prelude.use_intrabc,
            skip_flag: prelude.skip_flag,
        };
        let row_end = row.checked_add(n4h).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_row_overflow",
            )
        })?;
        let col_end = col.checked_add(n4w).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_col_overflow",
            )
        })?;
        if row_end > self.mi_rows || col_end > self.mi_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_bounds",
            ));
        }
        for r in row..row_end {
            for c in col..col_end {
                let index = self.index(r, c, tile_offset)?;
                self.values[index] = Some(facts);
            }
        }
        Ok(())
    }

    pub(super) fn intrabc_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let mut ctx = 0usize;
        for (r, c) in self.neighbor_positions(row, col, n4w, n4h, true, tile_offset)? {
            if self
                .value(r, c, tile_offset)?
                .is_some_and(|facts| facts.use_intrabc)
            {
                ctx += 1;
            }
        }
        Ok(ctx.min(INTRABC_CONTEXT_MAX))
    }

    fn skip_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let mut ctx = 0usize;
        for (r, c) in self.neighbor_positions(row, col, n4w, n4h, false, tile_offset)? {
            if self
                .value(r, c, tile_offset)?
                .is_some_and(|facts| facts.skip_flag)
            {
                ctx += 1;
            }
        }
        Ok(ctx.min(SKIP_CONTEXT_MAX))
    }

    fn neighbor_positions(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        same_sb_row: bool,
        tile_offset: ByteOffset,
    ) -> Result<Vec<(usize, usize)>> {
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
        let mut positions = Vec::new();
        for candidate in candidates.into_iter().flatten() {
            let (r, c) = candidate;
            if r >= self.mi_rows || c >= self.mi_cols {
                continue;
            }
            if same_sb_row && r / self.sb_size4 != row / self.sb_size4 {
                continue;
            }
            positions.push(candidate);
            if positions.len() == 2 {
                break;
            }
        }
        Ok(positions)
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

    fn has_recorded_intrabc(&self) -> bool {
        self.values
            .iter()
            .any(|value| value.is_some_and(|facts| facts.use_intrabc))
    }
}

pub(super) fn read_intrabc_use_and_skip(
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
pub(super) fn read_intrabc_info(
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
    ensure_intrabc_ref_stack_supported(state, tile_offset)?;
    let info =
        finish_intrabc_info_record(cdfs, symbols, sequence, core, geometry, syntax, tile_offset)?;
    let prediction = derive_intrabc_luma_prediction_geometry(core, geometry, info, tile_offset)?;
    // §7.13.3.18 IntrABC luma prediction: with the block-vector geometry bounds-checked
    // above, an attached reconstruction sink copies the displaced predictor rectangle
    // from the partially-built `CurrFrame` (gated to the proven integer-vector skip
    // subset inside the sink). The §6.19.7.12 `is_mv_valid` conformance predicate must
    // ALSO hold before the copy: the geometry derivation proves the tile-edge clause,
    // and `intrabc_dv_proven_valid` proves the global-intrabc wavefront clause (the
    // local-IBC-buffer clause needs runtime buffer state splot does not track, so it is
    // conservatively deferred). An invalid (or not-provably-valid) DV defers the copy —
    // never marks an out-of-buffer reference bit-exact. The walk then STILL fails closed
    // at the `currframe` frontier so the PUBLIC decode path (which threads no sink) stays
    // byte-identical: it emits no frame, and the test driver swallows only this one
    // reason — the sink retains the reconstructed IntrABC target for the region test.
    if let Some(sink) = sink
        && intrabc_dv_proven_valid(sequence, core, geometry, info, tile_offset)?
    {
        sink.reconstruct_intrabc_block(
            prediction.source,
            prediction.target,
            skip_flag,
            tile_offset,
        )?;
    }
    Err(wienerns_lr_selectable_transform_record_error_reason(
        tile_offset,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_currframe_samples",
    ))
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
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    finish_intrabc_info_record(cdfs, symbols, sequence, core, geometry, syntax, tile_offset)
}

fn read_intrabc_info_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfoSyntax> {
    let force_integer_mv = core.force_integer_mv.ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_mv_precision",
        )
    })?;
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

fn ensure_intrabc_ref_stack_supported(
    state: &TileIntrabcPreludeState,
    tile_offset: ByteOffset,
) -> Result<()> {
    if state.has_recorded_intrabc() {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack",
        ));
    }
    Ok(())
}

fn finish_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    syntax: IntrabcInfoSyntax,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    // The bounded fallback list is only valid after §7.12.2 `find_mv_stack(0)`
    // has no spatial or ref-MV-bank candidates. The live path proves that from
    // `TileIntrabcPreludeState` before reaching this shared test helper.
    let block_mv = assign_intrabc_mv(
        cdfs,
        symbols,
        sequence,
        geometry,
        syntax.intrabc_mode,
        syntax.ref_mv_idx,
        syntax.mv_precision,
        syntax.max_bvp_drl_bits_minus_1,
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

#[allow(clippy::too_many_arguments)]
fn assign_intrabc_mv(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    intrabc_mode: usize,
    ref_mv_idx: usize,
    mv_precision: u8,
    max_bvp_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<IntrabcBlockVector> {
    let candidates = intrabc_ref_stack(sequence, geometry, max_bvp_drl_bits_minus_1, tile_offset)?;
    let pred_mv = *candidates.get(ref_mv_idx).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx_out_of_range",
        )
    })?;
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

pub(super) fn derive_intrabc_luma_prediction_geometry(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<IntrabcPredictionGeometry> {
    let domain = intrabc_luma_prediction_domain(core, geometry, tile_offset)?;
    let width = geometry.n4w.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let height = geometry.n4h.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let target_x = geometry.block.col.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let target_y = geometry.block.row.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let target = PlaneRect::new(target_x, target_y, width, height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    if !target.is_within(domain.storage) || !rect_is_within_rect(target, domain.tile_bounds) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        ));
    }
    let source = intrabc_luma_source_envelope(target, info.block_mv, tile_offset)?;
    if !source.is_within(domain.storage) || !rect_is_within_rect(source, domain.tile_bounds) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    if rects_overlap(source, target) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity",
        ));
    }
    let scaling = derive_plane_scaling(
        i64::try_from(target_x).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::try_from(target_y).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::from(info.block_mv.row),
        i64::from(info.block_mv.col),
        0,
        0,
        domain.ref_mi_cols,
        domain.ref_mi_rows,
        i64::try_from(width).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::try_from(height).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
    );
    Ok(IntrabcPredictionGeometry {
        scaling,
        source,
        target,
    })
}

fn intrabc_luma_prediction_domain(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcLumaPredictionDomain> {
    // Keep the missing-frame-size diagnostic distinct, but derive the luma
    // storage domain from the padded MI sentinels used by §7.13.3.18.
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
    let width = usize::try_from(mi_cols)
        .ok()
        .and_then(|cols| cols.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let height = usize::try_from(mi_rows)
        .ok()
        .and_then(|rows| rows.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let storage = PlaneSize::new(width, height).map_err(|_| {
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
    let (tile_row_start, tile_row_end) = tile_interval_for_block(
        &tile_info.mi_row_starts,
        geometry.block.row,
        geometry.n4h,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let tile_x = tile_col_start.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let tile_y = tile_row_start.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let tile_width = tile_col_end
        .checked_sub(tile_col_start)
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_height = tile_row_end
        .checked_sub(tile_row_start)
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_bounds = PlaneRect::new(tile_x, tile_y, tile_width, tile_height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    Ok(IntrabcLumaPredictionDomain {
        storage,
        tile_bounds,
        ref_mi_cols: i64::from(mi_cols),
        ref_mi_rows: i64::from(mi_rows),
    })
}

/// AV2 §6.19.7.12 `is_mv_valid` for the bounded ac0ej3 IntrABC subset, proven via the
/// DETERMINISTIC part of the `allow_local_intrabc` local-IBC-buffer branch.
///
/// §6.19.7.12 first rejects a block vector whose displaced source leaves the current
/// tile (already enforced before this is called by the source/target tile-bounds
/// checks in [`derive_intrabc_luma_prediction_geometry`]). It then takes the
/// `allow_local_intrabc` branch (`av2_is_dv_in_local_range`), whose constraints split
/// into a DETERMINISTIC geometry part (the uncoded-bottom-right exclusion, the
/// same-superblock-row constraint, and the `valid_SB` "current SB or left N SBs"
/// window) and a RUNTIME `IBCCoded` / `IBCBufferValid` collocation part that depends on
/// per-sample IBC-buffer state splot does not track.
///
/// This predicate proves ONLY the deterministic local-range geometry. The runtime
/// collocation/buffer part is satisfied by a STRONGER guarantee enforced by the caller
/// ([`super::recon::WienerNsLrReconSink::reconstruct_intrabc_block`]): the entire
/// source rectangle is already RECONSTRUCTED by this sink in decode order — a sample
/// reconstructed earlier in this tile's walk is, by construction, coded and valid in
/// the IBC-buffer sense. It returns `false` (DEFER — over-rejecting is safe) for
/// everything it cannot prove deterministically: a non-integer block vector,
/// `allow_local_intrabc != 1`, or the §6.19.7.12 64x64-tier BRU `numLeftActiveSB`
/// reduction (the only local-range term that needs runtime SB-active state; deferred
/// when BRU could apply). ac0ej3's first IntrABC block (128x128 SB, integer DV, source
/// in the SAME superblock as the active block) is proven valid by exactly this branch
/// — verified against AVM `av2_is_dv_valid` / `av2_is_dv_in_local_range`
/// (`av2/common/mvref_common.h`). The `INTRABC_BUFFER_NUM` / `INTRABC_BUFFER_SIZE_LOG2`
/// constants mirror AVM.
fn intrabc_dv_proven_valid(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<bool> {
    // A non-integer block vector is deferred upstream (the source/target shapes differ
    // in the sink); the conformance predicate only proves the integer-copy subset.
    if info.block_mv.row & 7 != 0 || info.block_mv.col & 7 != 0 {
        return Ok(false);
    }
    // The local-range branch requires `allow_local_intrabc == 1` (inferred 1 only when
    // `allow_global_intrabc == 1`; otherwise it is explicitly read). DEFER otherwise.
    let allow_local = core
        .intrabc
        .as_ref()
        .and_then(|params| params.allow_local_intrabc)
        .unwrap_or(false);
    if !allow_local {
        return Ok(false);
    }
    // §6.19.7.12 superblock size (samples). The BRU `numLeftActiveSB` reduction (which
    // needs runtime SB-active state) only applies in the 64x64 tier; DEFER a 64x64-SB
    // frame whose sequence could enable BRU rather than assume the full window.
    let sb_samples = superblock_samples(sequence, tile_offset)?;
    let sb_size = usize::try_from(sb_samples).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    if sb_size == 64 && sequence_enables_bru(sequence) {
        return Ok(false);
    }
    Ok(local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w: geometry.n4w * MI_SIZE,
        block_h: geometry.n4h * MI_SIZE,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size,
    }))
}

/// Inputs to the deterministic §6.19.7.12 local-intrabc range check (sample / MI-unit
/// terms; the block vector is in eighth-pel units).
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

/// The DETERMINISTIC geometry constraints of AV2 §6.19.7.12 `av2_is_dv_in_local_range`
/// (`av2/common/mvref_common.h`), for an integer block vector: the uncoded-bottom-right
/// exclusion, the same-superblock-row constraint, and the `valid_SB` window (current SB
/// or the left `numLeftSB` SBs). Returns `true` when the source rectangle is within the
/// allowed local-IBC range geometry. The caller separately proves the source is fully
/// reconstructed, which subsumes the runtime `IBCCoded`/`IBCBufferValid` collocation part.
fn local_intrabc_range_valid(inputs: IntrabcLocalRangeInputs) -> bool {
    // INTRABC_BUFFER_NUM == 4, INTRABC_BUFFER_SIZE_LOG2 == 6 (a 4 x 64x64 IBC buffer).
    const INTRABC_BUFFER_NUM: i64 = 4;
    const INTRABC_BUFFER_SIZE_LOG2: u32 = 6;

    let bw = inputs.block_w as i64;
    let bh = inputs.block_h as i64;
    let dv_row = i64::from(inputs.dv_row);
    let dv_col = i64::from(inputs.dv_col);
    // sb_size_log2 from the superblock sample size (64 -> 6, 128 -> 7, 256 -> 8).
    let sb_size_log2 = match inputs.sb_size {
        64 => 6,
        128 => 7,
        256 => 8,
        _ => return false,
    };

    let src_top_edge = (inputs.mi_row as i64 * MI_SIZE as i64) * 8 + dv_row;
    let src_left_edge = (inputs.mi_col as i64 * MI_SIZE as i64) * 8 + dv_col;
    let src_bottom_edge = (inputs.mi_row as i64 * MI_SIZE as i64 + bh) * 8 + dv_row;
    let src_right_edge = (inputs.mi_col as i64 * MI_SIZE as i64 + bw) * 8 + dv_col;
    // Integer DV: no interp border, and the `-1` rounding on the bottom/right edges.
    let src_top_y = src_top_edge >> 3;
    let src_left_x = src_left_edge >> 3;
    let src_bottom_y = (src_bottom_edge >> 3) - 1;
    let src_right_x = (src_right_edge >> 3) - 1;
    let act_left_x = inputs.mi_col as i64 * MI_SIZE as i64;
    let act_top_y = inputs.mi_row as i64 * MI_SIZE as i64;

    // Reference block cannot be in the uncoded bottom-right region of the current
    // block's top-left corner (integer DV: no interp borders).
    if ((dv_col >> 3) + bw) > 0 && ((dv_row >> 3) + bh) > 0 {
        return false;
    }
    // Reference block must be in the same superblock row as the active block.
    if (src_top_y >> sb_size_log2) < (act_top_y >> sb_size_log2) {
        return false;
    }
    if (src_bottom_y >> sb_size_log2) > (act_top_y >> sb_size_log2) {
        return false;
    }
    // numLeftSB = round_up(IBC buffer samples / superblock samples).
    let sb_area_log2 = 2 * sb_size_log2;
    let buffer_samples = INTRABC_BUFFER_NUM << (2 * INTRABC_BUFFER_SIZE_LOG2);
    let num_left_sb = (buffer_samples + (1 << sb_area_log2) - 1) >> sb_area_log2;
    // Reference block must be in the current SB or the left `numLeftSB` SBs.
    (src_right_x >> sb_size_log2) <= (act_left_x >> sb_size_log2)
        && (src_left_x >> sb_size_log2) >= (act_left_x >> sb_size_log2) - num_left_sb
}

/// Whether the sequence could enable the §5.x BRU tool (block-reference-update). The
/// §6.19.7.12 64x64-tier `numLeftActiveSB` reduction depends on runtime BRU SB-active
/// state, so the bounded gate DEFERS a 64x64-SB frame when BRU is enabled.
fn sequence_enables_bru(sequence: &SequenceHeader) -> bool {
    sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_bru)
}

fn tile_interval_for_block(
    starts: &[u32],
    block_start: usize,
    block_len: usize,
    bounds_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    let block_end = block_start.checked_add(block_len).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
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
        if block_start >= start && block_end <= end {
            return Ok((start, end));
        }
    }
    Err(wienerns_lr_selectable_transform_record_error_reason(
        tile_offset,
        bounds_reason,
    ))
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
    // IntrABC forces BILINEAR (§5.20.5.4). In §7.13.3.18 its non-zero taps are
    // the integer sample and the next sample on the fractional side, so no
    // top/left EIGHTTAP halo is part of this effective luma footprint.
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

fn intrabc_ref_stack(
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    max_bvp_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<Vec<Mv>> {
    intrabc_ref_stack_with_limit(sequence, geometry, max_bvp_drl_bits_minus_1, tile_offset)
}

fn intrabc_ref_stack_with_limit(
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    max_bvp_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<Vec<Mv>> {
    let max_count = usize::try_from(max_bvp_drl_bits_minus_1)
        .ok()
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_candidate_count",
            )
        })?;
    let block_width = geometry
        .n4w
        .checked_mul(MI_SIZE)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_width",
            )
        })?;
    let block_height = geometry
        .n4h
        .checked_mul(MI_SIZE)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_height",
            )
        })?;
    let sb_width = superblock_samples(sequence, tile_offset)?;
    let sb_height = sb_width;
    let mut candidates = Vec::new();
    // Bounded local handoff assumes §5.20.5.4 `find_mv_stack(0)` found no
    // spatial/bank candidates; current prediction support must replace this
    // before `block_mv` or `ref_mv_idx` become output-affecting.
    add_to_ref_bv(&mut candidates, max_count, 0, -sb_height, tile_offset)?;
    add_to_ref_bv(
        &mut candidates,
        max_count,
        -(sb_width + INTRABC_DELAY_PIXELS),
        0,
        tile_offset,
    )?;
    add_to_ref_bv(&mut candidates, max_count, 0, -block_height, tile_offset)?;
    add_to_ref_bv(&mut candidates, max_count, -block_width, 0, tile_offset)?;
    Ok(candidates)
}

fn add_to_ref_bv(
    candidates: &mut Vec<Mv>,
    max_count: usize,
    dx: i32,
    dy: i32,
    tile_offset: ByteOffset,
) -> Result<()> {
    if candidates.len() >= max_count {
        return Ok(());
    }
    let row = dy.checked_mul(8).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_bv_row_overflow",
        )
    })?;
    let col = dx.checked_mul(8).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_bv_col_overflow",
        )
    })?;
    candidates.push(Mv { row, col });
    Ok(())
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
    core.allow_intrabc == Some(true)
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
mod tests {
    use splot_core::headers::frame::{
        FrameSize, IntrabcParams, TxMode, build_minimal_intra_clk_core,
        build_minimal_intra_sequence_header,
    };
    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
    use splot_core::symbol_encoder::SymbolEncoder;

    use crate::error::DecodeError;
    use crate::tile_payload::{FrameCdfSubset, MvCdfSelector};

    use super::*;

    const BLOCK_16X16: usize = 6;

    fn selectable_fixture() -> (SequenceHeader, FrameHeaderCore) {
        let mut sequence = build_minimal_intra_sequence_header().unwrap();
        let (mut core, _) = build_minimal_intra_clk_core().unwrap();
        sequence
            .inter
            .as_mut()
            .unwrap()
            .seq_max_bvp_drl_bits_minus_1 = 0;
        sequence.inter.as_mut().unwrap().enable_bawp = false;
        core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Select;
        core.allow_intrabc = Some(true);
        core.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(false),
            allow_local_intrabc: None,
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        core.force_integer_mv = Some(false);
        (sequence, core)
    }

    fn selectable_large_frame_fixture() -> (SequenceHeader, FrameHeaderCore) {
        let (sequence, mut core) = selectable_fixture();
        core.frame_size = Some(FrameSize::new(128, 128));
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 32];
        tile_info.mi_row_starts = vec![0, 32];
        (sequence, core)
    }

    fn unsupported_reason(error: DecodeError) -> &'static str {
        match error {
            DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
            other => panic!("unexpected decode error: {other:?}"),
        }
    }

    fn encode_steps(steps: &[(Option<TileCdfSelector>, u32)]) -> Vec<u8> {
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        for &(selector, value) in steps {
            if let Some(selector) = selector {
                cdfs.with_row_mut(selector, |row| {
                    encoder.write_symbol(row, Symbol::new(u8::try_from(value).unwrap()))
                })
                .unwrap()
                .unwrap();
            } else {
                encoder.write_literal(value, 1).unwrap();
            }
        }
        encoder.finish().unwrap().into_bytes()
    }

    fn decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    fn state() -> TileIntrabcPreludeState {
        let (sequence, _) = selectable_fixture();
        TileIntrabcPreludeState::new(64, 64, &sequence, ByteOffset::new(0)).unwrap()
    }

    #[test]
    fn active_intrabc_nearmv_reads_use_skip_mode_and_drl_in_order() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 1),
            (Some(TileCdfSelector::IntrabcMode), 1),
            (None, 0),
        ]);
        let mut symbols = decoder(&payload);
        let state = state();
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let error = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &state,
            &sequence,
            &core,
            geometry,
            false,
            None,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: true,
            }
        );
        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_currframe_samples"
        );
        assert_eq!(symbols.symbol_count(), 4);
    }

    #[test]
    fn active_intrabc_newmv_reads_block_vector_then_reaches_currframe_frontier() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 0),
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (Some(TileCdfSelector::IntrabcPrecision), 1),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_QUARTER_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                0,
            ),
        ]);
        let mut symbols = decoder(&payload);
        let state = state();
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: false,
            }
        );
        let error = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &state,
            &sequence,
            &core,
            geometry,
            false,
            None,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_currframe_samples"
        );
        assert_eq!(symbols.symbol_count(), 8);
    }

    #[test]
    fn active_intrabc_ref_stack_requires_proven_empty_candidate_stack() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 2 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 2 }), 0),
            (Some(TileCdfSelector::IntrabcMode), 1),
            (None, 0),
        ]);
        let mut symbols = decoder(&payload);
        let mut state = state();
        let prior_intrabc = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: false,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: IntrabcBlockVector { row: -512, col: 0 },
            }),
        };
        state
            .record_block(19, 0, 4, 1, prior_intrabc, ByteOffset::new(0))
            .unwrap();
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let error = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &state,
            &sequence,
            &core,
            geometry,
            false,
            None,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: false,
            }
        );
        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack"
        );
        assert_eq!(symbols.symbol_count(), 5);
    }

    #[test]
    fn intrabc_newmv_geometry_derives_integer_luma_copy_rectangles() {
        let (sequence, mut core) = selectable_large_frame_fixture();
        core.force_integer_mv = Some(true);
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_ONE_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                0,
            ),
        ]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 16);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_nearmv_geometry_derives_integer_luma_copy_rectangles() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[(Some(TileCdfSelector::IntrabcMode), 1), (None, 0)]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 16);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_geometry_derives_bilinear_fractional_luma_prediction_region() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 0,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -132, col: 0 },
        };

        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(32, 32, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(32, 15, 16, 17).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 32);
        assert_eq!(prediction.scaling.start_y >> 10, 15);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_ne!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_geometry_uses_mi_domain_for_partial_edge_frame() {
        let (_, mut core) = selectable_fixture();
        core.frame_size = Some(FrameSize::new(10, 10));
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 4];
        tile_info.mi_row_starts = vec![0, 4];
        let block = IntrabcBlockContext::new(2, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 2);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -64, col: 0 },
        };

        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 8, 16, 8).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 0, 16, 8).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 0);
    }

    #[test]
    fn intrabc_geometry_rejects_source_outside_current_tile() {
        let (_, mut core) = selectable_large_frame_fixture();
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 4, 8];
        tile_info.mi_row_starts = vec![0, 8];
        let block = IntrabcBlockContext::new(4, 4, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: -128 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_self_referential_source() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_out_of_frame_source() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -512, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_out_of_frame_target() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(32, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_missing_frame_size() {
        let (_, mut core) = selectable_large_frame_fixture();
        core.frame_size = None;
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size"
        );
    }

    #[test]
    fn intrabc_newmv_one_pel_record_shifts_shell_delta() {
        let (sequence, mut core) = selectable_fixture();
        core.force_integer_mv = Some(true);
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_ONE_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                1,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::ColMvIndex {
                    mv_ctx: 1,
                    ctx: 0,
                })),
                0,
            ),
            (None, 0),
        ]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();

        assert_eq!(
            info,
            IntrabcInfo {
                intrabc_mode: 0,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_ONE_PEL,
                block_mv: IntrabcBlockVector { row: -504, col: 0 },
            }
        );
        assert_eq!(symbols.symbol_count(), 6);
    }

    #[test]
    fn intrabc_newmv_read_errors_use_intrabc_frontier_diagnostic() {
        let (sequence, _) = selectable_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = [];
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        // Force the shared read_mv helper to fail at the IntrABC caller boundary;
        // public IntrABC mode-info only passes spec-valid precisions.
        let error = assign_intrabc_mv(
            &mut cdfs,
            &mut symbols,
            &sequence,
            geometry,
            0,
            0,
            0,
            0,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv"
        );
    }

    #[test]
    fn non_intrabc_path_reads_only_use_intrabc_symbol() {
        let (_, core) = selectable_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[(Some(TileCdfSelector::Intrabc { ctx: 0 }), 0)]);
        let mut symbols = decoder(&payload);
        let state = state();
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: false,
            }
        );
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn contexts_use_intrabc_npos_and_skip_nposbuf_boundaries() {
        let mut state = state();
        let ordinary = IntrabcBlockPrelude {
            use_intrabc: false,
            is_inter: false,
            skip_flag: false,
            intrabc: None,
        };
        let intrabc_skip = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: true,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: IntrabcBlockVector { row: -512, col: 0 },
            }),
        };
        state
            .record_block(15, 4, 4, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(15, 8, 4, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(16, 3, 1, 4, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(16, 4, 4, 4, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(),
            2
        );
        assert_eq!(state.skip_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(), 2);
    }

    #[test]
    fn contexts_stop_after_first_two_valid_neighbour_candidates() {
        let mut state = state();
        let ordinary = IntrabcBlockPrelude {
            use_intrabc: false,
            is_inter: false,
            skip_flag: false,
            intrabc: None,
        };
        let intrabc_skip = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: true,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: IntrabcBlockVector { row: -512, col: 0 },
            }),
        };
        state
            .record_block(23, 7, 1, 1, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(19, 11, 1, 1, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(20, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(19, 8, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(),
            0
        );
        assert_eq!(state.skip_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(), 0);
    }

    #[test]
    fn contexts_preserve_duplicate_neighbour_slots_before_cap() {
        let mut state = state();
        let intrabc_skip = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: true,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: IntrabcBlockVector { row: -512, col: 0 },
            }),
        };
        state
            .record_block(0, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(),
            2
        );
        assert_eq!(state.skip_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(), 2);
    }

    #[test]
    fn intrabc_ref_stack_caps_256_sequence_superblocks_to_intra_sb_size() {
        let (mut sequence, _) = selectable_fixture();
        let partition = sequence.partition.as_mut().unwrap();
        partition.use_256x256_superblock = true;
        partition.use_128x128_superblock = false;
        let geometry =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(0, 0, BLOCK_16X16, false), 4, 4);

        let candidates =
            intrabc_ref_stack_with_limit(&sequence, geometry, 2, ByteOffset::new(0)).unwrap();

        assert_eq!(
            candidates,
            vec![
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -128, col: 0 },
                Mv { row: 0, col: -128 },
            ]
        );
    }

    // Codex finding 2: the §6.19.7.12 local-range geometry. ac0ej3's first IntrABC
    // block (128x128 SB, MI(16,56), block 32x64, DV (-512, 0)) is PROVEN valid — its
    // source x[224,256) y[0,64) sits in the SAME superblock as the active block and the
    // same superblock column. Verified against AVM `av2_is_dv_in_local_range`.
    #[test]
    fn local_intrabc_range_admits_ac0ej3_first_block() {
        assert!(local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 16,
            mi_col: 56,
            block_w: 32,
            block_h: 64,
            dv_row: -512,
            dv_col: 0,
            sb_size: 128,
        }));
    }

    // A DV whose source lies in the UNCODED bottom-right region of the active block's
    // top-left corner is rejected (the §6.19.7.12 first local-range guard).
    #[test]
    fn local_intrabc_range_rejects_uncoded_bottom_right_source() {
        // dv (+8, +8) eighth-pel == +1 sample down/right: source overlaps the uncoded
        // region (`(dvCol>>3)+bw > 0 && (dvRow>>3)+bh > 0`).
        assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 16,
            mi_col: 56,
            block_w: 32,
            block_h: 64,
            dv_row: 8,
            dv_col: 8,
            sb_size: 128,
        }));
    }

    // A DV whose source is too far LEFT (outside the current SB or the left numLeftSB
    // window) is rejected (the §6.19.7.12 `valid_SB` guard). For a 128x128 SB the IBC
    // buffer is one SB wide (numLeftSB == 1), so a source two SBs left is out of range.
    #[test]
    fn local_intrabc_range_rejects_source_beyond_left_buffer_window() {
        // Block at SB column 2 (mi_col 64 == 256px), source displaced 3 SBs left
        // (dv_col == -3 * 128 * 8 == -3072): src SB col is 3 SBs left of the active SB.
        assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 64,
            mi_col: 64,
            block_w: 32,
            block_h: 32,
            dv_row: 0,
            dv_col: -3072,
            sb_size: 128,
        }));
    }

    // The full `intrabc_dv_proven_valid` gate DEFERS when `allow_local_intrabc` is not
    // set, even for an otherwise-valid integer DV (the local-range branch is the only
    // one this bounded gate proves).
    #[test]
    fn proven_valid_defers_when_local_intrabc_disabled() {
        let (sequence, mut core) = selectable_large_frame_fixture();
        core.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(true),
            allow_local_intrabc: Some(false),
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 0, 2, false), 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -512, col: 0 },
        };
        assert!(
            !intrabc_dv_proven_valid(&sequence, &core, geometry, info, ByteOffset::new(20))
                .unwrap()
        );
    }
}
