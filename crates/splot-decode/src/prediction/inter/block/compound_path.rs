// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_recon::wedge_mask_plane_sample;

use super::super::CompoundOrderHint;
use super::super::compound::{
    CompoundBlockSyntax, CompoundParseInput, CompoundYMode, read_compound_mode_syntax,
    read_compound_reference_pair,
};
use super::super::find_mv_stack::TemporalMotionBlock;
use super::super::read_mv::apply_inter_mvd_sign_pair;
use super::prediction::PlacedInterGeometry;
use super::temporal::temporal_motion_block;
use super::warp::mvd_sign_derivation_block_scope_allowed;
use super::*;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const REFINE_SWITCHABLE: u32 = 1;
const REFINE_ALL: u32 = 2;
const MV_PROJECTION_DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];
pub(super) fn read_reference_mode(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    neighbour_ctx: &BlockNeighbourContext,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let ctx = neighbour_ctx.comp_mode_ctx(
        ref_frame_idx,
        ref_order_hint,
        CompoundOrderHint::current(current_order_hint),
    );
    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::CompMode { ctx }, symbols)
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    match mode {
        0 => Ok(false),
        _ => Ok(true),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_compound_inter_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    tip_ref_pair: Option<(i8, i8)>,
    block_ctx: &mut MvBlockContext,
    neighbour_ctx: &BlockNeighbourContext,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    num_total_refs: usize,
    num_same_ref_compound: u8,
    skip: u8,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    temporal_first_frame: bool,
    enable_adaptive_mvd: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, ParsedLeaf)> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let ref_contexts = compound_ref_contexts(neighbour_ctx, num_total_refs)?;
    let ref_distance_nonnegative = compound_ref_distance_signs(
        ref_frame_idx,
        reference,
        core.display_order_hint().unwrap_or(0),
        num_total_refs,
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
        core.display_order_hint().unwrap_or(0),
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
    let frame_mv_config = inter_mv_read_config(core)?;
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
    let joint = (compound.y_mode == CompoundYMode::JointNew)
        .then(|| {
            compound_joint_mv_projection(
                core,
                reference,
                ref_frame_idx,
                compound.ref_frame0,
                compound.ref_frame1,
            )
        })
        .transpose()?;
    let mvd = read_compound_mvd_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        CompoundMvdInput {
            y_mode: compound.y_mode,
            use_amvd,
            ref_mv_idx,
            precision,
            frame_mv_config,
            motion_mode: if local_warp {
                MotionMode::LocalWarp
            } else {
                MotionMode::Simple
            },
        },
        tile_offset,
    )?;
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
        compound.use_optflow = true;
    }
    let interp = resolve_compound_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        compound.use_optflow || refinemv_signalled,
        compound_needs_interp_filter(n4w, n4h, compound.y_mode, local_warp),
        neighbour_ctx.interp_filter_ctx(compound.ref_frame0, true),
        tile_offset,
    )?;
    let force_integer_mv = effective_force_integer_mv(core);
    let temporal_allowed = compound.ref_frame0 != compound.ref_frame1;
    let temporal_first = |ref_frame| {
        temporal_first_frame
            && super::block_ref_within_temporal_distance(
                reference,
                ref_frame_idx,
                core.display_order_hint().unwrap_or(0),
                ref_frame,
            )
    };
    finish_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        residual_scratch,
        residual_blocks,
        sequence,
        core,
        frontier,
        mv_grid,
        frame_mv_config.precision(),
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        intrabc_state,
        ref_frame_idx,
        reference,
        ParsedCompoundBlock {
            block_ctx: *block_ctx,
            motion: CompoundMotionSyntax {
                y_mode: compound.y_mode,
                ref_frame1: compound.ref_frame1,
                skip_mode: false,
                use_optflow: compound.use_optflow,
                local_warp,
                global_warp: if compound.y_mode == CompoundYMode::GlobalGlobal {
                    [
                        global_motion_warp(core, compound.ref_frame0, force_integer_mv, n4w, n4h),
                        global_motion_warp(core, compound.ref_frame1, force_integer_mv, n4w, n4h),
                    ]
                } else {
                    [None, None]
                },
                ref_mv_idx,
                independent_idx: compound_reads_second_drl(compound)
                    .then_some([ref_mv_idx0, ref_mv_idx1]),
                mvd,
                joint,
                jmvd_scale_mode,
                temporal_allowed,
                temporal_first: [
                    temporal_first(compound.ref_frame0),
                    temporal_first(compound.ref_frame1),
                ],
                use_refinemv,
                refinemv_switchable,
            },
            blend: compound_blend,
            interp,
            use_amvd,
            precision,
        },
        skip,
        mi_rows,
        mi_cols,
        residual_tool_policy,
        block_qindex,
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

