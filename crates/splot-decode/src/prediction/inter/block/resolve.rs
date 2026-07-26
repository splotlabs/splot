// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7 inter syntax records and their § 7.12 motion resolution.
//!
//! [`InterBlockSyntax`] holds everything the entropy pass reads for one inter
//! leaf, with no reference MV stack consulted. [`resolve_inter_block`] turns
//! that record into motion vectors and warp models: it builds the § 7.12
//! stack, derives global-motion and warp models, publishes the neighbour
//! motion plane and updates the reference MV and warp parameter banks. The
//! flag plane is published by the parse side, so leaf order is what keeps the
//! two halves equivalent to the fused walk while they run back-to-back.

use super::super::compound::CompoundYMode;
use super::super::find_mv_stack::{
    CompoundMvCandidate, MvStack, NeighbourFlagSyntax, NeighbourMotionValues, OrderHintMvContext,
    RefMvBank, WarpParamBank, compound_motion_mode, find_warp_samples, reduce_warp_model,
};
use super::compound_path::{
    add_mv_clamped, compound_local_warp_models, project_joint_mvd, scale_joint_projected_mvd,
    select_near_near_candidates,
};
use super::warp::{apply_warp_delta, extend_warp_estimation, set_warp_translation};
#[allow(clippy::wildcard_imports)]
use super::*;

/// Per-leaf § 5.20.7 inter syntax, before any § 7.12 candidate is resolved.
pub(super) struct InterBlockSyntax {
    pub(super) block_ctx: MvBlockContext,
    pub(super) motion: InterMotionSyntax,
    pub(super) interp: ReconInterpolationFilter,
    pub(super) precision: BlockPrecisionRecord,
    pub(super) skip: bool,
    pub(super) use_amvd: bool,
    pub(super) tip_size_16x16: bool,
    pub(super) blend: mc::CompoundBlend,
    pub(super) bawp: BawpSyntax,
    pub(super) interintra: Option<InterIntraPrediction>,
    pub(super) optflow_distances: Option<[i32; 2]>,
    pub(super) residual: Option<InterResidual>,
}

/// Mode-dependent half of [`InterBlockSyntax`].
pub(super) enum InterMotionSyntax {
    Single(SingleMotionSyntax),
    Warp(WarpMotionSyntax),
    Compound(CompoundMotionSyntax),
}

/// § 5.20.7.11 single-reference and § 5.20.7.16 TIP syntax.
pub(super) struct SingleMotionSyntax {
    pub(super) mode: u8,
    pub(super) tip_ref: bool,
    pub(super) ref_mv_idx: usize,
    pub(super) mvd: Mv,
    pub(super) use_temporal_first: bool,
    /// § 7.13.3.22 GLOBALMV warp model, absent for every other mode.
    pub(super) global_warp: Option<[i32; 6]>,
}

/// § 5.20.7.13 warp-mode syntax.
pub(super) struct WarpMotionSyntax {
    pub(super) source: WarpModelSource,
    pub(super) ref_mv_idx: usize,
    pub(super) ref_warp_idx: usize,
    pub(super) mvd: Option<Mv>,
    pub(super) extend_delta: Option<(i32, i32)>,
    pub(super) derive_wrl: bool,
    pub(super) use_temporal_first: bool,
}

/// Which § 7.13.3 derivation builds the block's warp model.
pub(super) enum WarpModelSource {
    /// § 7.13.3.23 least squares over the § 7.12.3 warp samples.
    LocalSamples,
    /// § 7.13.3.24 extension of the DRL candidate's neighbour model.
    Extended,
    /// § 5.20.7.13 warp-delta parameters on top of the warp candidate.
    Delta(WarpDeltaSyntax),
    /// § 5.20.7.13 WARPMV: the warp candidate supplies model and base MV.
    Candidate,
}

impl WarpModelSource {
    const fn motion_mode(&self) -> MotionMode {
        match self {
            Self::LocalSamples => MotionMode::LocalWarp,
            Self::Extended => MotionMode::ExtendWarp,
            Self::Delta(_) | Self::Candidate => MotionMode::DeltaWarp,
        }
    }
}

/// § 5.20.7.13 `warp_delta` parameter deltas for model indices 2 through 5.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WarpDeltaSyntax {
    pub(super) deltas: Option<[i32; 4]>,
    pub(super) six_param: bool,
}

