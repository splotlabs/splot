// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::InterpolationFilter as ReconInterpolationFilter;

use super::super::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};
use super::compound::{CompoundParseInput, read_compound_average_syntax};
use super::find_mv_stack::{
    MvBlockContext, NeighbourMvGrid, NeighbourYMode, block_neighbour_ctx, find_mode_ctx,
    find_mv_stack,
};
use super::read_mv::{mv_clamp_to_integer, read_newmv_block_mvd};
use super::{
    InterBlock, InterResidual, Mv, PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, SPEC_MODE_INFO, effective_quantizer_deltas_are_zero, unsupported_at,
    unsupported_compound_at,
};
use crate::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraMultiblockError,
    GeneralIntraTreeWalkError, LumaCoeffBlock, TileCdfSelector, TileCdfSubset,
    TileCoeffContextState, TilePartitionTraversalError, TransformToolResidualPolicy,
    decode_general_intra_multiblock_tree, decode_general_intra_plane_coeffs, frame_mi_dimensions,
};

const INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE: usize = 3;
const INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET: usize = 4;
const MIN_INTER_LEAF_N4: usize = 8;
const FULL_SB_N4: usize = 16;
const SINGLE_REF_FRAME0: i8 = 0;

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_inter_blocks(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
) -> Result<Vec<PlacedInterBlock>> {
    let offset = frame_envelope.offset;
    let mut tile_plan = super::super::derive_inter_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
    )?;
    let [tile] = tile_plan.work_units_mut() else {
        return Err(inter_cap!(
            "inter_unexpected_tile_work_units",
            offset,
            "inter.tile_count != 1",
            SPEC_MODE_INFO
        ));
    };
    let tile_offset = tile.tile_byte_span().start;

    let max_drl_bits_minus_1 = core
        .inter
        .as_ref()
        .and_then(|inter| inter.max_drl_bits_minus_1)
        .ok_or_else(|| {
            inter_missing!(
                "inter_missing_max_drl_bits",
                offset,
                "inter.max_drl_bits_minus_1",
                SPEC_MODE_INFO
            )
        })?;

    let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(|_| {
        inter_missing!(
            "inter_mi_dimensions",
            offset,
            "inter.mi_dimensions",
            SPEC_MODE_INFO
        )
    })?;
    let mut coeff_ctx = TileCoeffContextState::new(mi_rows, mi_cols).map_err(|_| {
        inter_cap!(
            "inter_coeff_context_state",
            offset,
            "inter.residual_context_state",
            SPEC_MODE_INFO
        )
    })?;

    let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols)
        .ok_or_else(|| inter_cap!("inter_mv_grid", offset, "inter.mv_grid", SPEC_MODE_INFO))?;
    let sb_h4 = superblock_h4(sequence, core).ok_or_else(|| {
        inter_missing!(
            "inter_sb_size",
            offset,
            "inter.superblock_size",
            SPEC_MODE_INFO
        )
    })?;

    let residual_tools_present = sequence.transform_quant_entropy.is_none_or(|tq| {
        tq.enable_inter_ist
            || tq.enable_inter_ddt
            || tq.enable_cctx
            || tq.enable_fsc
            || tq.enable_idtx_intra
    });
    let residual_quantizer_deltas_are_zero = core
        .quantization_params
        .as_ref()
        .is_some_and(|quant| effective_quantizer_deltas_are_zero(sequence, quant));

    let mv_stack_tools_present = sequence.inter.as_ref().is_none_or(|seq_inter| {
        seq_inter.enable_ref_frame_mvs
            || seq_inter.enable_refmvbank
            || seq_inter.drl_reorder != splot_core::headers::sequence::DrlReorder::Disabled
    });

    let mut decoded_blocks: Vec<PlacedInterBlock> = Vec::new();
    let limits = options.limits();
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit,
         symbols,
         frontier,
         _joint_modes,
         _uses_mrls,
         _fsc_modes,
         _is_cfl_ctx,
         _block_decoded| {
            let placed = decode_one_inter_block(
                work_unit,
                symbols,
                frontier,
                &mut coeff_ctx,
                &mut mv_grid,
                sb_h4,
                mi_rows,
                mi_cols,
                max_drl_bits_minus_1,
                frame_interpolation_filter,
                residual_tools_present,
                residual_quantizer_deltas_are_zero,
                mv_stack_tools_present,
                num_total_refs,
                reference_select,
                compound_is_joint_ctx,
                num_same_ref_compound,
                tile_offset,
            )?;
            decoded_blocks.push(placed);
            Ok(crate::tile_payload::GeneralIntraLeafMode::no_luma_mode())
        },
    )
    .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;

    symbols.exit_symbol().map_err(|_| {
        if reference_select {
            compound_cap!(
                "compound_exit_symbol",
                tile_offset,
                "inter.compound.exit_symbol",
                SPEC_MODE_INFO
            )
        } else {
            inter_cap!(
                "inter_exit_symbol",
                tile_offset,
                "inter.exit_symbol",
                SPEC_MODE_INFO
            )
        }
    })?;

    if decoded_blocks.is_empty() {
        return Err(inter_missing!(
            "inter_no_decoded_block",
            tile_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }
    Ok(decoded_blocks)
}

fn superblock_h4(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Option<usize> {
    let partition = sequence.partition?;
    core.frame_is_intra?;
    match partition.seq_sb_size() {
        splot_core::headers::sequence::SuperblockSize::Block64x64 => Some(16),
        splot_core::headers::sequence::SuperblockSize::Block128x128 => Some(32),
        splot_core::headers::sequence::SuperblockSize::Block256x256 => Some(64),
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn decode_one_inter_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    coeff_ctx: &mut TileCoeffContextState,
    mv_grid: &mut NeighbourMvGrid,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tools_present: bool,
    residual_quantizer_deltas_are_zero: bool,
    mv_stack_tools_present: bool,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
    tile_offset: ByteOffset,
) -> Result<PlacedInterBlock> {
    let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
        inter_diag!(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO
        )
    })?;
    let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
        inter_diag!(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO
        )
    })?;
    let mi_row = frontier.r;
    let mi_col = frontier.c;
    let placed_block = |block| PlacedInterBlock {
        luma_x: mi_col * 4,
        luma_y: mi_row * 4,
        luma_w: n4w * 4,
        luma_h: n4h * 4,
        block,
    };

    let mut block_ctx = MvBlockContext {
        mi_row,
        mi_col,
        bw4: n4w,
        bh4: n4h,
        sb_h4,
        ref_frame0: SINGLE_REF_FRAME0,
        mi_rows,
        mi_cols,
    };

    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);

    if mv_stack_tools_present && neighbour_ctx.has_neighbour {
        return Err(inter_cap!(
            "inter_block_mv_stack_tools_with_neighbour",
            tile_offset,
            "inter.mv_stack.temporal_or_reordered_neighbour",
            super::SPEC_MV
        ));
    }

    if n4w < MIN_INTER_LEAF_N4 || n4h < MIN_INTER_LEAF_N4 {
        return Err(inter_cap!(
            "inter_block_unsupported_size",
            tile_offset,
            "inter.block_size < 32x32",
            super::SPEC_MV
        ));
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    let is_inter = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::IsInter {
                ctx: neighbour_ctx.is_inter_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    if is_inter.get() != 1 {
        return Err(inter_cap!(
            "inter_block_is_intra",
            tile_offset,
            "inter.block.is_inter == 0",
            SPEC_MODE_INFO
        ));
    }

    let skip = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::Skip {
                ctx: neighbour_ctx.skip_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let skip = skip.get();
    if skip != 0 && skip != 1 {
        return Err(inter_cap!(
            "inter_block_unexpected_skip",
            tile_offset,
            "inter.block.skip out of range",
            SPEC_MODE_INFO
        ));
    }

    if reference_select {
        let is_joint_ctx = compound_is_joint_ctx.ok_or_else(|| {
            compound_missing!(
                "compound_missing_is_joint_context",
                tile_offset,
                "inter.compound.is_joint_context",
                SPEC_MODE_INFO
            )
        })?;
        let compound = read_compound_average_syntax(
            cdfs,
            symbols,
            CompoundParseInput {
                num_total_refs,
                num_same_ref_compound,
                has_neighbour: neighbour_ctx.has_neighbour,
                new_mv_context: mode_ctx.new_mv_context,
                is_joint_ctx,
                skip,
                n4w,
                n4h,
                mi_row,
                mi_col,
                mi_rows,
                mi_cols,
            },
            tile_offset,
        )?;
        let ref_mv_idx0 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        let ref_mv_idx1 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        if ref_mv_idx0 != 0 || ref_mv_idx1 != 0 {
            return Err(compound_cap!(
                "compound_block_drl_idx",
                tile_offset,
                "inter.compound.drl_idx != 0",
                SPEC_MODE_INFO
            ));
        }
        let interp = resolve_interp_filter(
            cdfs,
            symbols,
            frame_interpolation_filter,
            SINGLE_MODE_NEARMV,
            true,
            neighbour_ctx.has_neighbour,
            tile_offset,
        )?;
        mv_grid.record_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            true,
            compound.ref_frame0,
            NeighbourYMode::Other,
            compound.mv0,
            true,
        );
        return Ok(placed_block(InterBlock {
            ref_frame0: compound.ref_frame0,
            ref_frame1: Some(compound.ref_frame1),
            mv: compound.mv0,
            mv1: compound.mv1,
            interp,
            residual: None,
        }));
    }

    let ref_frame0: i8 = if num_total_refs >= 2 {
        if neighbour_ctx.has_neighbour {
            return Err(inter_cap!(
                "inter_block_single_ref_with_neighbour",
                tile_offset,
                "inter.single_ref.neighbour_context",
                SPEC_MODE_INFO
            ));
        }
        let ctx = neighbour_ctx
            .single_ref_ctx(0, num_total_refs)
            .ok_or_else(|| {
                inter_missing!(
                    "inter_block_single_ref_ctx",
                    tile_offset,
                    "inter.single_ref.context",
                    SPEC_MODE_INFO
                )
            })?;
        let contexts = [ctx];
        let selected = super::single_ref::read_single_ref(cdfs, symbols, num_total_refs, &contexts)
            .map_err(|_| {
                inter_missing!(
                    "inter_block_single_ref_read",
                    tile_offset,
                    "inter.single_ref.symbol",
                    SPEC_MODE_INFO
                )
            })?;
        i8::try_from(selected).map_err(|_| {
            inter_cap!(
                "inter_block_single_ref_value",
                tile_offset,
                "inter.single_ref.selection out of range",
                SPEC_MODE_INFO
            )
        })?
    } else {
        SINGLE_REF_FRAME0
    };
    block_ctx.ref_frame0 = ref_frame0;

    let single_mode = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::SingleMode {
                ctx: mode_ctx.new_mv_context,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let single_mode = single_mode.get();
    if single_mode != SINGLE_MODE_NEARMV
        && single_mode != SINGLE_MODE_GLOBALMV
        && single_mode != SINGLE_MODE_NEWMV
    {
        return Err(inter_cap!(
            "inter_block_unsupported_single_mode",
            tile_offset,
            "inter.single_mode not in {NEARMV, GLOBALMV, NEWMV}",
            SPEC_MODE_INFO
        ));
    }

    let stack = find_mv_stack(mv_grid, &block_ctx, Mv::ZERO);

    let ref_mv_idx = if single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV {
        read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?
    } else {
        0
    };

    if (single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV)
        && ref_mv_idx >= stack.num_mv_found()
    {
        return Err(inter_cap!(
            "inter_block_drl_idx_out_of_range",
            tile_offset,
            "inter.drl_idx past MV stack",
            SPEC_MODE_INFO
        ));
    }
    let pred_mv = stack.candidate(ref_mv_idx);
    let mv = match single_mode {
        SINGLE_MODE_GLOBALMV => Mv::ZERO,
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let diff = read_newmv_block_mvd(cdfs, symbols, tile_offset)?;
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
    };

    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        single_mode,
        false,
        neighbour_ctx.has_neighbour,
        tile_offset,
    )?;

    let residual = if skip == 0 {
        if !residual_quantizer_deltas_are_zero {
            return Err(inter_cap!(
                "inter_block_residual_quantizer_delta",
                tile_offset,
                "inter.residual.nonzero_quantizer_delta",
                SPEC_MODE_INFO
            ));
        }
        if n4w != FULL_SB_N4
            || n4h != FULL_SB_N4
            || mi_row != 0
            || mi_col != 0
            || mi_rows > FULL_SB_N4
            || mi_cols > FULL_SB_N4
        {
            return Err(inter_cap!(
                "inter_block_multiblock_residual",
                tile_offset,
                "inter.residual.block_geometry",
                SPEC_MODE_INFO
            ));
        }
        if residual_tools_present {
            return Err(inter_cap!(
                "inter_block_residual_tools",
                tile_offset,
                "inter.residual.transform_tools",
                SPEC_MODE_INFO
            ));
        }
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            tile_offset,
        )?)
    } else {
        None
    };

    let y_mode = if single_mode == SINGLE_MODE_NEWMV {
        NeighbourYMode::NewMv
    } else {
        NeighbourYMode::Other
    };
    mv_grid.record_block(
        mi_row,
        mi_col,
        n4w,
        n4h,
        true,
        ref_frame0,
        y_mode,
        mv,
        skip == 1,
    );

    Ok(placed_block(InterBlock {
        ref_frame0,
        ref_frame1: None,
        mv,
        mv1: Mv::ZERO,
        interp,
        residual,
    }))
}

