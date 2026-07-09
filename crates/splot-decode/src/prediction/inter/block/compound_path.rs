// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;

use super::super::compound::{
    CompoundParseInput, CompoundYMode, read_compound_mode_syntax, read_compound_reference_pair,
};
use super::super::read_mv::apply_inter_mvd_sign_pair;
use super::*;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const REFINE_SWITCHABLE: u32 = 1;
const REFINE_ALL: u32 = 2;
const MV_PROJECTION_DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];
const SPEC_READ_REFINEMV: &str = "5.20.7.17";
const SPEC_PREDICT_OPTFLOW: &str = "7.13.3.8";

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_compound_inter_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    mv_grid: &mut NeighbourMvGrid,
    temporal_context: Option<&TemporalMvContext>,
    motion_field: &mut TemporalMotionField,
    block_ctx: &mut MvBlockContext,
    neighbour_ctx: &BlockNeighbourContext,
    ref_mv_bank: &mut Option<super::super::find_mv_stack::RefMvBank>,
    warp_param_bank: &mut super::super::find_mv_stack::WarpParamBank,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    num_total_refs: usize,
    num_same_ref_compound: u8,
    compound_is_joint_ctx: Option<usize>,
    skip: u8,
    n4w: usize,
    n4h: usize,
    mi_row: usize,
    mi_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    max_drl_bits_minus_1: u32,
    drl_reorder: DrlReorder,
    temporal_first_frame: bool,
    enable_adaptive_mvd: bool,
    residual_quantizer_deltas_are_zero: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let ref_contexts = compound_ref_contexts(neighbour_ctx, num_total_refs, tile_offset)?;
    let ref_distance_nonnegative = compound_ref_distance_signs(
        ref_frame_idx,
        reference,
        core.order_hint_lsb.unwrap_or(0),
        num_total_refs,
        tile_offset,
    )?;
    let pair = read_compound_reference_pair(
        cdfs,
        symbols,
        CompoundParseInput {
            num_total_refs,
            num_same_ref_compound,
            ref_contexts,
            ref_distance_nonnegative,
            is_joint_ctx: compound_is_joint_ctx,
        },
        tile_offset,
    )?;
    block_ctx.ref_frame0 = pair.0;
    block_ctx.ref_frame1 = Some(pair.1);
    let mode_ctx = find_mode_ctx(mv_grid, block_ctx);
    let mut compound = read_compound_mode_syntax(
        cdfs,
        symbols,
        pair,
        mode_ctx.new_mv_context,
        compound_is_joint_ctx,
        tile_offset,
    )?;
    if compound_switchable_opfl_reachable(
        core,
        reference,
        ref_frame_idx,
        compound,
        n4w,
        n4h,
        tile_offset,
    )? {
        compound.use_optflow =
            read_compound_use_optflow_syntax(cdfs, symbols, compound.y_mode, tile_offset)?;
    }
    let use_amvd = read_compound_use_amvd_syntax(
        cdfs,
        symbols,
        enable_adaptive_mvd,
        compound.y_mode,
        compound.use_optflow,
        neighbour_ctx.amvd_ctx(compound.ref_frame0),
        tile_offset,
    )?;
    let jmvd_scale_mode = read_compound_jmvd_scale_mode_syntax(
        cdfs,
        symbols,
        compound.y_mode,
        use_amvd,
        tile_offset,
    )?;
    let skip_mode_present = core
        .inter_tail
        .as_ref()
        .is_some_and(|tail| tail.skip_mode_present);
    let mut ref_mv_idx = 0;
    let mut ref_mv_idx0 = 0;
    let mut ref_mv_idx1 = 0;
    if compound.y_mode.reads_drl_idx() {
        if compound_reads_second_drl(compound, skip_mode_present) {
            ref_mv_idx0 = read_drl_idx(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?;
            let second_min_idx = if compound.ref_frame0 == compound.ref_frame1
                && compound.y_mode == CompoundYMode::NearNear
            {
                ref_mv_idx0.saturating_add(1)
            } else {
                0
            };
            ref_mv_idx1 = read_drl_idx_from(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                second_min_idx,
                tile_offset,
            )?;
        } else {
            ref_mv_idx = read_drl_idx(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?;
        }
    }
    let frame_mv_config = inter_mv_read_config(core, tile_offset)?;
    let precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        frame_mv_config.precision(),
        compound.y_mode.has_newmv(),
        use_amvd,
        tile_offset,
    )?;
    if compound.y_mode.has_newmv() || compound.y_mode.has_nearmv() {
        let config = MvReadConfig::inter(precision.mv_precision);
        let bank = ref_mv_bank
            .as_ref()
            .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2));
        let compound_temporal_allowed = compound.ref_frame0 != compound.ref_frame1;
        let temporal = if compound_temporal_allowed {
            temporal_context
        } else {
            None
        };
        let temporal_first0 = compound_temporal_allowed
            && temporal_first_frame
            && super::block_ref_within_temporal_distance(
                reference,
                ref_frame_idx,
                core.order_hint_lsb.unwrap_or(0),
                compound.ref_frame0,
            );
        let temporal_first1 = compound_temporal_allowed
            && temporal_first_frame
            && super::block_ref_within_temporal_distance(
                reference,
                ref_frame_idx,
                core.order_hint_lsb.unwrap_or(0),
                compound.ref_frame1,
            );
        match compound.y_mode {
            CompoundYMode::NearNear => {
                let stack0 = find_mv_stack_with_temporal(
                    mv_grid,
                    &single_ref_block_context(block_ctx, compound.ref_frame0),
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    temporal,
                    temporal_first0,
                );
                let stack1 = find_mv_stack_with_temporal(
                    mv_grid,
                    &single_ref_block_context(block_ctx, compound.ref_frame1),
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    temporal,
                    temporal_first1,
                );
                compound.mv0 = stack0.candidate(ref_mv_idx0);
                compound.mv1 = stack1.candidate(ref_mv_idx1);
            }
            CompoundYMode::NearNew | CompoundYMode::NewNear => {
                let stack0 = find_mv_stack_with_temporal(
                    mv_grid,
                    &single_ref_block_context(block_ctx, compound.ref_frame0),
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    temporal,
                    temporal_first0,
                );
                let stack1 = find_mv_stack_with_temporal(
                    mv_grid,
                    &single_ref_block_context(block_ctx, compound.ref_frame1),
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    temporal,
                    temporal_first1,
                );
                let has_second_drl = compound_reads_second_drl(compound, skip_mode_present);
                let candidates = [
                    stack0.candidate(if has_second_drl {
                        ref_mv_idx0
                    } else {
                        ref_mv_idx
                    }),
                    stack1.candidate(if has_second_drl {
                        ref_mv_idx1
                    } else {
                        ref_mv_idx
                    }),
                ];
                let new_ref = usize::from(compound.y_mode == CompoundYMode::NearNew);
                let pred_mv = if use_amvd {
                    candidates[new_ref]
                } else {
                    lowered_pred_mv(precision, candidates[new_ref])
                };
                let diff = if use_amvd {
                    let magnitude = read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?;
                    apply_inter_mvd_signs(
                        magnitude,
                        symbols,
                        tile_offset,
                        config,
                        false,
                        compound.y_mode.mvd_sign_derivation_threshold(),
                    )?
                } else {
                    let magnitude =
                        read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?;
                    apply_inter_mvd_signs(
                        magnitude,
                        symbols,
                        tile_offset,
                        config,
                        false,
                        compound.y_mode.mvd_sign_derivation_threshold(),
                    )?
                };
                let mut mvs = candidates;
                mvs[new_ref] = add_mv_clamped(pred_mv, diff);
                [compound.mv0, compound.mv1] = mvs;
            }
            CompoundYMode::JointNew => {
                let stack = find_mv_stack(
                    mv_grid,
                    block_ctx,
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    false,
                );
                let projection = compound_joint_mv_projection(
                    core,
                    reference,
                    ref_frame_idx,
                    compound.ref_frame0,
                    compound.ref_frame1,
                    tile_offset,
                )?;
                let raw_pred_mv = stack.candidate(ref_mv_idx);
                let pred_mv = if use_amvd {
                    raw_pred_mv
                } else {
                    lowered_pred_mv(precision, raw_pred_mv)
                };
                let magnitude = if use_amvd {
                    read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?
                } else {
                    read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?
                };
                let diff = apply_inter_mvd_signs(
                    magnitude,
                    symbols,
                    tile_offset,
                    config,
                    ref_mv_idx == 0
                        && inter_mvd_sign_derivation_allowed(
                            sequence,
                            core,
                            SINGLE_MODE_NEWMV,
                            use_amvd,
                            frame_mv_config,
                            config,
                        ),
                    compound.y_mode.mvd_sign_derivation_threshold(),
                )?;
                let base_mv = add_mv_clamped(pred_mv, diff);
                let projected = scale_joint_projected_mvd(
                    project_joint_mvd(diff, projection.second_dist, projection.first_dist),
                    jmvd_scale_mode,
                    use_amvd,
                );
                let other_mv = add_mv_clamped(raw_pred_mv, projected);
                if projection.base_list == 0 {
                    compound.mv0 = base_mv;
                    compound.mv1 = other_mv;
                } else {
                    compound.mv0 = other_mv;
                    compound.mv1 = base_mv;
                }
            }
            CompoundYMode::NewNew => {
                let stack = find_mv_stack(
                    mv_grid,
                    block_ctx,
                    Mv::ZERO,
                    bank,
                    warp_param_bank,
                    false,
                    drl_reorder,
                    false,
                );
                let (diff0, diff1) = if use_amvd {
                    let magnitude0 = read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?;
                    let magnitude1 = read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?;
                    apply_inter_mvd_sign_pair(
                        magnitude0,
                        magnitude1,
                        symbols,
                        tile_offset,
                        config,
                        false,
                        compound.y_mode.mvd_sign_derivation_threshold(),
                    )?
                } else {
                    let magnitude0 =
                        read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?;
                    let magnitude1 =
                        read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?;
                    apply_inter_mvd_sign_pair(
                        magnitude0,
                        magnitude1,
                        symbols,
                        tile_offset,
                        config,
                        ref_mv_idx == 0
                            && inter_mvd_sign_derivation_allowed(
                                sequence,
                                core,
                                SINGLE_MODE_NEWMV,
                                use_amvd,
                                frame_mv_config,
                                config,
                            ),
                        compound.y_mode.mvd_sign_derivation_threshold(),
                    )?
                };
                let pred_mv = if use_amvd {
                    stack.candidate(ref_mv_idx)
                } else {
                    lowered_pred_mv(precision, stack.candidate(ref_mv_idx))
                };
                compound.mv0 = Mv {
                    row: mv_clamp_to_integer(pred_mv.row + diff0.row),
                    col: mv_clamp_to_integer(pred_mv.col + diff0.col),
                };
                compound.mv1 = Mv {
                    row: mv_clamp_to_integer(pred_mv.row + diff1.row),
                    col: mv_clamp_to_integer(pred_mv.col + diff1.col),
                };
            }
        }
    }
    if compound.use_optflow {
        return Err(compound_cap!(
            "compound_optflow_prediction",
            tile_offset,
            "inter.compound.optflow_prediction",
            SPEC_PREDICT_OPTFLOW
        ));
    }
    if compound_refinemv_reachable(
        sequence,
        core,
        reference,
        ref_frame_idx,
        compound,
        n4w,
        n4h,
        tile_offset,
    )? {
        return Err(compound_cap!(
            "compound_refinemv_enabled",
            tile_offset,
            "inter.compound.refinemv",
            SPEC_READ_REFINEMV
        ));
    }
    let compound_blend_tools = CompoundBlendToolConfig::from_sequence(sequence);
    let compound_blend_thin = compound_blend_is_thin(n4w, n4h);
    let comp_group_idx_ctx = if !compound_blend_tools.masked_enabled || compound_blend_thin {
        0
    } else {
        compound_group_idx_context(
            neighbour_ctx,
            reference,
            ref_frame_idx,
            core,
            compound.ref_frame0,
            compound.ref_frame1,
            num_total_refs,
            tile_offset,
        )?
    };
    let compound_blend = read_compound_blend_syntax(
        cdfs,
        symbols,
        compound_blend_tools,
        CompoundBlendInput {
            skip_mode: false,
            n4w,
            n4h,
            block_size_index: frontier.b_size.index(),
            comp_group_idx_ctx,
        },
        tile_offset,
    )?;
    let compound_blend = read_compound_cwp_syntax(
        cdfs,
        symbols,
        CompoundCwpContext {
            sequence,
            core,
            reference,
            ref_frame_idx,
        },
        CompoundCwpInput {
            y_mode: compound.y_mode,
            jmvd_scale_mode,
            skip_mode: false,
            ref_frame0: compound.ref_frame0,
            ref_frame1: compound.ref_frame1,
            blend: compound_blend,
        },
        tile_offset,
    )?;
    if compound_all_opfl_reachable(
        core,
        reference,
        ref_frame_idx,
        compound,
        n4w,
        n4h,
        compound_blend,
        tile_offset,
    )? {
        return Err(compound_cap!(
            "compound_opfl_refine_all_active",
            tile_offset,
            "inter.compound.opfl_refine",
            SPEC_MODE_INFO
        ));
    }
    let interp_ctx = neighbour_ctx.interp_filter_ctx(compound.ref_frame0, true);
    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        true,
        interp_ctx,
        tile_offset,
    )?;
    mv_grid.record_compound_block(
        mi_row,
        mi_col,
        n4w,
        n4h,
        compound.ref_frame0,
        compound.ref_frame1,
        compound.y_mode.list0_is_newmv(),
        compound.y_mode.list1_is_newmv(),
        compound.mv0,
        compound.mv1,
        skip == 1,
        interp_filter_symbol(interp),
        use_amvd,
        !matches!(compound_blend, mc::CompoundBlend::Average { .. }),
        precision,
    );
    record_temporal_motion_block(
        motion_field,
        reference,
        ref_frame_idx,
        mi_row,
        mi_col,
        n4w,
        n4h,
        mi_rows,
        mi_cols,
        compound.ref_frame0,
        Some(compound.ref_frame1),
        compound.mv0,
        compound.mv1,
        None,
    );
    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.update_for_block(
            compound.ref_frame0,
            Some(compound.ref_frame1),
            compound.mv0,
            Some(compound.mv1),
            mi_row,
            mi_col,
            n4w,
            n4h,
            sb_h4,
        );
    }
    let residual = if skip == 0 {
        if !residual_quantizer_deltas_are_zero {
            return Err(compound_cap!(
                "compound_block_residual_quantizer_delta",
                tile_offset,
                "inter.compound.residual.nonzero_quantizer_delta",
                SPEC_MODE_INFO
            ));
        }
        if !inter_residual_geometry_supported(frontier) {
            return Err(compound_cap!(
                "compound_block_chroma_partitioned_residual",
                tile_offset,
                "inter.compound.residual.chroma_partition_geometry",
                SPEC_MODE_INFO
            ));
        }
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            sequence,
            core,
            frontier,
            n4w,
            n4h,
            mi_rows,
            mi_cols,
            current_residual_lossless(work_unit),
            InterResidualLumaTxSizeMode::Inter,
            residual_tool_policy,
            tile_offset,
        )?)
    } else {
        reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
        None
    };
    record_inter_deblock_geometry(
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        frontier,
        (n4w, n4h),
        sequence.general.chroma_format_idc,
        residual.as_ref(),
        block_qindex,
        current_residual_lossless(work_unit),
        tile_offset,
    )?;
    let placed_geometry = placed_inter_geometry(frontier, n4w, n4h, tile_offset)?;
    let placed = PlacedInterBlock {
        luma_x: placed_geometry.luma_x,
        luma_y: placed_geometry.luma_y,
        luma_w: placed_geometry.luma_w,
        luma_h: placed_geometry.luma_h,
        chroma_luma_x: placed_geometry.chroma_luma_x,
        chroma_luma_y: placed_geometry.chroma_luma_y,
        chroma_luma_w: placed_geometry.chroma_luma_w,
        chroma_luma_h: placed_geometry.chroma_luma_h,
        has_chroma: placed_geometry.has_chroma,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: compound.ref_frame0,
            ref_frame1: Some(compound.ref_frame1),
            mv: compound.mv0,
            mv1: compound.mv1,
            interp,
            warp_params: None,
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend,
            residual,
        },
    };
    reconstruct_placed_inter_block(
        workspace,
        &placed,
        block_decoded,
        ref_frame_idx,
        reference,
        block_qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        sequence_enables_ibp(sequence),
        tile_offset,
    )?;
    intrabc_state.record_block(
        frontier.r,
        frontier.c,
        n4w,
        n4h,
        IntrabcBlockPrelude::from_use_skip(
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: skip == 1,
            },
            None,
        ),
        tile_offset,
    )?;
    Ok(non_intra_leaf_mode(frontier))
}