fn compound_needs_interp_filter(
    n4w: usize,
    n4h: usize,
    y_mode: CompoundYMode,
    local_warp: bool,
) -> bool {
    !local_warp && (n4w < 2 || n4h < 2 || y_mode != CompoundYMode::GlobalGlobal)
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
/// once per reference list. A signalled list can gather no samples because the
/// § 5.20.7.14 `WarpSampleFound` scan is wider than the § 7.12.3.1 gathering.
/// AVM marks that list's warp model invalid, which selects the translational
/// prediction equivalent to the spec's det==0 identity model.
#[allow(clippy::too_many_arguments)]
pub(super) fn compound_local_warp_models(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    ref_frame1: i8,
    mv0: Mv,
    mv1: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
) -> Result<[Option<[i32; 6]>; 2]> {
    let model0 = compound_ref_warp_model(
        mv_grid,
        block_ctx,
        block_ctx.ref_frame0,
        mv0,
        mi_row,
        mi_col,
        n4w,
        n4h,
    )?;
    let model1 = compound_ref_warp_model(
        mv_grid, block_ctx, ref_frame1, mv1, mi_row, mi_col, n4w, n4h,
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
) -> Result<Option<[i32; 6]>> {
    let samples = super::super::find_mv_stack::find_warp_samples(mv_grid, block_ctx, target_ref);
    if samples.is_empty() {
        return Ok(None);
    }
    Ok(Some(local_warp_estimation(
        &samples, mv, mi_row, mi_col, n4w, n4h,
    )?))
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
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    Ok(use_local_warp != 0)
}

/// Parsed § 5.20.7.12 compound syntax handed to the § 7.12 resolution step.
#[derive(Clone, Copy)]
pub(super) struct ParsedCompoundBlock {
    pub(super) block_ctx: MvBlockContext,
    pub(super) motion: CompoundMotionSyntax,
    pub(super) blend: mc::CompoundBlend,
    pub(super) interp: ReconInterpolationFilter,
    pub(super) use_amvd: bool,
    pub(super) precision: BlockPrecisionRecord,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_compound_inter_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    frame_precision: u8,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    parsed: ParsedCompoundBlock,
    skip: u8,
    mi_rows: usize,
    mi_cols: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, ParsedLeaf)> {
    let ParsedCompoundBlock {
        block_ctx,
        motion,
        blend,
        interp,
        use_amvd,
        precision,
    } = parsed;
    let (mi_row, mi_col, n4w, n4h) = (
        block_ctx.mi_row,
        block_ctx.mi_col,
        block_ctx.bw4,
        block_ctx.bh4,
    );
    let residual = if skip == 0 {
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            residual_scratch,
            residual_blocks,
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
        reset_inter_skip_coeff_contexts(
            coeff_ctx,
            frontier,
            n4w,
            n4h,
            super::block_chroma_subsampling(sequence.general.chroma_format_idc),
            tile_offset,
        )?;
        None
    };
    let sub_pu_size = compound_deblock_sub_pu_size(
        motion.use_optflow,
        motion.use_refinemv,
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
        residual_blocks,
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
    )?;
    let reference_pair = CompoundBlockSyntax {
        y_mode: motion.y_mode,
        use_optflow: motion.use_optflow,
        ref_frame0: block_ctx.ref_frame0,
        ref_frame1: motion.ref_frame1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };
    let optflow_distances = if motion.use_optflow {
        compound_sized_reference_distances(
            core,
            reference,
            ref_frame_idx,
            reference_pair,
            CompoundReferencePath::Opfl,
            tile_offset,
        )?
        .map(|(dist0, dist1)| [dist0, dist1])
    } else {
        None
    };
    intrabc_state.record_block(
        frontier.r,
        frontier.c,
        n4w,
        n4h,
        IntrabcBlockPrelude::from_use_skip(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: skip == 1,
        })
        .mark_inter(),
    )?;
    let syntax = InterBlockSyntax {
        block_ctx,
        motion: InterMotionSyntax::Compound(motion),
        interp,
        precision,
        skip: skip == 1,
        use_amvd,
        tip_size_16x16: false,
        blend,
        bawp: BawpSyntax::default(),
        interintra: None,
        optflow_distances,
        residual,
    };
    mv_grid.record_flags(mi_row, mi_col, n4w, n4h, syntax.flag_syntax());
    Ok((
        non_intra_leaf_mode(frontier),
        pending_inter_leaf(
            syntax,
            PlacedInterGeometry {
                interintra_chroma: false,
                ..placed_geometry
            },
            block_qindex,
            frame_precision,
        ),
    ))
}

/// Input to the § 5.20.7.7 compound motion-vector difference reads.
#[derive(Clone, Copy)]
struct CompoundMvdInput {
    y_mode: CompoundYMode,
    use_amvd: bool,
    ref_mv_idx: usize,
    precision: BlockPrecisionRecord,
    frame_mv_config: MvReadConfig,
    motion_mode: MotionMode,
}

/// AV2 § 5.20.7.7 compound motion-vector differences: one per NEWMV list, with
/// JOINT_NEWMV reading a single difference the § 7.12.2.4 projection reuses.
fn read_compound_mvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    input: CompoundMvdInput,
    tile_offset: ByteOffset,
) -> Result<[Mv; 2]> {
    if !input.y_mode.has_newmv() {
        return Ok([Mv::ZERO; 2]);
    }
    let config = MvReadConfig::inter(input.precision.mv_precision);
    let threshold = input.y_mode.mvd_sign_derivation_threshold();
    let derive_sign =
        mvd_sign_derivation_block_scope_allowed(input.motion_mode, false, Some(input.ref_mv_idx))
            && inter_mvd_sign_derivation_allowed(
                sequence,
                core,
                SINGLE_MODE_NEWMV,
                input.use_amvd,
                input.frame_mv_config,
                config,
            );
    match input.y_mode {
        CompoundYMode::NearNew | CompoundYMode::NewNear => {
            let magnitude =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let diff =
                apply_inter_mvd_signs(magnitude, symbols, tile_offset, config, false, threshold)?;
            let mut mvd = [Mv::ZERO; 2];
            mvd[usize::from(input.y_mode == CompoundYMode::NearNew)] = diff;
            Ok(mvd)
        }
        CompoundYMode::JointNew => {
            let magnitude =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let diff = apply_inter_mvd_signs(
                magnitude,
                symbols,
                tile_offset,
                config,
                derive_sign,
                threshold,
            )?;
            Ok([diff, Mv::ZERO])
        }
        CompoundYMode::NewNew => {
            let magnitude0 =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let magnitude1 =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let (diff0, diff1) = apply_inter_mvd_sign_pair(
                magnitude0,
                magnitude1,
                symbols,
                tile_offset,
                config,
                derive_sign,
                threshold,
            )?;
            Ok([diff0, diff1])
        }
        CompoundYMode::NearNear | CompoundYMode::GlobalGlobal => Ok([Mv::ZERO; 2]),
    }
}