/// § 7.12.2.4 joint-MVD projection distances and base list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundJointMvProjection {
    pub(super) base_list: usize,
    pub(super) first_dist: i32,
    pub(super) second_dist: i32,
}

/// § 5.20.7.12 compound and § 5.20.7.10 skip-mode syntax.
#[derive(Clone, Copy)]
pub(super) struct CompoundMotionSyntax {
    pub(super) y_mode: CompoundYMode,
    pub(super) ref_frame1: i8,
    pub(super) skip_mode: bool,
    pub(super) use_optflow: bool,
    pub(super) local_warp: bool,
    /// § 7.13.3.22 GLOBAL_GLOBALMV warp models, absent for every other mode.
    pub(super) global_warp: [Option<[i32; 6]>; 2],
    pub(super) ref_mv_idx: usize,
    pub(super) independent_idx: Option<[usize; 2]>,
    pub(super) mvd: [Mv; 2],
    pub(super) joint: Option<CompoundJointMvProjection>,
    pub(super) jmvd_scale_mode: u8,
    /// § 7.12.2 temporal candidates are gated off when both lists share a
    /// reference, for the paired stack only.
    pub(super) temporal_allowed: bool,
    pub(super) temporal_first: [bool; 2],
}

/// § 7.12 resolution output consumed by the reconstruction command.
pub(super) struct ResolvedInterBlock {
    pub(super) mv: [Mv; 2],
    pub(super) warp_params: [Option<[i32; 6]>; 2],
    pub(super) blend: mc::CompoundBlend,
}

/// Mutable state the § 7.12 resolution chain owns, plus the frame-level facts
/// its derivations read.
pub(super) struct MvResolutionState<'a> {
    pub(super) grid: &'a mut NeighbourMvGrid,
    pub(super) ref_mv_bank: &'a mut Option<RefMvBank>,
    pub(super) warp_param_bank: &'a mut WarpParamBank,
    pub(super) core: &'a FrameHeaderCore,
    pub(super) temporal: Option<&'a TemporalMvContext>,
    pub(super) order_hints: OrderHintMvContext<'a>,
    pub(super) drl_reorder: DrlReorder,
    pub(super) max_drl_bits_minus_1: u32,
    pub(super) frame_precision: u8,
    pub(super) tile_offset: ByteOffset,
}

impl MvResolutionState<'_> {
    fn bank(&self) -> Option<(&RefMvBank, usize)> {
        self.ref_mv_bank
            .as_ref()
            .map(|bank| (bank, self.max_drl_bits_minus_1 as usize + 2))
    }
}

impl InterBlockSyntax {
    /// Flag-plane record for this leaf. Symbol decode of later leaves reads
    /// only this half, so the parse side publishes it before resolution runs.
    pub(super) fn flag_syntax(&self) -> NeighbourFlagSyntax {
        let (ref_frame1, newmv, skip_mode, motion_mode) = match &self.motion {
            InterMotionSyntax::Single(single) => (
                None,
                [single.mode == SINGLE_MODE_NEWMV, false],
                false,
                MotionMode::Simple,
            ),
            InterMotionSyntax::Warp(warp) => {
                (None, [false, false], false, warp.source.motion_mode())
            }
            InterMotionSyntax::Compound(compound) => (
                Some(compound.ref_frame1),
                [
                    compound.y_mode.list0_is_newmv(),
                    compound.y_mode.list1_is_newmv(),
                ],
                compound.skip_mode,
                compound_motion_mode(compound.has_warp_model()),
            ),
        };
        NeighbourFlagSyntax {
            is_inter: true,
            ref_frame0: self.block_ctx.ref_frame0,
            ref_frame1,
            newmv,
            skip: self.skip,
            skip_mode,
            use_amvd: self.use_amvd,
            masked_compound: !matches!(self.blend, mc::CompoundBlend::Average { .. }),
            tip_size_16x16: self.tip_size_16x16,
            interp_filter: interp_filter_symbol(self.interp),
            motion_mode,
            precision: self.precision,
        }
    }
}

impl CompoundMotionSyntax {
    const fn has_warp_model(&self) -> bool {
        self.local_warp || self.global_warp[0].is_some() || self.global_warp[1].is_some()
    }

    const fn is_global(&self) -> bool {
        matches!(self.y_mode, CompoundYMode::GlobalGlobal)
    }
}

