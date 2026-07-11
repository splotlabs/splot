// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_recon::wedge_mask_plane_sample;

use super::super::compound::{
    CompoundParseInput, CompoundYMode, read_compound_mode_syntax, read_compound_reference_pair,
};
use super::super::find_mv_stack::OrderHintMvContext;
use super::super::read_mv::apply_inter_mvd_sign_pair;
use super::*;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const REFINE_SWITCHABLE: u32 = 1;
const REFINE_ALL: u32 = 2;
const RESTRICTED_ORDER_HINT: i32 = -1;
const MV_PROJECTION_DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];
const SPEC_READ_REFINEMV: &str = "5.20.7.17";

pub(super) fn read_reference_mode(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    neighbour_ctx: &BlockNeighbourContext,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let current_order_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
    let ctx = neighbour_ctx.comp_mode_ctx(ref_frame_idx, ref_order_hint, current_order_hint);
    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::CompMode { ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    match mode {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(inter_cap!(
            "inter_block_reference_mode",
            tile_offset,
            "inter.reference_mode out of range",
            SPEC_MODE_INFO
        )),
    }
}

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
    order_hints: OrderHintMvContext<'_>,
    tip_ref_pair: Option<(i8, i8)>,
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
        },
        tile_offset,
    )?;
    let compound_is_joint_ctx = super::super::compound_is_joint_context(
        ref_frame_idx,
        &reference.ref_order_hint,
        pair,
        compound_current_order_hint(core, tile_offset)?,
        tile_offset,
    )?;
    block_ctx.ref_frame0 = pair.0;
    block_ctx.ref_frame1 = Some(pair.1);
    let mode_ctx = find_mode_ctx_with_tip(mv_grid, block_ctx, tip_ref_pair);
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
    let frame_modes = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .unwrap_or([false; splot_core::headers::frame::MOTION_MODES]);
    let signal_local_warp = compound_local_warp_signal_allowed(
        compound,
        n4w,
        n4h,
        effective_force_integer_mv(core),
        compound_opfl_refine_type(core, tile_offset)?,
        [mode_ctx.warp_sample_found, mode_ctx.warp_sample_found1],
        frame_modes[splot_core::headers::frame::LOCALWARP],
    );
    let local_warp = read_compound_motion_mode_syntax(
        cdfs,
        symbols,
        signal_local_warp,
        neighbour_ctx,
        tile_offset,
    )?;
    let jmvd_scale_mode = read_compound_jmvd_scale_mode_syntax(
        cdfs,
        symbols,
        compound.y_mode,
        use_amvd,
        tile_offset,
    )?;
    let mut ref_mv_idx = 0;
    let mut ref_mv_idx0 = 0;
    let mut ref_mv_idx1 = 0;
    if compound.y_mode.reads_drl_idx() {
        if compound_reads_second_drl(compound) {
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
        let paired_candidate = |idx| {
            find_compound_mv_stack_with_temporal(
                mv_grid,
                block_ctx,
                [Mv::ZERO; 2],
                bank,
                drl_reorder,
                temporal,
            )
            .candidate(idx)
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
        let independent_candidates = |idx0, idx1| {
            let stack0 = find_mv_stack_with_temporal(
                mv_grid,
                &single_ref_block_context(block_ctx, compound.ref_frame0),
                Mv::ZERO,
                bank,
                warp_param_bank,
                false,
                drl_reorder,
                temporal_context,
                Some(order_hints),
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
                temporal_context,
                Some(order_hints),
                temporal_first1,
            );
            [stack0.candidate(idx0), stack1.candidate(idx1)]
        };
        match compound.y_mode {
            CompoundYMode::GlobalGlobal => {}
            CompoundYMode::NearNear => {
                [compound.mv0, compound.mv1] = select_near_near_candidates(
                    compound,
                    ref_mv_idx,
                    [ref_mv_idx0, ref_mv_idx1],
                    |idx| paired_candidate(idx).mvs,
                    |[idx0, idx1]| independent_candidates(idx0, idx1),
                );
            }
            CompoundYMode::NearNew | CompoundYMode::NewNear => {
                let has_second_drl = compound_reads_second_drl(compound);
                let candidates = if has_second_drl {
                    independent_candidates(ref_mv_idx0, ref_mv_idx1)
                } else {
                    paired_candidate(ref_mv_idx).mvs
                };
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
                let projection = compound_joint_mv_projection(
                    core,
                    reference,
                    ref_frame_idx,
                    compound.ref_frame0,
                    compound.ref_frame1,
                    tile_offset,
                )?;
                let candidates = paired_candidate(ref_mv_idx).mvs;
                let raw_pred_mv = candidates[projection.base_list];
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
                let other_mv = add_mv_clamped(candidates[1 - projection.base_list], projected);
                if projection.base_list == 0 {
                    compound.mv0 = base_mv;
                    compound.mv1 = other_mv;
                } else {
                    compound.mv0 = other_mv;
                    compound.mv1 = base_mv;
                }
            }
            CompoundYMode::NewNew => {
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
                let candidate = paired_candidate(ref_mv_idx).mvs;
                let pred_mvs = if use_amvd {
                    candidate
                } else {
                    candidate.map(|mv| lowered_pred_mv(precision, mv))
                };
                compound.mv0 = Mv {
                    row: mv_clamp_to_integer(pred_mvs[0].row + diff0.row),
                    col: mv_clamp_to_integer(pred_mvs[0].col + diff0.col),
                };
                compound.mv1 = Mv {
                    row: mv_clamp_to_integer(pred_mvs[1].row + diff1.row),
                    col: mv_clamp_to_integer(pred_mvs[1].col + diff1.col),
                };
            }
        }
    }
    let warp_models = if local_warp {
        compound_local_warp_models(
            mv_grid,
            block_ctx,
            compound.mv0,
            compound.mv1,
            mi_row,
            mi_col,
            n4w,
            n4h,
            tile_offset,
        )?
    } else {
        [None, None]
    };
    let refinemv_switchable =
        compound_refinemv_is_switchable(compound, compound_opfl_refine_type(core, tile_offset)?);
    let refinemv_signalled = if !local_warp
        && compound_refinemv_reachable(
            sequence,
            core,
            reference,
            ref_frame_idx,
            compound,
            n4w,
            n4h,
            tile_offset,
        )? {
        if refinemv_switchable {
            read_compound_use_refinemv_syntax(
                cdfs,
                symbols,
                compound.y_mode,
                compound.use_optflow,
                tile_offset,
            )?
        } else {
            true
        }
    } else {
        false
    };
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
            use_optflow: compound.use_optflow,
            joint_amvd: compound.y_mode == CompoundYMode::JointNew && use_amvd,
            switchable_refinemv_on: refinemv_signalled && refinemv_switchable,
            n4w,
            n4h,
            block_size_index: frontier.b_size.index(),
            comp_group_idx_ctx,
        },
        tile_offset,
    )?;
    let use_refinemv = compound_refinemv_active_after_blend(refinemv_signalled, compound_blend);
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
            use_optflow: compound.use_optflow,
            use_refinemv,
            motion_simple: !local_warp,
            ref_frame0: compound.ref_frame0,
            ref_frame1: compound.ref_frame1,
            blend: compound_blend,
        },
        tile_offset,
    )?;
    let compound_blend = compound_warp_blend(compound_blend, local_warp);
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
    let interp = resolve_compound_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        compound.use_optflow || refinemv_signalled,
        !local_warp && compound.y_mode != CompoundYMode::GlobalGlobal,
        neighbour_ctx.interp_filter_ctx(compound.ref_frame0, true),
        tile_offset,
    )?;
    if let Some(params) = warp_models[0] {
        warp_param_bank.update(compound.ref_frame0, params);
    }
    if let Some(params) = warp_models[1] {
        warp_param_bank.update(compound.ref_frame1, params);
    }
    reconstruct_resolved_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        sequence,
        core,
        frontier,
        workspace,
        block_decoded,
        mv_grid,
        motion_field,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        intrabc_state,
        ref_frame_idx,
        reference,
        ref_mv_bank,
        ResolvedCompoundBlock {
            syntax: compound,
            blend: compound_blend,
            interp,
            use_amvd,
            precision,
            skip_mode: false,
            use_refinemv,
            refinemv_switchable,
            warp_params: warp_models,
        },
        skip,
        n4w,
        n4h,
        mi_row,
        mi_col,
        mi_rows,
        mi_cols,
        sb_h4,
        residual_quantizer_deltas_are_zero,
        residual_tool_policy,
        block_qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        tile_offset,
    )
}