fn read_compound_mvd_magnitude(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    use_amvd: bool,
    config: MvReadConfig,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    if use_amvd {
        read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)
    } else {
        read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)
    }
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
pub(super) fn append_compound_temporal_motion<T: ReconSample>(
    records: &mut Vec<TemporalMotionBlock>,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    placed: &PlacedInterBlock,
    compound: super::super::compound::CompoundBlockSyntax,
    warp_params: [Option<[i32; 6]>; 2],
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
                let unit_mv = |params: Option<[i32; 6]>, block_mv: Mv| {
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
            records.push(temporal_motion_block(
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
            ));
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
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    block_ctx: &mut MvBlockContext,
    neighbour_ctx: &BlockNeighbourContext,
    tip_ref_pair: Option<(i8, i8)>,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    intrabc_state: &mut TileIntrabcPreludeState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    num_total_refs: usize,
    skip: u8,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, ParsedLeaf)> {
    let current = compound_current_order_hint(core);
    let ref_order_hints = if num_total_refs > 1 {
        Some((
            compound_reference_order_hint(reference, ref_frame_idx, 0)?,
            compound_reference_order_hint(reference, ref_frame_idx, 1)?,
        ))
    } else {
        None
    };
    let default_pair = skip_mode_default_pair(current, ref_order_hints);
    let (ref_frame0, ref_frame1) = checked_skip_mode_reference_pair(
        neighbour_ctx.skip_mode_ref_pair(default_pair, tip_ref_pair),
        num_total_refs,
        tile_offset,
    )?;
    block_ctx.ref_frame0 = ref_frame0;
    block_ctx.ref_frame1 = Some(ref_frame1);
    let ref_mv_idx = read_skip_drl_idx(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let precision = frame_mv_precision(core)?;
    finish_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        residual_scratch,
        residual_blocks,
        sequence,
        core,
        frontier,
        mv_grid,
        precision,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        intrabc_state,
        ref_frame_idx,
        reference,
        ParsedCompoundBlock {
            block_ctx: *block_ctx,
            motion: CompoundMotionSyntax {
                y_mode: CompoundYMode::NearNear,
                ref_frame1,
                skip_mode: true,
                use_optflow: false,
                local_warp: false,
                global_warp: [None, None],
                ref_mv_idx,
                independent_idx: None,
                mvd: [Mv::ZERO; 2],
                joint: None,
                jmvd_scale_mode: 0,
                temporal_allowed: true,
                temporal_first: [false; 2],
                use_refinemv: false,
                refinemv_switchable: false,
            },
            blend: mc::CompoundBlend::average_with_implicit_mask(
                CompoundBlendToolConfig::from_sequence(sequence).implicit_mask,
            ),
            interp: ReconInterpolationFilter::EightTapSharp,
            use_amvd: false,
            precision: BlockPrecisionRecord::most_probable(precision),
        },
        skip,
        mi_rows,
        mi_cols,
        residual_tool_policy,
        block_qindex,
        tile_offset,
    )
}

/// AV2 § 5.9.22 `skip_mode_params` fallback reference pair.
fn skip_mode_default_pair(
    current: CompoundOrderHint,
    order_hints: Option<(CompoundOrderHint, CompoundOrderHint)>,
) -> (i8, i8) {
    let Some((order_hint0, order_hint1)) = order_hints else {
        return (0, 0);
    };
    let distance = |reference: CompoundOrderHint| {
        if reference.is_restricted() {
            0
        } else {
            current.relative_dist(reference).abs()
        }
    };
    let second = i8::from((distance(order_hint0) - distance(order_hint1)).abs() <= 1);
    (0, second)
}

fn checked_skip_mode_reference_pair(
    pair: (i8, i8),
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<(i8, i8)> {
    for reference in [pair.0, pair.1] {
        if reference < 0 || reference as usize >= num_total_refs {
            return Err(crate::pipeline::malformed_tile_payload(
                tile_offset,
                SPEC_MODE_INFO,
                DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                    index: reference,
                    list_len: num_total_refs,
                },
            ));
        }
    }
    Ok(pair)
}

fn compound_switchable_opfl_reachable<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
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
        .map_err(|error| symbol_read_error(error, tile_offset))?;
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
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    Ok(use_refinemv.get() != 0)
}