/// Resolves one parsed inter leaf: § 7.12 stack, warp models, motion-plane
/// publication and bank maintenance, in the fused walk's order.
pub(super) fn resolve_inter_block(
    syntax: &InterBlockSyntax,
    state: &mut MvResolutionState<'_>,
) -> Result<ResolvedInterBlock> {
    let block = &syntax.block_ctx;
    let resolved = match &syntax.motion {
        InterMotionSyntax::Single(single) => resolve_single(syntax, single, state),
        InterMotionSyntax::Warp(warp) => resolve_warp(syntax, warp, state)?,
        InterMotionSyntax::Compound(compound) => resolve_compound(syntax, compound, state)?,
    };
    state.grid.record_motion(
        block.mi_row,
        block.mi_col,
        block.bw4,
        block.bh4,
        NeighbourMotionValues {
            mv: resolved.mv,
            cwp_weight: resolved.blend.cwp_weight(),
            stored_warp: resolved.warp_params[0],
            splat_warp: match &syntax.motion {
                InterMotionSyntax::Single(_) => [None, None],
                InterMotionSyntax::Warp(_) | InterMotionSyntax::Compound(_) => resolved.warp_params,
            },
        },
    );
    if let Some(bank) = state.ref_mv_bank.as_mut() {
        bank.update_for_block(
            block.ref_frame0,
            block.ref_frame1,
            resolved.mv[0],
            block.ref_frame1.map(|_| resolved.mv[1]),
            resolved.blend.cwp_weight(),
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            block.sb_h4,
        );
    }
    Ok(resolved)
}

/// § 7.13 single-reference reconstruction input, assembled from the parsed
/// syntax and its resolved motion.
pub(super) fn single_inter_block(
    syntax: InterBlockSyntax,
    resolved: &ResolvedInterBlock,
) -> InterBlock {
    InterBlock {
        ref_frame0: syntax.block_ctx.ref_frame0,
        ref_frame1: None,
        mv: resolved.mv[0],
        mv1: Mv::ZERO,
        interp: syntax.interp,
        warp_params: resolved.warp_params,
        bawp: syntax.bawp,
        interintra: syntax.interintra,
        compound_blend: resolved.blend,
        optflow_distances: syntax.optflow_distances,
        residual: syntax.residual,
    }
}

/// § 7.13 compound reconstruction input, assembled from the parsed syntax and
/// its resolved motion.
pub(super) fn compound_inter_block(
    syntax: InterBlockSyntax,
    compound: &CompoundMotionSyntax,
    resolved: &ResolvedInterBlock,
) -> InterBlock {
    InterBlock {
        ref_frame0: syntax.block_ctx.ref_frame0,
        ref_frame1: Some(compound.ref_frame1),
        mv: resolved.mv[0],
        mv1: resolved.mv[1],
        interp: syntax.interp,
        warp_params: resolved.warp_params,
        bawp: BawpSyntax::default(),
        interintra: None,
        compound_blend: resolved.blend,
        optflow_distances: syntax.optflow_distances,
        residual: syntax.residual,
    }
}

fn resolve_single(
    syntax: &InterBlockSyntax,
    single: &SingleMotionSyntax,
    state: &mut MvResolutionState<'_>,
) -> ResolvedInterBlock {
    let block = &syntax.block_ctx;
    let global_mv = if single.tip_ref {
        Mv::ZERO
    } else {
        global_motion_mv(state.core, block.ref_frame0, block, state.frame_precision)
    };
    let stack = find_mv_stack_with_temporal(
        state.grid,
        block,
        global_mv,
        DEFAULT_WARP_PARAMS,
        state.bank(),
        state.warp_param_bank,
        false,
        state.drl_reorder,
        state.temporal,
        Some(state.order_hints),
        single.use_temporal_first,
    );
    let pred_mv = stack.candidate(single.ref_mv_idx);
    let mv = match single.mode {
        SINGLE_MODE_GLOBALMV => global_motion_mv(
            state.core,
            block.ref_frame0,
            block,
            syntax.precision.mv_precision,
        ),
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let pred_mv = if syntax.use_amvd {
                pred_mv
            } else {
                lowered_pred_mv(syntax.precision, pred_mv)
            };
            add_mv_clamped(pred_mv, single.mvd)
        }
    };
    ResolvedInterBlock {
        mv: [mv, Mv::ZERO],
        warp_params: [single.global_warp, None],
        blend: syntax.blend,
    }
}

