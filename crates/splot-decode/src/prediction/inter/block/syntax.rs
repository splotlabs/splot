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
use super::super::{Mv, SINGLE_MODE_NEWMV};
use super::warp::inter_mv_read_config;
use super::{
    DecodeBlockFrontier, INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE,
    INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET, TileCdfSelector, TileCdfSubset, read_row, row_at,
    symbol_read_error,
};
use crate::Result;

pub(super) fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    needs_interp_filter: bool,
    ctx: usize,
) -> Result<ReconInterpolationFilter> {
    match frame_interpolation_filter {
        FrameInterpolationFilter::Eighttap => Ok(ReconInterpolationFilter::EightTap),
        FrameInterpolationFilter::EighttapSmooth => Ok(ReconInterpolationFilter::EightTapSmooth),
        FrameInterpolationFilter::EighttapSharp => Ok(ReconInterpolationFilter::EightTapSharp),
        FrameInterpolationFilter::Bilinear => Ok(ReconInterpolationFilter::Bilinear),
        FrameInterpolationFilter::Switchable => {
            if !needs_interp_filter {
                return Ok(ReconInterpolationFilter::EightTap);
            }
            let symbol = read_row(
                symbols,
                row_at(&mut cdfs.block_rows_mut().interp_filter, ctx),
            );
            Ok(interp_filter_from_symbol(symbol as u8))
        }
        _ => Err(crate::error::DecodeHeaderStateError::InvalidInterpolationFilter.into()),
    }
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

fn interp_filter_from_symbol(symbol: u8) -> ReconInterpolationFilter {
    match symbol {
        0 => ReconInterpolationFilter::EightTap,
        1 => ReconInterpolationFilter::EightTapSmooth,
        _ => ReconInterpolationFilter::EightTapSharp,
    }
}

pub(super) fn read_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
) -> usize {
    read_drl_idx_from(cdfs, symbols, new_mv_context, max_drl_bits_minus_1, 0)
}

pub(super) fn read_drl_idx_from(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    min_idx: usize,
) -> usize {
    let m = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in min_idx.min(m)..m {
        let bank = row_at(&mut cdfs.block_rows_mut().drl_mode, idx.min(2));
        if read_row(symbols, row_at(bank, new_mv_context)) == 0 {
            return idx;
        }
    }
    m
}

pub(super) fn read_skip_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    read_indexed_drl_idx(cdfs, symbols, max_drl_bits_minus_1, tile_offset, |idx| {
        TileCdfSelector::SkipDrlMode { idx }
    })
}

fn read_indexed_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
    selector: impl Fn(usize) -> TileCdfSelector,
) -> Result<usize> {
    let max_drl_bits = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in 0..max_drl_bits {
        let drl_mode = cdfs
            .read_block_symbol_trace(selector(idx.min(2)), symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?;
        if drl_mode.get() == 0 {
            return Ok(idx);
        }
    }
    Ok(max_drl_bits)
}

pub(super) fn read_tip_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    read_indexed_drl_idx(cdfs, symbols, max_drl_bits_minus_1, tile_offset, |idx| {
        TileCdfSelector::TipDrlMode { idx }
    })
}

pub(super) fn read_use_amvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    enable_adaptive_mvd: bool,
    single_mode: u8,
    ctx: usize,
) -> bool {
    single_use_amvd_selector(enable_adaptive_mvd, single_mode, ctx).is_some()
        && read_row(symbols, row_at(&mut cdfs.block_rows_mut().use_amvd[4], ctx)) != 0
}

/// The `use_amvd` selector of a single-reference block coding `single_mode`
/// (§ 5.20.7.6); `None` where the decoder reads no `use_amvd`.
pub(crate) fn single_use_amvd_selector(
    enable_adaptive_mvd: bool,
    single_mode: u8,
    ctx: usize,
) -> Option<TileCdfSelector> {
    (enable_adaptive_mvd && single_mode == SINGLE_MODE_NEWMV)
        .then_some(TileCdfSelector::UseAmvd { index: 4, ctx })
}

pub(super) fn read_skip_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    skip_mode_present: bool,
    frontier: &DecodeBlockFrontier,
    comp_ref_allowed: bool,
    ctx: usize,
) -> u8 {
    if !skip_mode_present
        || frontier.is_luma_part()
        || frontier.is_chroma_part()
        || !comp_ref_allowed
    {
        return 0;
    }
    read_row(symbols, row_at(&mut cdfs.block_rows_mut().skip_mode, ctx)) as u8
}

pub(super) fn effective_force_integer_mv(core: &FrameHeaderCore) -> bool {
    core.force_integer_mv
        .or_else(|| core.inter.as_ref().and_then(|inter| inter.force_integer_mv))
        .unwrap_or(false)
}

pub(crate) fn frame_mv_precision(core: &FrameHeaderCore) -> Result<u8> {
    if core.frame_is_intra == Some(true) {
        return Ok(0);
    }
    Ok(inter_mv_read_config(core)?.precision())
}

/// Whether the frame codes a per-block MV precision (§ 5.20.7.6).
pub(crate) fn use_per_block_mv_precision(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> bool {
    sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_flex_mvres)
        && !effective_force_integer_mv(core)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read_block_mv_precision_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    per_block_precision: bool,
    neighbour_ctx: &BlockNeighbourContext,
    frame_precision: u8,
    block_has_newmv: bool,
    is_adaptive_mvd: bool,
    tile_offset: ByteOffset,
) -> Result<BlockPrecisionRecord> {
    block_mv_precision_with(
        per_block_precision,
        neighbour_ctx,
        frame_precision,
        (block_has_newmv, is_adaptive_mvd),
        |selector| {
            let rows = cdfs.block_rows_mut();
            Ok(match selector {
                TileCdfSelector::UseMostProbablePrecision { ctx } => {
                    read_row(symbols, row_at(&mut rows.use_most_probable_precision, ctx)) as u8
                }
                TileCdfSelector::PbMvPrecision { ctx, frame_ctx } => {
                    let bank = row_at(&mut rows.pb_mv_precision, ctx);
                    read_row(symbols, row_at(bank, frame_ctx)) as u8
                }
                selector => cdfs
                    .read_block_symbol_trace(selector, symbols)
                    .map_err(|error| symbol_read_error(error, tile_offset))?
                    .get(),
            })
        },
    )
}