const fn compound_reads_second_drl(compound: super::super::compound::CompoundBlockSyntax) -> bool {
    !compound.use_optflow && compound.y_mode.has_second_drl()
}

pub(super) fn select_near_near_candidates(
    independent_indices: Option<[usize; 2]>,
    paired_idx: usize,
    paired: impl FnOnce(usize) -> [Mv; 2],
    independent: impl FnOnce([usize; 2]) -> [Mv; 2],
) -> [Mv; 2] {
    match independent_indices {
        Some(indices) => independent(indices),
        None => paired(paired_idx),
    }
}

#[allow(clippy::too_many_arguments)]
fn compound_all_opfl_reachable<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    blend: mc::CompoundBlend,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if compound_opfl_refine_type(core, tile_offset)? != REFINE_ALL
        || !compound_all_opfl_block_allowed(compound, n4w, n4h, blend)
    {
        return Ok(false);
    }
    compound_opfl_reference_allowed(core, reference, ref_frame_idx, compound, tile_offset)
}

fn compound_all_opfl_block_allowed(
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    blend: mc::CompoundBlend,
) -> bool {
    compound_opfl_block_size_allowed(n4w, n4h)
        && compound.y_mode != CompoundYMode::GlobalGlobal
        && matches!(blend, mc::CompoundBlend::Average { .. })
        && blend.cwp_weight() == mc::CWP_EQUAL
}