fn resolve_compound_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    force_sharp: bool,
    needs_interp_filter: bool,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    if force_sharp {
        return Ok(ReconInterpolationFilter::EightTapSharp);
    }
    resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        needs_interp_filter,
        ctx,
        tile_offset,
    )
}

fn compound_local_warp_signal_allowed(
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    force_integer_mv: bool,
    opfl_refine_type: u32,
    warp_sample_found: [bool; 2],
    local_warp_enabled: bool,
) -> bool {
    compound.y_mode == CompoundYMode::NewNew
        && compound.ref_frame0 != compound.ref_frame1
        && n4w >= 2
        && n4h >= 2
        && !force_integer_mv
        && !compound.use_optflow
        && opfl_refine_type != REFINE_ALL
        && warp_sample_found[0]
        && warp_sample_found[1]
        && local_warp_enabled
}

/// AV2 § 7.13.3.23: § 7.12.3 warp-sample search plus least-squares estimation
/// once per reference list. A signalled list that gathers no samples (the
/// § 5.20.7.14 `WarpSampleFound` scan is wider than the § 7.12.3.1 gathering)
/// fails closed: the spec's det==0 identity model and AVM's
/// `wm_params[ref].invalid` translational fallback diverge in that corner.
#[allow(clippy::too_many_arguments)]
fn compound_local_warp_models(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    mv0: Mv,
    mv1: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[Option<[i64; 6]>; 2]> {
    let ref_frame1 = block_ctx.ref_frame1.ok_or_else(|| {
        compound_missing!(
            "compound_local_warp_missing_ref_frame1",
            tile_offset,
            "inter.compound.local_warp.ref_frame1",
            "5.20.7.14"
        )
    })?;
    let model0 = compound_ref_warp_model(
        mv_grid,
        block_ctx,
        block_ctx.ref_frame0,
        mv0,
        mi_row,
        mi_col,
        n4w,
        n4h,
        tile_offset,
    )?;
    let model1 = compound_ref_warp_model(
        mv_grid,
        block_ctx,
        ref_frame1,
        mv1,
        mi_row,
        mi_col,
        n4w,
        n4h,
        tile_offset,
    )?;
    Ok([model0, model1])
}

#[allow(clippy::too_many_arguments)]
fn compound_ref_warp_model(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    target_ref: i8,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<Option<[i64; 6]>> {
    match super::super::find_mv_stack::find_warp_samples(mv_grid, block_ctx, target_ref) {
        super::super::find_mv_stack::WarpSampleCollection::Samples(samples) => {
            if samples.is_empty() {
                return Err(inter_cap!(
                    "compound_local_warp_empty_sample_list",
                    tile_offset,
                    "inter.compound.local_warp.empty_sample_list",
                    "7.12.3.1"
                ));
            }
            Ok(Some(local_warp_estimation(
                &samples,
                mv,
                mi_row,
                mi_col,
                n4w,
                n4h,
                tile_offset,
            )?))
        }
        super::super::find_mv_stack::WarpSampleCollection::List1MvUnretained => Err(inter_cap!(
            "compound_warp_sample_list1_mv_unretained",
            tile_offset,
            "inter.compound.local_warp.second_list_neighbour_mv",
            "7.12.3.2"
        )),
    }
}

/// AV2 § 7.13.3.14: compound LOCALWARP (`compoundWarp = 1`) disables the
/// implicit-mask average branch, so COMPOUND_AVERAGE collapses to the plain
/// weighted average; wedge/diff-weighted blends are unaffected.
fn compound_warp_blend(blend: mc::CompoundBlend, local_warp: bool) -> mc::CompoundBlend {
    if !local_warp {
        return blend;
    }
    match blend {
        mc::CompoundBlend::Average { cwp_weight, .. } => {
            mc::CompoundBlend::average_with_implicit_mask(false).average_with_cwp_weight(cwp_weight)
        }
        other => other,
    }
}

/// AV2 § 5.20.7.14 `read_motion_mode`: reads `use_local_warp` when LOCALWARP is
/// allowed and returns whether the block uses compound LOCALWARP (AVM WARP_CAUSAL).
fn read_compound_motion_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    signal_local_warp: bool,
    neighbour_ctx: &BlockNeighbourContext,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if !signal_local_warp {
        return Ok(false);
    }
    let use_local_warp = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseLocalWarp {
                ctx: neighbour_ctx.use_local_warp_ctx(),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    Ok(use_local_warp != 0)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ResolvedCompoundBlock {
    pub(super) syntax: super::super::compound::CompoundBlockSyntax,
    pub(super) blend: mc::CompoundBlend,
    pub(super) interp: ReconInterpolationFilter,
    pub(super) use_amvd: bool,
    pub(super) precision: BlockPrecisionRecord,
    pub(super) skip_mode: bool,
    pub(super) use_refinemv: bool,
    pub(super) refinemv_switchable: bool,
    /// Per-list § 7.13.3.23 LOCALWARP models (`[None, None]` when translational).
    pub(super) warp_params: [Option<[i64; 6]>; 2],
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_resolved_compound_inter_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    mv_grid: &mut NeighbourMvGrid,
    motion_field: &mut TemporalMotionField,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    ref_mv_bank: &mut Option<super::super::find_mv_stack::RefMvBank>,
    resolved: ResolvedCompoundBlock,
    skip: u8,
    n4w: usize,
    n4h: usize,
    mi_row: usize,
    mi_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    residual_quantizer_deltas_are_zero: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let ResolvedCompoundBlock {
        syntax: compound,
        blend: compound_blend,
        interp,
        use_amvd,
        precision,
        skip_mode,
        use_refinemv,
        refinemv_switchable,
        warp_params,
    } = resolved;
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
        compound_blend.cwp_weight(),
        skip_mode,
        precision,
        warp_params,
    );
    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.update_for_block(
            compound.ref_frame0,
            Some(compound.ref_frame1),
            compound.mv0,
            Some(compound.mv1),
            compound_blend.cwp_weight(),
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
    let sub_pu_size = compound_deblock_sub_pu_size(
        compound.use_optflow,
        use_refinemv,
        n4w * MI_SIZE,
        n4h * MI_SIZE,
    );
    record_inter_deblock_geometry(
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        frontier,
        (n4w, n4h),
        sequence.general.chroma_format_idc,
        residual.as_ref(),
        sub_pu_size,
        block_qindex,
        current_residual_lossless(work_unit),
        tile_offset,
    )?;
    let placed_geometry = placed_inter_geometry(
        frontier,
        n4w,
        n4h,
        sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
        tile_offset,
    )?;
    let optflow_distances = if compound.use_optflow {
        compound_sized_reference_distances(
            core,
            reference,
            ref_frame_idx,
            compound,
            CompoundReferencePath::Opfl,
            tile_offset,
        )?
        .map(|(dist0, dist1)| [dist0, dist1])
    } else {
        None
    };
    let placed = PlacedInterBlock {
        luma_x: placed_geometry.luma_x,
        luma_y: placed_geometry.luma_y,
        luma_w: placed_geometry.luma_w,
        luma_h: placed_geometry.luma_h,
        chroma_luma_x: placed_geometry.chroma_luma_x,
        chroma_luma_y: placed_geometry.chroma_luma_y,
        chroma_luma_w: placed_geometry.chroma_luma_w,
        chroma_luma_h: placed_geometry.chroma_luma_h,
        predict_chroma: placed_geometry.predict_chroma,
        sub8x8_chroma: placed_geometry.sub8x8_chroma,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: compound.ref_frame0,
            ref_frame1: Some(compound.ref_frame1),
            mv: compound.mv0,
            mv1: compound.mv1,
            interp,
            warp_params,
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend,
            optflow_distances,
            residual,
        },
    };
    let motion_grid = super::prediction::reconstruct_placed_inter_block(
        workspace,
        &placed,
        use_refinemv,
        refinemv_switchable,
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
    record_compound_temporal_motion(
        motion_field,
        reference,
        ref_frame_idx,
        &placed,
        compound,
        warp_params,
        motion_grid.as_ref(),
        mi_row,
        mi_col,
        mi_rows,
        mi_cols,
        core.order_hint_lsb.unwrap_or(0),
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

const fn compound_deblock_sub_pu_size(
    use_optflow: bool,
    use_refinemv: bool,
    luma_width: usize,
    luma_height: usize,
) -> Option<crate::filters::deblock::DeblockSubPuSize> {
    if use_optflow {
        let size = super::super::mc::optflow_unit_size(luma_width, luma_height);
        Some(crate::filters::deblock::DeblockSubPuSize::square(size))
    } else if use_refinemv {
        Some(crate::filters::deblock::DeblockSubPuSize::new(
            if luma_width < 16 { luma_width } else { 16 },
            if luma_height < 16 { luma_height } else { 16 },
        ))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn record_compound_temporal_motion<T: ReconSample>(
    motion_field: &mut TemporalMotionField,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    placed: &PlacedInterBlock,
    compound: super::super::compound::CompoundBlockSyntax,
    warp_params: [Option<[i64; 6]>; 2],
    motion_grid: Option<&mc::CompoundMotionGrid>,
    mi_row: usize,
    mi_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
) -> Result<()> {
    for y in (0..placed.luma_h).step_by(8) {
        for x in (0..placed.luma_w).step_by(8) {
            let mvs = if let Some(grid) = motion_grid {
                grid.temporal_mvs_at_luma_offset(x, y)?
            } else {
                let unit_mv = |params: Option<[i64; 6]>, block_mv: Mv| {
                    params.map_or(block_mv, |params| {
                        super::super::find_mv_stack::warp_sub_mv_at(
                            params,
                            mi_row,
                            mi_col,
                            mi_row + y / 4,
                            mi_col + x / 4,
                        )
                    })
                };
                [
                    unit_mv(warp_params[0], compound.mv0),
                    unit_mv(warp_params[1], compound.mv1),
                ]
            };
            let allowed = wedge_temporal_allowed_lists(
                placed.block.compound_blend,
                placed.luma_w,
                placed.luma_h,
                x,
                y,
            )?;
            let (ref_frame0, ref_frame1, mvs) = match allowed {
                [true, false] => (compound.ref_frame0, None, [mvs[0], Mv::ZERO]),
                [false, true] => (compound.ref_frame1, None, [mvs[1], Mv::ZERO]),
                _ => (compound.ref_frame0, Some(compound.ref_frame1), mvs),
            };
            record_temporal_motion_block(
                motion_field,
                reference,
                ref_frame_idx,
                mi_row + y / 4,
                mi_col + x / 4,
                (placed.luma_w - x).min(8).div_ceil(4),
                (placed.luma_h - y).min(8).div_ceil(4),
                mi_rows,
                mi_cols,
                current_order_hint,
                ref_frame0,
                ref_frame1,
                mvs[0],
                mvs[1],
                [None, None],
            );
        }
    }
    Ok(())
}

fn wedge_temporal_allowed_lists(
    blend: mc::CompoundBlend,
    luma_width: usize,
    luma_height: usize,
    x: usize,
    y: usize,
) -> Result<[bool; 2]> {
    let mc::CompoundBlend::Wedge { index, sign } = blend else {
        return Ok([true; 2]);
    };
    let mut dominant = [0usize; 2];
    for row in y..y + 8 {
        for col in x..x + 8 {
            let mask = wedge_mask_plane_sample(
                luma_width,
                luma_height,
                usize::from(index),
                sign,
                0,
                0,
                col,
                row,
            )?;
            dominant[0] += usize::from(mask > 60);
            dominant[1] += usize::from(mask < 4);
        }
    }
    Ok(if dominant[0] >= 60 {
        [true, false]
    } else if dominant[1] >= 60 {
        [false, true]
    } else {
        [true; 2]
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_skip_mode_inter_block<T: ReconSample>(
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
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    num_total_refs: usize,
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
    residual_quantizer_deltas_are_zero: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let current = compound_current_order_hint(core, tile_offset)?;
    let ref_order_hints = if num_total_refs > 1 {
        Some((
            compound_reference_order_hint(reference, ref_frame_idx, 0, tile_offset)?,
            compound_reference_order_hint(reference, ref_frame_idx, 1, tile_offset)?,
        ))
    } else {
        None
    };
    let default_pair = skip_mode_default_pair(current, ref_order_hints);
    let (ref_frame0, ref_frame1) = neighbour_ctx.skip_mode_ref_pair(default_pair);
    if ref_frame0 < 0
        || ref_frame1 < 0
        || ref_frame0 as usize >= num_total_refs
        || ref_frame1 as usize >= num_total_refs
    {
        return Err(compound_cap!(
            "skip_mode_reference_pair",
            tile_offset,
            "inter.skip_mode.reference_pair",
            SPEC_MODE_INFO
        ));
    }
    block_ctx.ref_frame0 = ref_frame0;
    block_ctx.ref_frame1 = Some(ref_frame1);
    let ref_mv_idx = read_skip_drl_idx(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let bank = ref_mv_bank
        .as_ref()
        .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2));
    let temporal = (ref_frame0 != ref_frame1)
        .then_some(temporal_context)
        .flatten();
    let candidate = find_compound_mv_stack_with_temporal(
        mv_grid,
        block_ctx,
        [Mv::ZERO; 2],
        bank,
        drl_reorder,
        temporal,
    )
    .candidate(ref_mv_idx);
    let compound = super::super::compound::CompoundBlockSyntax {
        ref_frame0,
        ref_frame1,
        y_mode: CompoundYMode::NearNear,
        mv0: candidate.mvs[0],
        mv1: candidate.mvs[1],
        use_optflow: false,
    };
    reconstruct_resolved_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        sequence,
        core,
        frontier,
        workspace,
        block_decoded,
        mv_grid,
        motion_field,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        intrabc_state,
        ref_frame_idx,
        reference,
        ref_mv_bank,
        ResolvedCompoundBlock {
            syntax: compound,
            blend: mc::CompoundBlend::average_with_implicit_mask(
                CompoundBlendToolConfig::from_sequence(sequence).implicit_mask,
            )
            .average_with_cwp_weight(candidate.cwp_weight),
            interp: ReconInterpolationFilter::EightTapSharp,
            use_amvd: false,
            precision: BlockPrecisionRecord::most_probable(frame_mv_precision(core, tile_offset)?),
            skip_mode: true,
            use_refinemv: false,
            refinemv_switchable: false,
            warp_params: [None, None],
        },
        skip,
        n4w,
        n4h,
        mi_row,
        mi_col,
        mi_rows,
        mi_cols,
        sb_h4,
        residual_quantizer_deltas_are_zero,
        residual_tool_policy,
        block_qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        tile_offset,
    )
}

/// AV2 § 5.9.22 `skip_mode_params` fallback reference pair.
fn skip_mode_default_pair(current: i32, order_hints: Option<(i32, i32)>) -> (i8, i8) {
    let Some((order_hint0, order_hint1)) = order_hints else {
        return (0, 0);
    };
    let distance = |reference| {
        if reference == RESTRICTED_ORDER_HINT {
            0
        } else {
            super::super::get_relative_dist(current, reference).abs()
        }
    };
    let second = i8::from((distance(order_hint0) - distance(order_hint1)).abs() <= 1);
    (0, second)
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
    if compound.y_mode == CompoundYMode::GlobalGlobal {
        return Ok(false);
    }
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

/// AV2 § 5.20.7.17 switchable `use_refinemv` read over
/// `TileUseRefinemvCdf[ctx]`, with the § 8.3.2 context
/// `1 + (YMode - NEAR_NEARMV) + 6 * use_optflow`, reduced by one for
/// optical-flow modes past `GLOBAL_GLOBALMV`.
fn read_compound_use_refinemv_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    use_optflow: bool,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let mode_delta = match y_mode {
        CompoundYMode::NearNear => 0usize,
        CompoundYMode::NearNew => 1,
        CompoundYMode::NewNear => 2,
        CompoundYMode::GlobalGlobal => 3,
        CompoundYMode::NewNew => 4,
        CompoundYMode::JointNew => 5,
    };
    let mut ctx = 1 + mode_delta + 6 * usize::from(use_optflow);
    if use_optflow && mode_delta > 3 {
        ctx -= 1;
    }
    let use_refinemv = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseRefinemv { ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(use_refinemv.get() != 0)
}

const fn compound_reads_second_drl(compound: super::super::compound::CompoundBlockSyntax) -> bool {
    !compound.use_optflow && compound.y_mode.has_second_drl()
}

fn select_near_near_candidates(
    compound: super::super::compound::CompoundBlockSyntax,
    paired_idx: usize,
    independent_indices: [usize; 2],
    paired: impl FnOnce(usize) -> [Mv; 2],
    independent: impl FnOnce([usize; 2]) -> [Mv; 2],
) -> [Mv; 2] {
    if compound_reads_second_drl(compound) {
        independent(independent_indices)
    } else {
        paired(paired_idx)
    }
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
    if !compound_refinemv_mode_allowed(core, compound, tile_offset)? {
        return Ok(false);
    }
    compound_refinemv_reference_allowed(core, reference, ref_frame_idx, compound, tile_offset)
}

const fn compound_refinemv_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2 && (n4w >= 4 || n4h >= 4)
}

fn compound_refinemv_mode_allowed(
    core: &FrameHeaderCore,
    compound: super::super::compound::CompoundBlockSyntax,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if compound.y_mode == CompoundYMode::GlobalGlobal {
        return Ok(false);
    }
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
    Ok(compound_refinemv_mode_allowed_for_type(
        compound,
        opfl_refine_type,
    ))
}

const fn compound_refinemv_mode_allowed_for_type(
    compound: super::super::compound::CompoundBlockSyntax,
    opfl_refine_type: u32,
) -> bool {
    !(matches!(compound.y_mode, CompoundYMode::GlobalGlobal)
        || opfl_refine_type == REFINE_SWITCHABLE
            && compound.y_mode.has_newmv()
            && !compound.use_optflow)
}

fn compound_refinemv_is_switchable(
    compound: super::super::compound::CompoundBlockSyntax,
    opfl_refine_type: u32,
) -> bool {
    compound.y_mode != CompoundYMode::NearNear
        && !(compound.y_mode == CompoundYMode::JointNew
            && compound.use_optflow
            && opfl_refine_type == REFINE_SWITCHABLE)
}

const fn compound_refinemv_active_after_blend(
    refinemv_default: bool,
    blend: mc::CompoundBlend,
) -> bool {
    refinemv_default && matches!(blend, mc::CompoundBlend::Average { .. })
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
    use_optflow: bool,
    joint_amvd: bool,
    switchable_refinemv_on: bool,
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
    use_optflow: bool,
    use_refinemv: bool,
    motion_simple: bool,
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
    if input.skip_mode
        || input.use_optflow
        || input.joint_amvd
        || input.switchable_refinemv_on
        || !tools.masked_enabled
        || thin
    {
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
    if !compound_cwp_signal_allowed(cwp_enabled, input) {
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

fn compound_cwp_signal_allowed(cwp_enabled: bool, input: CompoundCwpInput) -> bool {
    cwp_enabled
        && !input.skip_mode
        && !input.use_optflow
        && !input.use_refinemv
        && input.motion_simple
        && compound_cwp_mode_allowed(input.y_mode, input.jmvd_scale_mode)
        && matches!(input.blend, mc::CompoundBlend::Average { .. })
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
mod tests;