fn compound_switchable_opfl_reachable<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if compound_opfl_refine_type(core, tile_offset)? != REFINE_SWITCHABLE
        || !compound_opfl_block_size_allowed(n4w, n4h)
    {
        return Ok(false);
    }
    compound_opfl_reference_allowed(core, reference, ref_frame_idx, compound, tile_offset)
}

fn read_compound_use_optflow_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let use_optflow = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseOptflow {
                ctx: usize::from(y_mode != CompoundYMode::NearNear),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(use_optflow.get() != 0)
}

const fn compound_reads_second_drl(
    compound: super::super::compound::CompoundBlockSyntax,
    skip_mode_present: bool,
) -> bool {
    !compound.use_optflow && compound.y_mode.has_second_drl(skip_mode_present)
}

#[allow(clippy::too_many_arguments)]
fn compound_all_opfl_reachable<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    blend: mc::CompoundBlend,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if compound_opfl_refine_type(core, tile_offset)? != REFINE_ALL
        || !compound_opfl_block_size_allowed(n4w, n4h)
        || !matches!(blend, mc::CompoundBlend::Average { .. })
    {
        return Ok(false);
    }
    compound_opfl_reference_allowed(core, reference, ref_frame_idx, compound, tile_offset)
}

fn compound_opfl_refine_type(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<u32> {
    core.inter
        .as_ref()
        .and_then(|inter| inter.opfl_refine_type)
        .ok_or_else(|| {
            compound_missing!(
                "compound_missing_opfl_refine_type",
                tile_offset,
                "inter.compound.opfl_refine_type",
                SPEC_MODE_INFO
            )
        })
}

const fn compound_opfl_block_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2
}