fn compound_opfl_refine_type(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<u32> {
    core.inter
        .as_ref()
        .and_then(|inter| inter.opfl_refine_type)
        .ok_or(inter_internal!(
            "compound_missing_opfl_refine_type",
            tile_offset
        ))
}

const fn compound_opfl_block_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2
}

fn compound_opfl_reference_allowed<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
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
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some(seq_inter) = sequence.inter.as_ref() else {
        return Err(inter_internal!(
            "compound_refinemv_missing_sequence_inter",
            tile_offset
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
        .ok_or(inter_internal!(
            "compound_refinemv_missing_opfl_refine_type",
            tile_offset
        ))?;
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
    reference: &InterReferenceState<T>,
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
    reference: &InterReferenceState<T>,
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
            CompoundReferencePath::Opfl => {
                inter_internal!("compound_opfl_missing_frame_size", tile_offset)
            }
            CompoundReferencePath::RefineMv => {
                inter_internal!("compound_refinemv_missing_frame_size", tile_offset)
            }
        });
    };
    let ref0 = compound_reference_facts(reference, ref_frame_idx, compound.ref_frame0)?;
    let ref1 = compound_reference_facts(reference, ref_frame_idx, compound.ref_frame1)?;
    if ref0.order_hint.is_restricted()
        || ref1.order_hint.is_restricted()
        || ref0.width != frame_size.width
        || ref0.height != frame_size.height
        || ref1.width != frame_size.width
        || ref1.height != frame_size.height
    {
        return Ok(None);
    }
    let current = compound_current_order_hint(core);
    let d0 = current.relative_dist(ref0.order_hint);
    let d1 = current.relative_dist(ref1.order_hint);
    Ok(Some((d0, d1)))
}

fn compound_current_order_hint(core: &FrameHeaderCore) -> CompoundOrderHint {
    CompoundOrderHint::current(core.display_order_hint().unwrap_or(0))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompoundReferenceFacts {
    order_hint: CompoundOrderHint,
    width: u32,
    height: u32,
}

fn compound_reference_facts<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
) -> Result<CompoundReferenceFacts> {
    let slot = super::super::block_reference_slot(ref_frame_idx, ref_frame)?;
    let slot = usize::try_from(slot).unwrap_or(usize::MAX);
    let order_hint = reference.ref_order_hint.get(slot).copied().ok_or(
        crate::DecodeReferenceStateError::SlotOutOfRange {
            slot,
            slot_count: reference.ref_order_hint.len(),
        },
    )?;
    let order_hint = CompoundOrderHint::reference(order_hint);
    let width = *reference.ref_frame_width.get(slot).ok_or(
        crate::DecodeReferenceStateError::SlotOutOfRange {
            slot,
            slot_count: reference.ref_frame_width.len(),
        },
    )?;
    let height = *reference.ref_frame_height.get(slot).ok_or(
        crate::DecodeReferenceStateError::SlotOutOfRange {
            slot,
            slot_count: reference.ref_frame_height.len(),
        },
    )?;
    Ok(CompoundReferenceFacts {
        order_hint,
        width,
        height,
    })
}

