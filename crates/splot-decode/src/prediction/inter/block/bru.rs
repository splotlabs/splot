// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.5.2 `bru_mode_info()` leaves of non-active BRU superblocks.

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::{InterpolationFilter as ReconInterpolationFilter, ReconSample};

use super::super::find_mv_stack::BlockPrecisionRecord;
use super::super::read_mv::MV_PRECISION_ONE_PEL;
use super::resolve::{InterBlockSyntax, InterMotionSyntax};
use super::{BlockDecodeContext, finish_inter_leaf, parsed_inter_block, tile};
use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, DecodedLeafPublication, GeneralIntraLeafMode,
};
use crate::error::Result;

/// Decodes one leaf of a superblock whose `bru_mode` is not `BRU_ACTIVE`:
/// § 5.20.5.2 infers a skipped single-reference `NEWMV` block on `bru_ref`
/// with a zero motion vector, `EIGHTTAP_SHARP` and `segment_id = 0`, resets
/// `CurrentQIndex` to `base_q_idx`, and reads only `read_gdf()` and
/// `read_ccso()`.
#[inline(never)]
pub(super) fn decode_bru_leaf<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    context: &mut BlockDecodeContext<'_, T>,
) -> Result<GeneralIntraLeafMode> {
    let (Ok(n4w), Ok(n4h)) = (
        frontier.b_size.num_4x4_wide(),
        frontier.b_size.num_4x4_high(),
    ) else {
        return Err(crate::DecodeHeaderStateError::InvalidBlockGeometry.into());
    };
    let tile_offset: ByteOffset = context.sink.tile_offset;
    let bru_ref = context
        .sink
        .core
        .inter
        .as_ref()
        .and_then(|inter| inter.bru_ref)
        .and_then(|bru_ref| i8::try_from(bru_ref).ok())
        .ok_or(crate::DecodeHeaderStateError::InvalidInterReferenceMap)?;
    context
        .segment_id_state
        .record_block(frontier.r, frontier.c, n4w, n4h, 0);
    context
        .segment_id_state
        .record_predicted(frontier.r, frontier.c, n4w, n4h, false);
    context.delta_q_state.enter_bru_block();
    context
        .gdf_state
        .read_for_block(work_unit, symbols, frontier, tile_offset)?;
    context
        .ccso_state
        .read_for_block(work_unit, symbols, frontier, tile_offset)?;
    let block_qindex = super::segment_block_qindex(
        context.sink.sequence,
        context.sink.core,
        0,
        context.delta_q_state.qindex_u32(),
    );
    let interp = ReconInterpolationFilter::EightTapSharp;
    let state = finish_inter_leaf(
        work_unit,
        symbols,
        &mut context.sink,
        frontier,
        InterBlockSyntax {
            motion: InterMotionSyntax::Bru,
            interp,
            precision: BlockPrecisionRecord::explicit(MV_PRECISION_ONE_PEL),
            skip: true,
            use_amvd: false,
            tip_size_16x16: false,
            blend: super::mc::CompoundBlend::default(),
        },
        parsed_inter_block(bru_ref, None, interp),
        None,
        (n4w, n4h),
        (1, 0, block_qindex),
    )?;
    tile::push_leaf_entry(
        context.superblocks,
        context.entries,
        DecodedLeafPublication::from_frontier(frontier, context.sb_mask),
        state,
    );
    Ok(GeneralIntraLeafMode::no_luma_mode())
}