/// The § 5.20.7.6 block motion vector precision of a block that codes
/// `NEWMV` and adaptive MVD as `(block_has_newmv, is_adaptive_mvd)` in a
/// frame whose [`use_per_block_mv_precision`] is `per_block_precision`;
/// `read` gives each `use_most_probable_precision` and `pb_mv_precision`
/// value, and the encoder facade answers it for a target.
pub(crate) fn block_mv_precision_with(
    per_block_precision: bool,
    neighbour_ctx: &BlockNeighbourContext,
    frame_precision: u8,
    (block_has_newmv, is_adaptive_mvd): (bool, bool),
    mut read: impl FnMut(TileCdfSelector) -> Result<u8>,
) -> Result<BlockPrecisionRecord> {
    if is_adaptive_mvd
        || !block_has_newmv
        || !per_block_precision
        || frame_precision < MV_PRECISION_HALF_PEL
    {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let use_most_probable = read(TileCdfSelector::UseMostProbablePrecision {
        ctx: neighbour_ctx.most_probable_precision_ctx(),
    })?;
    if use_most_probable != 0 {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let pb_mv_precision = read(TileCdfSelector::PbMvPrecision {
        ctx: neighbour_ctx.pb_mv_precision_ctx(frame_precision),
        frame_ctx: usize::from(frame_precision - MV_PRECISION_HALF_PEL),
    })?;
    let adjusted = MV_PRECISION_ONE_PEL
        .max(frame_precision - 2)
        .checked_sub(pb_mv_precision)
        .filter(|&adjusted| adjusted > 0)
        .ok_or(crate::DecodeHeaderStateError::InvalidInterBlockModeState)?;
    let mv_precision = if adjusted <= MV_PRECISION_TWO_PEL {
        adjusted - 1
    } else {
        adjusted
    };
    Ok(BlockPrecisionRecord::explicit(mv_precision))
}

pub(super) fn lowered_pred_mv(precision: BlockPrecisionRecord, pred_mv: Mv) -> Mv {
    if precision.mv_precision < MV_PRECISION_HALF_PEL {
        lower_mv_precision(precision.mv_precision, pred_mv)
    } else {
        pred_mv
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
    use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

    use super::*;
    use crate::bitstream::tile_payload::FrameCdfSubset;

    const TILE_OFFSET: ByteOffset = ByteOffset::new(0);

    fn encode_indexed_symbols(
        sequence: &[(usize, u8)],
        selector: impl Fn(usize) -> TileCdfSelector,
    ) -> Vec<u8> {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::with_config(
            SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        );
        for &(idx, value) in sequence {
            tile.with_row_mut(selector(idx.min(2)), |row| {
                encoder.write_symbol_u16(row, Symbol::new(value))
            })
            .unwrap()
            .unwrap();
        }
        encoder.finish().unwrap().into_bytes()
    }

    fn encode_drl_symbols(sequence: &[(usize, u8)]) -> Vec<u8> {
        encode_indexed_symbols(sequence, |idx| TileCdfSelector::DrlMode { idx, ctx: 0 })
    }

    fn encode_skip_drl_symbols(sequence: &[(usize, u8)]) -> Vec<u8> {
        encode_indexed_symbols(sequence, |idx| TileCdfSelector::SkipDrlMode { idx })
    }

    fn encode_tip_drl_symbols(sequence: &[(usize, u8)]) -> Vec<u8> {
        encode_indexed_symbols(sequence, |idx| TileCdfSelector::TipDrlMode { idx })
    }

    fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            TILE_OFFSET,
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    #[test]
    fn drl_idx_from_skips_prefix_indices() {
        let payload = encode_drl_symbols(&[(2, 0)]);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);

        let idx = read_drl_idx_from(&mut tile, &mut symbols, 0, 3, 2);

        assert_eq!(idx, 2);
    }

    #[test]
    fn drl_idx_from_continues_after_one_symbol() {
        let payload = encode_drl_symbols(&[(1, 1), (2, 0)]);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);

        let idx = read_drl_idx_from(&mut tile, &mut symbols, 0, 3, 1);

        assert_eq!(idx, 2);
    }

    #[test]
    fn drl_idx_from_at_cap_reads_no_symbol() {
        let payload = encode_drl_symbols(&[]);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);

        let idx = read_drl_idx_from(&mut tile, &mut symbols, 0, 3, 4);

        assert_eq!(idx, 4);
    }

    fn assert_drl_reader(
        payload: &[u8],
        reader: fn(&mut TileCdfSubset, &mut SymbolDecoder<'_>, u32, ByteOffset) -> Result<usize>,
    ) {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(payload);

        let idx = reader(&mut tile, &mut symbols, 3, TILE_OFFSET).unwrap();

        assert_eq!(idx, 1);
    }

    #[test]
    fn skip_and_tip_drl_idx_use_dedicated_cdf_banks() {
        let sequence = [(0, 1), (1, 0)];
        assert_drl_reader(&encode_skip_drl_symbols(&sequence), read_skip_drl_idx);
        assert_drl_reader(&encode_tip_drl_symbols(&sequence), read_tip_drl_idx);
    }
}