fn compound_ref_contexts(
    neighbour_ctx: &BlockNeighbourContext,
    num_total_refs: usize,
) -> Result<[usize; 7]> {
    let mut contexts = [0usize; 7];
    for (ref_idx, ctx) in contexts.iter_mut().take(num_total_refs).enumerate() {
        *ctx = neighbour_ctx
            .single_ref_ctx(ref_idx, num_total_refs)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterReferenceMap)?;
    }
    Ok(contexts)
}

fn compound_ref_distance_signs<T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    current_order_hint: u32,
    num_total_refs: usize,
) -> Result<[bool; 7]> {
    let mut signs = [true; 7];
    let current_order_hint = CompoundOrderHint::current(current_order_hint);
    for (ref_idx, sign) in signs.iter_mut().take(num_total_refs).enumerate() {
        let slot = *ref_frame_idx
            .get(ref_idx)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterReferenceMap)?;
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        let ref_order_hint = reference
            .ref_order_hint
            .get(slot)
            .copied()
            .map(CompoundOrderHint::reference)
            .ok_or(crate::DecodeReferenceStateError::SlotOutOfRange {
                slot,
                slot_count: reference.ref_order_hint.len(),
            })?;
        *sign = ref_order_hint.frame_distance_from(current_order_hint) >= 0;
    }
    Ok(signs)
}

#[allow(clippy::too_many_arguments)]
fn compound_group_idx_context<T: ReconSample>(
    neighbour_ctx: &BlockNeighbourContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    core: &FrameHeaderCore,
    ref_frame0: i8,
    ref_frame1: i8,
    num_total_refs: usize,
) -> Result<usize> {
    let current_order_hint = compound_current_order_hint(core);
    let ref0_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame0)?;
    let ref1_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame1)?;
    let equal_ref_distance = current_order_hint.relative_dist(ref0_order_hint).abs()
        == ref1_order_hint.relative_dist(current_order_hint).abs();
    let furthest_future_ref =
        compound_furthest_future_ref(reference, ref_frame_idx, current_order_hint, num_total_refs)?;
    Ok(neighbour_ctx.compound_group_idx_ctx(equal_ref_distance, furthest_future_ref))
}

fn compound_furthest_future_ref<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    current_order_hint: CompoundOrderHint,
    num_total_refs: usize,
) -> Result<Option<i8>> {
    let mut best = None;
    for ref_idx in 0..num_total_refs {
        let ref_order_hint =
            compound_reference_order_hint(reference, ref_frame_idx, ref_idx as i8)?;
        let CompoundOrderHint::Value(order_hint) = ref_order_hint else {
            continue;
        };
        let distance = ref_order_hint.relative_dist(current_order_hint);
        if distance <= 0 {
            continue;
        }
        if best.is_none_or(|(best_order_hint, _)| order_hint > best_order_hint) {
            best = Some((order_hint, ref_idx as i8));
        }
    }
    Ok(best.map(|(_, ref_idx)| ref_idx))
}

fn compound_reference_order_hint<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
) -> Result<CompoundOrderHint> {
    let slot = usize::try_from(super::super::block_reference_slot(
        ref_frame_idx,
        ref_frame,
    )?)
    .unwrap_or(usize::MAX);
    reference
        .ref_order_hint
        .get(slot)
        .copied()
        .ok_or_else(|| {
            crate::DecodeReferenceStateError::SlotOutOfRange {
                slot,
                slot_count: reference.ref_order_hint.len(),
            }
            .into()
        })
        .map(CompoundOrderHint::reference)
}