fn compound_opfl_reference_allowed<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some((d0, d1)) = compound_sized_reference_distances(
        core,
        reference,
        ref_frame_idx,
        compound,
        CompoundReferencePath::Opfl,
        tile_offset,
    )?
    else {
        return Ok(false);
    };
    Ok((d0 <= 0) ^ (d1 <= 0))
}

#[allow(clippy::too_many_arguments)]
fn compound_refinemv_reachable<T: ReconSample>(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some(seq_inter) = sequence.inter.as_ref() else {
        return Err(compound_missing!(
            "compound_refinemv_missing_sequence_inter",
            tile_offset,
            "inter.sequence_tools",
            SPEC_MODE_INFO
        ));
    };
    if !seq_inter.enable_refinemv || !compound_refinemv_size_allowed(n4w, n4h) {
        return Ok(false);
    }
    if !compound_refinemv_mode_allowed(core, compound.y_mode, tile_offset)? {
        return Ok(false);
    }
    compound_refinemv_reference_allowed(core, reference, ref_frame_idx, compound, tile_offset)
}

const fn compound_refinemv_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2 && (n4w >= 4 || n4h >= 4)
}

fn compound_refinemv_mode_allowed(
    core: &FrameHeaderCore,
    y_mode: CompoundYMode,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let opfl_refine_type = core
        .inter
        .as_ref()
        .and_then(|inter| inter.opfl_refine_type)
        .ok_or_else(|| {
            compound_missing!(
                "compound_refinemv_missing_opfl_refine_type",
                tile_offset,
                "inter.compound.opfl_refine_type",
                SPEC_READ_REFINEMV
            )
        })?;
    Ok(!(opfl_refine_type == REFINE_SWITCHABLE && y_mode.has_newmv()))
}

