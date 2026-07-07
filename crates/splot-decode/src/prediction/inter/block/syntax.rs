// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{
    FrameHeaderCore, InterpolationFilter as FrameInterpolationFilter,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::InterpolationFilter as ReconInterpolationFilter;

use super::super::find_mv_stack::{BlockNeighbourContext, BlockPrecisionRecord};
use super::super::read_mv::{
    MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_TWO_PEL, lower_mv_precision,
};
use super::super::{Mv, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEWMV, SPEC_MODE_INFO, unsupported_at};
use super::warp::inter_mv_read_config;
use super::{
    INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE, INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET,
    TileCdfSelector, TileCdfSubset, symbol_read_error,
};
use crate::Result;

pub(super) fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    mode_for_needs_interp_filter: u8,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match frame_interpolation_filter {
        FrameInterpolationFilter::Eighttap => Ok(ReconInterpolationFilter::EightTap),
        FrameInterpolationFilter::EighttapSmooth => Ok(ReconInterpolationFilter::EightTapSmooth),
        FrameInterpolationFilter::EighttapSharp => Ok(ReconInterpolationFilter::EightTapSharp),
        FrameInterpolationFilter::Bilinear => Ok(ReconInterpolationFilter::Bilinear),
        FrameInterpolationFilter::Switchable => {
            if mode_for_needs_interp_filter == SINGLE_MODE_GLOBALMV {
                return Ok(ReconInterpolationFilter::EightTap);
            }
            let symbol = cdfs
                .read_block_symbol_trace(TileCdfSelector::InterpFilter { ctx }, symbols)
                .map_err(|_| symbol_read_error(tile_offset))?;
            interp_filter_from_symbol(symbol.get(), tile_offset)
        }
        _ => Err(inter_cap!(
            "inter_unsupported_interpolation_filter",
            tile_offset,
            "inter.interpolation_filter",
            SPEC_MODE_INFO
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn trace_interp_filter_context(
    kind: &'static str,
    mi_row: usize,
    mi_col: usize,
    ref_frame0: i8,
    ref_frame1_is_inter: bool,
    ctx: usize,
    neighbour_ctx: &BlockNeighbourContext,
    symbols: &SymbolDecoder<'_>,
) {
    if !crate::trace_flags::trace_flag!("SPLOT_TRACE_INTERP_FILTER_CTX") {
        return;
    }
    eprintln!(
        "interp_filter_ctx kind={kind} r={mi_row} c={mi_col} ref0={ref_frame0} ref1_is_inter={ref_frame1_is_inter} ctx={ctx} neighbour={neighbour_ctx:?} checkpoint={:?}",
        symbols.checkpoint(),
    );
}

pub(crate) fn interp_filter_no_neighbour_ctx(ref_frame1_is_inter: bool) -> usize {
    INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE
        + usize::from(ref_frame1_is_inter) * INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET
}

pub(super) fn interp_filter_symbol(filter: ReconInterpolationFilter) -> u8 {
    match filter {
        ReconInterpolationFilter::EightTap => 0,
        ReconInterpolationFilter::EightTapSmooth => 1,
        ReconInterpolationFilter::EightTapSharp => 2,
        ReconInterpolationFilter::Bilinear => 3,
    }
}

fn interp_filter_from_symbol(
    symbol: u8,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match symbol {
        0 => Ok(ReconInterpolationFilter::EightTap),
        1 => Ok(ReconInterpolationFilter::EightTapSmooth),
        2 => Ok(ReconInterpolationFilter::EightTapSharp),
        3 => Ok(ReconInterpolationFilter::Bilinear),
        _ => Err(inter_cap!(
            "inter_invalid_interp_filter_symbol",
            tile_offset,
            "inter.interp_filter symbol out of range",
            SPEC_MODE_INFO
        )),
    }
}

pub(super) fn read_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let m = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in 0..m {
        let bank = idx.min(2);
        let drl_mode = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::DrlMode {
                    idx: bank,
                    ctx: new_mv_context,
                },
                symbols,
            )
            .map_err(|_| symbol_read_error(tile_offset))?;
        if drl_mode.get() == 0 {
            return Ok(idx);
        }
    }
    Ok(m)
}

pub(super) fn read_use_amvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    enable_adaptive_mvd: bool,
    single_mode: u8,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if !enable_adaptive_mvd || single_mode != SINGLE_MODE_NEWMV {
        return Ok(false);
    }
    let use_amvd = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseAmvd { index: 4, ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(use_amvd.get() != 0)
}

pub(super) fn effective_force_integer_mv(core: &FrameHeaderCore) -> bool {
    core.force_integer_mv
        .or_else(|| core.inter.as_ref().and_then(|inter| inter.force_integer_mv))
        .unwrap_or(false)
}

/// § 5.18.2 `FrameMvPrecision` as a Table 6.19 code.
pub(super) fn frame_mv_precision(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<u8> {
    if core.frame_is_intra == Some(true) {
        return Ok(0);
    }
    Ok(inter_mv_read_config(core, tile_offset)?.precision())
}

/// § 5.18.2 `UsePerBlockMvPrecision`: `enable_flex_mvres` outside the
/// `force_integer_mv` path (which pins `MV_PRECISION_ONE_PEL`).
fn use_per_block_mv_precision(sequence: &SequenceHeader, core: &FrameHeaderCore) -> bool {
    sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_flex_mvres)
        && !effective_force_integer_mv(core)
}

/// § 5.20.7.13 per-block MV precision: the `use_most_probable_precision` and
/// `pb_mv_precision` reads plus the `adjustedPrecision` derivation.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_block_mv_precision_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    frame_precision: u8,
    block_has_newmv: bool,
    is_adaptive_mvd: bool,
    tile_offset: ByteOffset,
) -> Result<BlockPrecisionRecord> {
    if is_adaptive_mvd
        || !block_has_newmv
        || !use_per_block_mv_precision(sequence, core)
        || frame_precision < MV_PRECISION_HALF_PEL
    {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let use_most_probable = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseMostProbablePrecision {
                ctx: neighbour_ctx.most_probable_precision_ctx(),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    if use_most_probable.get() != 0 {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let pb_mv_precision = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::PbMvPrecision {
                ctx: neighbour_ctx.pb_mv_precision_ctx(frame_precision),
                frame_ctx: usize::from(frame_precision - MV_PRECISION_HALF_PEL),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let adjusted = MV_PRECISION_ONE_PEL
        .max(frame_precision - 2)
        .checked_sub(pb_mv_precision.get())
        .filter(|&adjusted| adjusted > 0)
        .ok_or_else(|| symbol_read_error(tile_offset))?;
    let mv_precision = if adjusted <= MV_PRECISION_TWO_PEL {
        adjusted - 1
    } else {
        adjusted
    };
    Ok(BlockPrecisionRecord::explicit(mv_precision))
}

/// § 5.20.7.13 `assign_mv` predictor rounding: `lower_mv_precision` applies to
/// NEWMV-family predictors below `MV_PRECISION_HALF_PEL`.
pub(super) fn lowered_pred_mv(precision: BlockPrecisionRecord, pred_mv: Mv) -> Mv {
    if precision.mv_precision < MV_PRECISION_HALF_PEL {
        lower_mv_precision(precision.mv_precision, pred_mv)
    } else {
        pred_mv
    }
}