fn resolve_warp(
    syntax: &InterBlockSyntax,
    warp: &WarpMotionSyntax,
    state: &mut MvResolutionState<'_>,
) -> Result<ResolvedInterBlock> {
    let block = &syntax.block_ctx;
    let tile_offset = state.tile_offset;
    let global_mv = global_motion_mv(state.core, block.ref_frame0, block, state.frame_precision);
    let stack = find_mv_stack_with_temporal(
        state.grid,
        block,
        global_mv,
        global_motion_model(state.core, block.ref_frame0).gm_params,
        state.bank(),
        state.warp_param_bank,
        warp.derive_wrl,
        state.drl_reorder,
        state.temporal,
        Some(state.order_hints),
        warp.use_temporal_first,
    );
    let (mv, params) = match &warp.source {
        WarpModelSource::Candidate => {
            let base_precision = if warp.mvd.is_some() {
                state.frame_precision
            } else {
                MV_PRECISION_EIGHTH_PEL
            };
            let base_mv = warp_predicted_mv(
                stack.warp_candidate(warp.ref_warp_idx),
                block,
                base_precision,
            );
            let mv = warp.mvd.map_or(base_mv, |mvd| add_mv_clamped(base_mv, mvd));
            let mut params = stack.warp_candidate(warp.ref_warp_idx);
            reduce_warp_model(&mut params);
            set_warp_translation(
                &mut params,
                mv,
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                tile_offset,
            )?;
            (mv, params)
        }
        source => {
            let mv = warp_newmv(syntax, warp, &stack);
            let params = warp_newmv_model(source, warp, block, &stack, mv, state, tile_offset)?;
            (mv, params)
        }
    };
    state.warp_param_bank.update(block.ref_frame0, params);
    Ok(ResolvedInterBlock {
        mv: [mv, Mv::ZERO],
        warp_params: [Some(params), None],
        blend: syntax.blend,
    })
}

fn warp_newmv(syntax: &InterBlockSyntax, warp: &WarpMotionSyntax, stack: &MvStack) -> Mv {
    let pred_mv = lowered_pred_mv(syntax.precision, stack.candidate(warp.ref_mv_idx));
    add_mv_clamped(pred_mv, warp.mvd.unwrap_or(Mv::ZERO))
}

fn warp_newmv_model(
    source: &WarpModelSource,
    warp: &WarpMotionSyntax,
    block: &MvBlockContext,
    stack: &MvStack,
    mv: Mv,
    state: &MvResolutionState<'_>,
    tile_offset: ByteOffset,
) -> Result<[i32; 6]> {
    match source {
        WarpModelSource::LocalSamples => {
            let samples = find_warp_samples(state.grid, block, block.ref_frame0);
            local_warp_estimation(
                &samples,
                mv,
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                tile_offset,
            )
        }
        WarpModelSource::Extended => extend_warp_estimation(
            state.grid,
            block,
            warp.extend_delta,
            stack,
            warp.ref_mv_idx,
            mv,
            tile_offset,
        ),
        WarpModelSource::Delta(delta) => apply_warp_delta(
            stack.warp_candidate(warp.ref_warp_idx),
            *delta,
            mv,
            block,
            tile_offset,
        ),
        WarpModelSource::Candidate => Ok(DEFAULT_WARP_PARAMS),
    }
}

fn resolve_compound(
    syntax: &InterBlockSyntax,
    compound: &CompoundMotionSyntax,
    state: &mut MvResolutionState<'_>,
) -> Result<ResolvedInterBlock> {
    let block = &syntax.block_ctx;
    let precision = syntax.precision.mv_precision;
    let global_mvs = [
        global_motion_mv(state.core, block.ref_frame0, block, precision),
        global_motion_mv(state.core, compound.ref_frame1, block, precision),
    ];
    let paired_temporal = if compound.temporal_allowed {
        state.temporal
    } else {
        None
    };
    let paired = |idx: usize| {
        find_compound_mv_stack_with_temporal(
            state.grid,
            block,
            global_mvs,
            state.bank(),
            state.drl_reorder,
            paired_temporal,
        )
        .candidate(idx)
    };
    if compound.skip_mode {
        let candidate = paired(compound.ref_mv_idx);
        return Ok(ResolvedInterBlock {
            mv: candidate.mvs,
            warp_params: [None, None],
            blend: syntax.blend.average_with_cwp_weight(candidate.cwp_weight),
        });
    }
    let independent = |idx: [usize; 2]| {
        [
            single_list_candidate(
                state,
                block,
                block.ref_frame0,
                global_mvs[0],
                idx[0],
                compound.temporal_first[0],
            ),
            single_list_candidate(
                state,
                block,
                compound.ref_frame1,
                global_mvs[1],
                idx[1],
                compound.temporal_first[1],
            ),
        ]
    };
    let mv = if compound.is_global() {
        global_mvs
    } else {
        compound_mvs(syntax, compound, &paired, &independent)
    };
    let warp_params = if compound.local_warp {
        compound_local_warp_models(
            state.grid,
            block,
            mv[0],
            mv[1],
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            state.tile_offset,
        )?
    } else if compound.is_global() {
        compound.global_warp
    } else {
        [None, None]
    };
    if let Some(params) = warp_params[0] {
        state.warp_param_bank.update(block.ref_frame0, params);
    }
    if let Some(params) = warp_params[1] {
        state.warp_param_bank.update(compound.ref_frame1, params);
    }
    Ok(ResolvedInterBlock {
        mv,
        warp_params,
        blend: syntax.blend,
    })
}