fn compound_joint_mv_projection<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame0: i8,
    ref_frame1: i8,
) -> Result<CompoundJointMvProjection> {
    let current = compound_current_order_hint(core);
    let ref0_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame0)?;
    let ref1_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame1)?;
    Ok(compound_joint_mv_projection_from_order_hints(
        current,
        ref0_order_hint,
        ref1_order_hint,
    ))
}

fn compound_joint_mv_projection_from_order_hints(
    current: CompoundOrderHint,
    ref0_order_hint: CompoundOrderHint,
    ref1_order_hint: CompoundOrderHint,
) -> CompoundJointMvProjection {
    let rel0 = ref0_order_hint.relative_dist(current);
    let rel1 = ref1_order_hint.relative_dist(current);
    let mut first_dist = rel0.abs();
    let mut second_dist = rel1.abs();
    let base_list = usize::from(
        first_dist < second_dist
            || (!ref0_order_hint.is_restricted() && ref1_order_hint.is_restricted()),
    );
    if base_list == 1 {
        core::mem::swap(&mut first_dist, &mut second_dist);
    }
    let same_side = compound_references_same_side(current, ref0_order_hint, ref1_order_hint);
    if !same_side {
        second_dist = -second_dist;
    }
    CompoundJointMvProjection {
        base_list,
        first_dist,
        second_dist,
    }
}

pub(super) fn project_joint_mvd(diff: Mv, num: i32, den: i32) -> Mv {
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
    rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(super) fn scale_joint_projected_mvd(
    mut projected: Mv,
    jmvd_scale_mode: u8,
    use_amvd: bool,
) -> Mv {
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

pub(super) fn add_mv_clamped(pred: Mv, diff: Mv) -> Mv {
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
        .map_err(|error| symbol_read_error(error, tile_offset))?;
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
        .map_err(|error| symbol_read_error(error, tile_offset))
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
    reference: &'a InterReferenceState<T>,
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
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    if comp_group_idx == 0 {
        return Ok(average_blend);
    }
    let compound_type = if wedge_bits(input.block_size_index) == 0 {
        MaskedCompoundType::DiffWeighted
    } else {
        match cdfs
            .read_block_symbol_trace(TileCdfSelector::CompoundType, symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?
            .get()
        {
            0 => MaskedCompoundType::Wedge,
            _ => MaskedCompoundType::DiffWeighted,
        }
    };
    match compound_type {
        MaskedCompoundType::DiffWeighted => {
            let mask_type = symbols.read_literal(1).map_err(|error| {
                symbol_read_error(BlockSymbolTraceReadError::Symbol(error), tile_offset)
            })? != 0;
            Ok(mc::CompoundBlend::DiffWeighted { inverse: mask_type })
        }
        MaskedCompoundType::Wedge => {
            let index = read_wedge_mode_syntax(cdfs, symbols, tile_offset)?;
            let sign = symbols.read_bool().map_err(|error| {
                symbol_read_error(BlockSymbolTraceReadError::Symbol(error), tile_offset)
            })?;
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
            .map_err(|error| symbol_read_error(error, tile_offset))?
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
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame0: i8,
    ref_frame1: i8,
) -> Result<bool> {
    let current = compound_current_order_hint(core);
    let ref0_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame0)?;
    let ref1_order_hint = compound_reference_order_hint(reference, ref_frame_idx, ref_frame1)?;
    Ok(compound_references_same_side(
        current,
        ref0_order_hint,
        ref1_order_hint,
    ))
}

fn compound_references_same_side(
    current: CompoundOrderHint,
    ref0_order_hint: CompoundOrderHint,
    ref1_order_hint: CompoundOrderHint,
) -> bool {
    let d0 = ref0_order_hint.frame_distance_from(current);
    let d1 = ref1_order_hint.frame_distance_from(current);
    (d0 < 0 && d1 < 0) || (d0 > 0 && d1 > 0)
}

#[cfg(test)]
mod tests;