fn compound_refinemv_reference_allowed<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some((d0, d1)) = compound_sized_reference_distances(
        core,
        reference,
        ref_frame_idx,
        compound,
        CompoundReferencePath::RefineMv,
        tile_offset,
    )?
    else {
        return Ok(false);
    };
    Ok(d0 != 0 && d0 == -d1)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompoundReferencePath {
    Opfl,
    RefineMv,
}

fn compound_sized_reference_distances<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    path: CompoundReferencePath,
    tile_offset: ByteOffset,
) -> Result<Option<(i32, i32)>> {
    if core.frame_type == Some(FrameType::Switch) {
        return Ok(None);
    }
    let Some(frame_size) = core.frame_size else {
        return Err(match path {
            CompoundReferencePath::Opfl => compound_missing!(
                "compound_opfl_missing_frame_size",
                tile_offset,
                "inter.compound.frame_size",
                SPEC_MODE_INFO
            ),
            CompoundReferencePath::RefineMv => compound_missing!(
                "compound_refinemv_missing_frame_size",
                tile_offset,
                "inter.compound.frame_size",
                SPEC_READ_REFINEMV
            ),
        });
    };
    let ref0 =
        compound_reference_facts(reference, ref_frame_idx, compound.ref_frame0, tile_offset)?;
    let ref1 =
        compound_reference_facts(reference, ref_frame_idx, compound.ref_frame1, tile_offset)?;
    if ref0.width != frame_size.width
        || ref0.height != frame_size.height
        || ref1.width != frame_size.width
        || ref1.height != frame_size.height
    {
        return Ok(None);
    }
    let current = compound_current_order_hint(core, tile_offset)?;
    let d0 = super::super::get_relative_dist(current, ref0.order_hint);
    let d1 = super::super::get_relative_dist(current, ref1.order_hint);
    Ok(Some((d0, d1)))
}