fn single_list_candidate(
    state: &MvResolutionState<'_>,
    block: &MvBlockContext,
    ref_frame: i8,
    global_mv: Mv,
    idx: usize,
    use_temporal_first: bool,
) -> Mv {
    let mut single = *block;
    single.ref_frame0 = ref_frame;
    single.ref_frame1 = None;
    find_mv_stack_with_temporal(
        state.grid,
        &single,
        global_mv,
        DEFAULT_WARP_PARAMS,
        state.bank(),
        state.warp_param_bank,
        false,
        state.drl_reorder,
        state.temporal,
        Some(state.order_hints),
        use_temporal_first,
    )
    .candidate(idx)
}

/// AV2 § 7.12.2 compound predictors combined with the parsed MVD pair.
fn compound_mvs(
    syntax: &InterBlockSyntax,
    compound: &CompoundMotionSyntax,
    paired: &impl Fn(usize) -> CompoundMvCandidate,
    independent: &impl Fn([usize; 2]) -> [Mv; 2],
) -> [Mv; 2] {
    let candidates = || {
        select_near_near_candidates(
            compound.independent_idx,
            compound.ref_mv_idx,
            |idx| paired(idx).mvs,
            independent,
        )
    };
    match compound.y_mode {
        CompoundYMode::GlobalGlobal => [Mv::ZERO, Mv::ZERO],
        CompoundYMode::NearNear => candidates(),
        CompoundYMode::NearNew | CompoundYMode::NewNear => {
            let mut mvs = candidates();
            let new_ref = usize::from(compound.y_mode == CompoundYMode::NearNew);
            mvs[new_ref] = add_mv_clamped(
                lowered_amvd_pred(syntax, mvs[new_ref]),
                compound.mvd[new_ref],
            );
            mvs
        }
        CompoundYMode::JointNew => joint_new_mvs(syntax, compound, paired(compound.ref_mv_idx).mvs),
        CompoundYMode::NewNew => {
            let candidate = paired(compound.ref_mv_idx).mvs;
            [
                add_mv_clamped(lowered_amvd_pred(syntax, candidate[0]), compound.mvd[0]),
                add_mv_clamped(lowered_amvd_pred(syntax, candidate[1]), compound.mvd[1]),
            ]
        }
    }
}

/// AV2 § 7.12.2.4 JOINT_NEWMV: the base list takes the parsed MVD, the other
/// list takes it projected across the reference distances.
fn joint_new_mvs(
    syntax: &InterBlockSyntax,
    compound: &CompoundMotionSyntax,
    candidates: [Mv; 2],
) -> [Mv; 2] {
    let Some(projection) = compound.joint else {
        return candidates;
    };
    let diff = compound.mvd[0];
    let base_mv = add_mv_clamped(
        lowered_amvd_pred(syntax, candidates[projection.base_list]),
        diff,
    );
    let projected = scale_joint_projected_mvd(
        project_joint_mvd(diff, projection.second_dist, projection.first_dist),
        compound.jmvd_scale_mode,
        syntax.use_amvd,
    );
    let other_mv = add_mv_clamped(candidates[1 - projection.base_list], projected);
    if projection.base_list == 0 {
        [base_mv, other_mv]
    } else {
        [other_mv, base_mv]
    }
}

fn lowered_amvd_pred(syntax: &InterBlockSyntax, pred_mv: Mv) -> Mv {
    if syntax.use_amvd {
        pred_mv
    } else {
        lowered_pred_mv(syntax.precision, pred_mv)
    }
}