const TX_64X64: usize = 4;
const TX_32X32: usize = 3;
const INTER_UV_MODE_DC: usize = 0;

fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    tile_offset: ByteOffset,
) -> Result<InterResidual> {
    let luma = read_inter_residual_plane(
        work_unit,
        symbols,
        coeff_ctx,
        0,
        TX_64X64,
        false,
        tile_offset,
    )?;
    let u = read_inter_residual_plane(
        work_unit,
        symbols,
        coeff_ctx,
        1,
        TX_32X32,
        false,
        tile_offset,
    )?;
    let v = read_inter_residual_plane(
        work_unit,
        symbols,
        coeff_ctx,
        2,
        TX_32X32,
        !u.all_zero,
        tile_offset,
    )?;
    Ok(InterResidual { luma, u, v })
}

fn read_inter_residual_plane(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    chroma_eob_ctx: bool,
    tile_offset: ByteOffset,
) -> Result<LumaCoeffBlock> {
    decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        plane,
        tx_size,
        0,
        0,
        true,
        chroma_eob_ctx,
        INTER_UV_MODE_DC,
        true,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|_| residual_read_error(tile_offset))
}

fn residual_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    inter_missing!(
        "inter_block_residual_parse",
        tile_offset,
        "inter.residual.coefficients",
        SPEC_MODE_INFO
    )
}

fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    mode_for_needs_interp_filter: u8,
    ref_frame1_is_inter: bool,
    has_neighbour: bool,
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
            if has_neighbour {
                return Err(inter_cap!(
                    "inter_block_interp_filter_neighbour_ctx",
                    tile_offset,
                    "inter.interp_filter.neighbour_context",
                    SPEC_MODE_INFO
                ));
            }
            let symbol = cdfs
                .read_block_symbol_trace(
                    TileCdfSelector::InterpFilter {
                        ctx: interp_filter_no_neighbour_ctx(ref_frame1_is_inter),
                    },
                    symbols,
                )
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

pub(super) fn interp_filter_no_neighbour_ctx(ref_frame1_is_inter: bool) -> usize {
    INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE
        + usize::from(ref_frame1_is_inter) * INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET
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

fn read_drl_idx(
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

fn symbol_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    inter_missing!(
        "inter_block_mode_parse",
        tile_offset,
        "inter.block.mode_info_symbols",
        SPEC_MODE_INFO
    )
}

fn map_inter_multiblock_error(
    error: GeneralIntraMultiblockError<super::super::DecodeError>,
    tile_offset: ByteOffset,
) -> super::super::DecodeError {
    match error {
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => super::super::DecodeError::Limit { source },
        _ => inter_cap!(
            "inter_partition_walk",
            tile_offset,
            "inter.partition_walk",
            SPEC_MODE_INFO
        ),
    }
}