fn compound_current_order_hint(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<i32> {
    i32::try_from(core.order_hint_lsb.unwrap_or(0)).map_err(|_| {
        compound_cap!(
            "compound_order_hint_range",
            tile_offset,
            "inter.compound.order_hint",
            SPEC_MODE_INFO
        )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundReferenceFacts {
    order_hint: i32,
    width: u32,
    height: u32,
}

fn compound_reference_facts<T: ReconSample>(
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
    tile_offset: ByteOffset,
) -> Result<CompoundReferenceFacts> {
    let ref_index = usize::try_from(ref_frame).map_err(|_| {
        compound_cap!(
            "compound_ref_frame_range",
            tile_offset,
            "inter.compound.ref_frame",
            SPEC_READ_REFINEMV
        )
    })?;
    let slot = *ref_frame_idx.get(ref_index).ok_or_else(|| {
        compound_missing!(
            "compound_refinemv_missing_ref_frame_idx",
            tile_offset,
            "inter.compound.ref_frame_idx",
            SPEC_READ_REFINEMV
        )
    })?;
    let slot = usize::try_from(slot).map_err(|_| {
        compound_cap!(
            "compound_ref_slot_range",
            tile_offset,
            "inter.compound.ref_slot",
            SPEC_READ_REFINEMV
        )
    })?;
    let order_hint = reference
        .ref_order_hint
        .get(slot)
        .copied()
        .ok_or_else(|| {
            compound_missing!(
                "compound_refinemv_missing_ref_order_hint",
                tile_offset,
                "inter.compound.ref_order_hint",
                SPEC_READ_REFINEMV
            )
        })
        .and_then(|hint| {
            i32::try_from(hint).map_err(|_| {
                compound_cap!(
                    "compound_ref_order_hint_range",
                    tile_offset,
                    "inter.compound.ref_order_hint",
                    SPEC_READ_REFINEMV
                )
            })
        })?;
    let width = *reference.ref_frame_width.get(slot).ok_or_else(|| {
        compound_missing!(
            "compound_missing_ref_width",
            tile_offset,
            "inter.compound.ref_width",
            SPEC_READ_REFINEMV
        )
    })?;
    let height = *reference.ref_frame_height.get(slot).ok_or_else(|| {
        compound_missing!(
            "compound_missing_ref_height",
            tile_offset,
            "inter.compound.ref_height",
            SPEC_READ_REFINEMV
        )
    })?;
    Ok(CompoundReferenceFacts {
        order_hint,
        width,
        height,
    })
}

fn compound_ref_contexts(
    neighbour_ctx: &BlockNeighbourContext,
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<[usize; 7]> {
    let mut contexts = [0usize; 7];
    for (ref_idx, ctx) in contexts.iter_mut().take(num_total_refs).enumerate() {
        *ctx = neighbour_ctx
            .single_ref_ctx(ref_idx, num_total_refs)
            .ok_or_else(|| {
                compound_missing!(
                    "compound_block_missing_ref_context",
                    tile_offset,
                    "inter.compound.ref_context",
                    SPEC_MODE_INFO
                )
            })?;
    }
    Ok(contexts)
}

fn compound_ref_distance_signs<T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    current_order_hint: u32,
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<[bool; 7]> {
    let mut signs = [true; 7];
    let current_order_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
    for (ref_idx, sign) in signs.iter_mut().take(num_total_refs).enumerate() {
        let slot = *ref_frame_idx.get(ref_idx).ok_or_else(|| {
            compound_missing!(
                "compound_block_missing_ref_frame_idx",
                tile_offset,
                "inter.compound.ref_frame_idx",
                SPEC_MODE_INFO
            )
        })?;
        let ref_order_hint = reference
            .ref_order_hint
            .get(slot as usize)
            .copied()
            .map(|hint| i32::try_from(hint).unwrap_or(i32::MAX))
            .ok_or_else(|| {
                compound_missing!(
                    "compound_missing_ref_order_hint",
                    tile_offset,
                    "inter.compound.ref_order_hint",
                    SPEC_MODE_INFO
                )
            })?;
        *sign = super::super::get_relative_dist(current_order_hint, ref_order_hint) >= 0;
    }
    Ok(signs)
}

#[allow(clippy::too_many_arguments)]
fn compound_group_idx_context<T: ReconSample>(
    neighbour_ctx: &BlockNeighbourContext,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    core: &FrameHeaderCore,
    ref_frame0: i8,
    ref_frame1: i8,
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let current_order_hint = compound_current_order_hint(core, tile_offset)?;
    let ref0_order_hint =
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame0, tile_offset)?;
    let ref1_order_hint =
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame1, tile_offset)?;
    let equal_ref_distance = super::super::get_relative_dist(current_order_hint, ref0_order_hint)
        .abs()
        == super::super::get_relative_dist(ref1_order_hint, current_order_hint).abs();
    let furthest_future_ref = compound_furthest_future_ref(
        reference,
        ref_frame_idx,
        current_order_hint,
        num_total_refs,
        tile_offset,
    )?;
    Ok(neighbour_ctx.compound_group_idx_ctx(equal_ref_distance, furthest_future_ref))
}

fn compound_furthest_future_ref<T: ReconSample>(
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    current_order_hint: i32,
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<Option<i8>> {
    let mut best = None;
    for ref_idx in 0..num_total_refs {
        let ref_order_hint =
            compound_reference_order_hint(reference, ref_frame_idx, ref_idx as i8, tile_offset)?;
        let distance = super::super::get_relative_dist(ref_order_hint, current_order_hint);
        if distance <= 0 {
            continue;
        }
        if best.is_none_or(|(best_distance, _)| distance > best_distance) {
            best = Some((distance, ref_idx as i8));
        }
    }
    Ok(best.map(|(_, ref_idx)| ref_idx))
}

fn compound_reference_order_hint<T: ReconSample>(
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
    tile_offset: ByteOffset,
) -> Result<i32> {
    let ref_index = usize::try_from(ref_frame).map_err(|_| {
        compound_cap!(
            "compound_group_ref_frame_range",
            tile_offset,
            "inter.compound.ref_frame",
            SPEC_MODE_INFO
        )
    })?;
    let slot = *ref_frame_idx.get(ref_index).ok_or_else(|| {
        compound_missing!(
            "compound_group_missing_ref_frame_idx",
            tile_offset,
            "inter.compound.ref_frame_idx",
            SPEC_MODE_INFO
        )
    })?;
    let slot = usize::try_from(slot).map_err(|_| {
        compound_cap!(
            "compound_group_ref_slot_range",
            tile_offset,
            "inter.compound.ref_slot",
            SPEC_MODE_INFO
        )
    })?;
    reference
        .ref_order_hint
        .get(slot)
        .copied()
        .ok_or_else(|| {
            compound_missing!(
                "compound_group_missing_ref_order_hint",
                tile_offset,
                "inter.compound.ref_order_hint",
                SPEC_MODE_INFO
            )
        })
        .and_then(|hint| {
            i32::try_from(hint).map_err(|_| {
                compound_cap!(
                    "compound_group_ref_order_hint_range",
                    tile_offset,
                    "inter.compound.ref_order_hint",
                    SPEC_MODE_INFO
                )
            })
        })
}

fn single_ref_block_context(block_ctx: &MvBlockContext, ref_frame0: i8) -> MvBlockContext {
    let mut single = *block_ctx;
    single.ref_frame0 = ref_frame0;
    single.ref_frame1 = None;
    single
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundJointMvProjection {
    base_list: usize,
    first_dist: i32,
    second_dist: i32,
}

fn compound_joint_mv_projection<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    ref_frame0: i8,
    ref_frame1: i8,
    tile_offset: ByteOffset,
) -> Result<CompoundJointMvProjection> {
    let current = compound_current_order_hint(core, tile_offset)?;
    let ref0_order_hint =
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame0, tile_offset)?;
    let ref1_order_hint =
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame1, tile_offset)?;
    let rel0 = super::super::get_relative_dist(ref0_order_hint, current);
    let rel1 = super::super::get_relative_dist(ref1_order_hint, current);
    let mut first_dist = rel0.abs();
    let mut second_dist = rel1.abs();
    let base_list = usize::from(first_dist < second_dist);
    if base_list == 1 {
        core::mem::swap(&mut first_dist, &mut second_dist);
    }
    let same_side = (rel0 < 0 && rel1 < 0) || (rel0 > 0 && rel1 > 0);
    if !same_side {
        second_dist = -second_dist;
    }
    Ok(CompoundJointMvProjection {
        base_list,
        first_dist,
        second_dist,
    })
}

fn project_joint_mvd(diff: Mv, num: i32, den: i32) -> Mv {
    let num = num.clamp(-31, 31);
    let den = den.clamp(1, 31);
    let frac = i64::from(num) * i64::from(MV_PROJECTION_DIV_MULT[den as usize]);
    Mv {
        row: project_joint_mvd_component(diff.row, frac),
        col: project_joint_mvd_component(diff.col, frac),
    }
}

fn project_joint_mvd_component(component: i32, frac: i64) -> i32 {
    let product = i64::from(component) * frac;
    let rounded = (product + 8192 + (product >> 63)) >> 14;
    clamp_i64_to_i32(rounded)
}

fn clamp_i64_to_i32(value: i64) -> i32 {
    if value < i64::from(i32::MIN) {
        i32::MIN
    } else if value > i64::from(i32::MAX) {
        i32::MAX
    } else {
        value as i32
    }
}

fn scale_joint_projected_mvd(mut projected: Mv, jmvd_scale_mode: u8, use_amvd: bool) -> Mv {
    if use_amvd {
        match jmvd_scale_mode {
            1 => {
                projected.row = projected.row.saturating_mul(2);
                projected.col = projected.col.saturating_mul(2);
            }
            2 => {
                projected.row /= 2;
                projected.col /= 2;
            }
            _ => {}
        }
        return projected;
    }
    match jmvd_scale_mode {
        1 => projected.row = projected.row.saturating_mul(2),
        2 => projected.col = projected.col.saturating_mul(2),
        3 => projected.row /= 2,
        4 => projected.col /= 2,
        _ => {}
    }
    projected
}

fn add_mv_clamped(pred: Mv, diff: Mv) -> Mv {
    Mv {
        row: mv_clamp_to_integer(pred.row.saturating_add(diff.row)),
        col: mv_clamp_to_integer(pred.col.saturating_add(diff.col)),
    }
}

fn read_compound_use_amvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    enable_adaptive_mvd: bool,
    y_mode: CompoundYMode,
    use_optflow: bool,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some(index) = y_mode
        .use_amvd_index(use_optflow)
        .filter(|_| enable_adaptive_mvd)
    else {
        return Ok(false);
    };
    let use_amvd = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseAmvd { index, ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(use_amvd.get() != 0)
}

fn read_compound_jmvd_scale_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    use_amvd: bool,
    tile_offset: ByteOffset,
) -> Result<u8> {
    if y_mode != CompoundYMode::JointNew {
        return Ok(0);
    }
    let selector = if use_amvd {
        TileCdfSelector::JmvdAdaptiveScaleMode
    } else {
        TileCdfSelector::JmvdScaleMode
    };
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(splot_core::symbol::Symbol::get)
        .map_err(|_| symbol_read_error(tile_offset))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundBlendInput {
    skip_mode: bool,
    n4w: usize,
    n4h: usize,
    block_size_index: usize,
    comp_group_idx_ctx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundBlendToolConfig {
    masked_enabled: bool,
    implicit_mask: bool,
}

impl CompoundBlendToolConfig {
    fn from_sequence(sequence: &SequenceHeader) -> Self {
        Self {
            masked_enabled: sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_masked_compound),
            implicit_mask: sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_imp_msk_bld),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundCwpInput {
    y_mode: CompoundYMode,
    jmvd_scale_mode: u8,
    skip_mode: bool,
    ref_frame0: i8,
    ref_frame1: i8,
    blend: mc::CompoundBlend,
}

#[derive(Clone, Copy)]
struct CompoundCwpContext<'a, T: ReconSample> {
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    reference: &'a InterReferenceState<'a, T>,
    ref_frame_idx: &'a [u32],
}

fn read_compound_blend_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tools: CompoundBlendToolConfig,
    input: CompoundBlendInput,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlend> {
    let average_blend = mc::CompoundBlend::average_with_implicit_mask(tools.implicit_mask);
    let thin = compound_blend_is_thin(input.n4w, input.n4h);
    if input.skip_mode || !tools.masked_enabled || thin {
        return Ok(average_blend);
    }
    let comp_group_idx = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::CompGroupIdx {
                ctx: input.comp_group_idx_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if comp_group_idx == 0 {
        return Ok(average_blend);
    }
    let compound_type = if wedge_bits(input.block_size_index) == 0 {
        MaskedCompoundType::DiffWeighted
    } else {
        match cdfs
            .read_block_symbol_trace(TileCdfSelector::CompoundType, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
        {
            0 => MaskedCompoundType::Wedge,
            1 => MaskedCompoundType::DiffWeighted,
            _ => {
                return Err(compound_cap!(
                    "compound_type_out_of_range",
                    tile_offset,
                    "inter.compound.compound_type",
                    SPEC_MODE_INFO
                ));
            }
        }
    };
    match compound_type {
        MaskedCompoundType::DiffWeighted => {
            let mask_type = symbols
                .read_literal(1)
                .map_err(|_| symbol_read_error(tile_offset))?
                != 0;
            Ok(mc::CompoundBlend::DiffWeighted { inverse: mask_type })
        }
        MaskedCompoundType::Wedge => {
            let index = read_wedge_mode_syntax(cdfs, symbols, tile_offset)?;
            let sign = symbols
                .read_bool()
                .map_err(|_| symbol_read_error(tile_offset))?;
            Ok(mc::CompoundBlend::Wedge { index, sign })
        }
    }
}

const fn compound_blend_is_thin(n4w: usize, n4h: usize) -> bool {
    (n4w == 1 && n4h >= 4) || (n4h == 1 && n4w >= 4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaskedCompoundType {
    Wedge,
    DiffWeighted,
}

fn wedge_bits(block_size_index: usize) -> u8 {
    const WEDGE_BITS: [u8; 29] = [
        0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 4, 4, 0, 0, 4, 4,
    ];
    WEDGE_BITS.get(block_size_index).copied().unwrap_or(0)
}

const CWP_WEIGHTING_FACTOR: [[i16; 5]; 2] = [[8, 12, 4, 10, 6], [8, 12, 4, 20, -4]];

fn read_compound_cwp_syntax<T: ReconSample>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    context: CompoundCwpContext<'_, T>,
    input: CompoundCwpInput,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlend> {
    let cwp_enabled = context
        .sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_cwp);
    if !cwp_enabled
        || input.skip_mode
        || !compound_cwp_mode_allowed(input.y_mode, input.jmvd_scale_mode)
        || !matches!(input.blend, mc::CompoundBlend::Average { .. })
    {
        return Ok(input.blend);
    }
    let mut coding_idx = 0usize;
    for idx in 0..CWP_WEIGHTING_FACTOR[0].len() - 1 {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::CwpIdx { idx }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        coding_idx = idx + usize::from(symbol != 0);
        if symbol == 0 {
            break;
        }
    }
    let same_side = compound_cwp_same_side(
        context.core,
        context.reference,
        context.ref_frame_idx,
        input.ref_frame0,
        input.ref_frame1,
        tile_offset,
    )?;
    Ok(input
        .blend
        .average_with_cwp_weight(CWP_WEIGHTING_FACTOR[usize::from(same_side)][coding_idx]))
}

const fn compound_cwp_mode_allowed(y_mode: CompoundYMode, jmvd_scale_mode: u8) -> bool {
    matches!(y_mode, CompoundYMode::NearNear)
        || matches!(y_mode, CompoundYMode::JointNew) && jmvd_scale_mode == 0
}

fn compound_cwp_same_side<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    ref_frame0: i8,
    ref_frame1: i8,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let current = compound_current_order_hint(core, tile_offset)?;
    let d0 = super::super::get_relative_dist(
        current,
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame0, tile_offset)?,
    );
    let d1 = super::super::get_relative_dist(
        current,
        compound_reference_order_hint(reference, ref_frame_idx, ref_frame1, tile_offset)?,
    );
    Ok((d0 < 0 && d1 < 0) || (d0 > 0 && d1 > 0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
    use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

    use super::*;
    use crate::bitstream::tile_payload::FrameCdfSubset;

    const TILE_OFFSET: ByteOffset = ByteOffset::new(0);

    fn encode_wedge_compound_blend() -> Vec<u8> {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::with_config(
            SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        );
        tile.with_row_mut(TileCdfSelector::CompGroupIdx { ctx: 0 }, |row| {
            encoder.write_symbol(row, Symbol::new(1))
        })
        .unwrap()
        .unwrap();
        tile.with_row_mut(TileCdfSelector::CompoundType, |row| {
            encoder.write_symbol(row, Symbol::new(0))
        })
        .unwrap()
        .unwrap();
        tile.with_row_mut(TileCdfSelector::WedgeQuad, |row| {
            encoder.write_symbol(row, Symbol::new(0))
        })
        .unwrap()
        .unwrap();
        tile.with_row_mut(TileCdfSelector::WedgeAngle { quad: 0 }, |row| {
            encoder.write_symbol(row, Symbol::new(0))
        })
        .unwrap()
        .unwrap();
        tile.with_row_mut(TileCdfSelector::WedgeDist2, |row| {
            encoder.write_symbol(row, Symbol::new(0))
        })
        .unwrap()
        .unwrap();
        encoder.write_bool(true).unwrap();
        encoder.finish().unwrap().into_bytes()
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
    fn compound_blend_reads_wedge_index_and_sign() {
        let payload = encode_wedge_compound_blend();
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);

        let blend = read_compound_blend_syntax(
            &mut tile,
            &mut symbols,
            CompoundBlendToolConfig {
                masked_enabled: true,
                implicit_mask: false,
            },
            CompoundBlendInput {
                skip_mode: false,
                n4w: 2,
                n4h: 2,
                block_size_index: 3,
                comp_group_idx_ctx: 0,
            },
            TILE_OFFSET,
        )
        .unwrap();

        assert_eq!(
            blend,
            mc::CompoundBlend::Wedge {
                index: 0,
                sign: true,
            }
        );
    }

    #[test]
    fn joint_mvd_projection_uses_reference_distance_ratio() {
        assert_eq!(
            project_joint_mvd(Mv { row: 96, col: -48 }, 1, 2),
            Mv { row: 48, col: -24 }
        );
        assert_eq!(
            project_joint_mvd(Mv { row: 96, col: -48 }, -1, 2),
            Mv { row: -48, col: 24 }
        );
    }

    #[test]
    fn joint_mvd_scale_mode_matches_amvd_and_non_amvd_axes() {
        assert_eq!(
            scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 1, true),
            Mv { row: 20, col: 12 }
        );
        assert_eq!(
            scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 2, false),
            Mv { row: 10, col: 12 }
        );
        assert_eq!(
            scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 4, false),
            Mv { row: 10, col: 3 }
        );
    }

    #[test]
    fn compound_cwp_mode_allows_unscaled_joint_newmv() {
        assert!(compound_cwp_mode_allowed(CompoundYMode::NearNear, 4));
        assert!(compound_cwp_mode_allowed(CompoundYMode::JointNew, 0));
        assert!(!compound_cwp_mode_allowed(CompoundYMode::JointNew, 1));
        assert!(!compound_cwp_mode_allowed(CompoundYMode::NearNew, 0));
    }

    #[test]
    fn compound_opfl_mode_suppresses_second_drl_idx() {
        let mut compound = crate::prediction::inter::compound::CompoundBlockSyntax {
            y_mode: CompoundYMode::NearNear,
            use_optflow: false,
            ref_frame0: 0,
            ref_frame1: 1,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
        };

        assert!(compound_reads_second_drl(compound, false));
        compound.use_optflow = true;
        assert!(!compound_reads_second_drl(compound, false));
    }
}
