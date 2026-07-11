// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
use std::borrow::Cow;

use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameType, MvPrecision, TipFrameMode, TxMode, get_qindex,
};
use splot_core::headers::sequence::{ChromaFormatIdc, DrlReorder, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, TX_HEIGHT, TX_HEIGHT_LOG2, TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::PlaneId as ReconPlaneId;
use splot_recon::{
    BitDepth, CurrentFrameIntraEdges, CurrentFrameWorkspace, IDENTITY_WARP_PARAMS, InterIntraMode,
    InterpolationFilter as ReconInterpolationFilter, IntraCardinalDirection,
    IntraDirectionalAngleEdges, IntraRectBlockSize, IntraSmoothMode, ReconSample,
    apply_intra_ibp_dc_rect, predict_intra_cardinal_directional_rect_into,
    predict_intra_dc_rect_value,
};

use super::find_mv_stack::{
    BlockNeighbourContext, BlockPrecisionRecord, ModeContext, MotionMode, MvBlockContext,
    NeighbourMvGrid, NeighbourYMode, TIP_REF_FRAME, TemporalMotionField, TemporalMvContext,
    TemporalProjectionConfig, block_neighbour_ctx, find_compound_mv_stack_with_temporal,
    find_mode_ctx, find_mode_ctx_with_tip, find_mv_stack_with_temporal,
};
use super::read_mv::{
    MV_PRECISION_EIGHTH_PEL, MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL,
    MvReadConfig, apply_inter_mvd_signs, mv_clamp_to_integer, read_newmv_amvd_block_mvd,
    read_newmv_block_mvd_magnitude,
};
use super::{
    BawpSyntax, InterBlock, InterIntraPrediction, InterReferenceState, InterResidual,
    InterResidualBlock, Mv, PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, SPEC_MODE_INFO, effective_quantizer_deltas_are_zero, mc, unsupported_at,
    unsupported_compound_at,
};
use crate::bitstream::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, BlockSize, CoeffContextReset,
    DecodeBlockFrontier, DecodeTileWorkUnit, FrameCdfSubset, FrameQmSegmentScope,
    GeneralIntraLeafMode, GeneralIntraMultiblockError, GeneralIntraTreeWalkError, IsCflContext,
    LumaCoeffBlock, SavedCdfSubset, TileBlockDecodedState, TileCdfSelector, TileCdfSubset,
    TileCoeffContextState, TileFscModeState, TileIntraJointModeState, TilePartitionTraversalError,
    TileSegmentIdState, TileUsesMrlsState, TransformToolResidualPolicy, chroma_subsampling,
    current_frame_qm_segment_id, decode_general_intra_multiblock_tree_with_lr_source_blocks,
    decode_general_intra_plane_coeffs, frame_mi_dimensions, get_plane_residual_size,
    neg_deinterleave, read_lossless_tx_size,
};
use crate::filters::wienerns_lr::intrabc_records::{
    IntrabcBlockGeometry, IntrabcBlockPrelude, IntrabcInfo, IntrabcUseSkip,
    TileIntrabcPreludeState, derive_intrabc_luma_prediction_geometry, read_intrabc_info,
    read_intrabc_use_and_skip,
};
use crate::filters::wienerns_lr::tx_records::{
    CdefState, DeltaQState, SelectableLumaTxRecord, ccso::CcsoState,
    derive_inter_luma_tx_records_for_block,
};
use crate::pipeline::effective_allow_screen_content_tools;
use crate::pipeline::reconstruct::{
    SmoothIntraPredictionRequest, predict_intra_smooth_over_available_edges,
};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

const INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE: usize = 3;
const INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET: usize = 4;
const SINGLE_REF_FRAME0: i8 = 0;
const MI_SIZE: usize = 4;
const CHUNK_64_N4: usize = 16;
pub(super) const BLOCK_8X8: usize = 3;
const BLOCK_64X64: usize = 12;
const MAX_WARP_REF_CANDIDATES: usize = 4;
const WARP_DELTA_NUM_SYMBOLS_LOW: u8 = 8;
const WARP_DELTA_NUM_SYMBOLS_HIGH: u8 = 8;
pub(crate) const WARPEDMODEL_PREC_BITS: u32 = 16;
const MI_SIZE_LOG2: u32 = 2;
pub(crate) const WARP_PARAM_REDUCE_BITS: u32 = 6;
const WARP_TRANS_INTEGER_BITS: u32 = 12;
const WARP_DELTA_STEP_BITS: u32 = 10;
pub(crate) const WARPEDMODEL_TRANS_CLAMP: i64 =
    1 << (WARPEDMODEL_PREC_BITS + WARP_TRANS_INTEGER_BITS - 1);
#[doc = "AV2 § 9.2 `Size_Group[BLOCK_SIZES]`."]
const SIZE_GROUP_LOOKUP: [usize; 29] = [
    0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2,
];
#[doc = "AV2 § 9.2 `Wedge_Bits[BLOCK_SIZES] != 0`."]
const WEDGE_USED_BY_BSIZE: [bool; 29] = [
    false, false, false, true, true, true, true, true, true, true, true, true, true, false, false,
    false, false, false, false, false, false, true, true, true, true, false, false, true, true,
];
const INTERINTRA_MODES: u8 = 4;
const WEDGE_QUADS: u8 = 4;
const QUAD_WEDGE_ANGLES: u8 = 5;
const H_WEDGE_ANGLES: u8 = 10;
const COEFF_CONTEXT_PLANES: [(usize, u32); 3] = [(0, 0), (1, 1), (2, 1)];
const WEDGE_0: u8 = 0;
const WEDGE_90: u8 = 5;
const NUM_WEDGE_DIST: u8 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WarpInterMode {
    Warpmv,
    WarpNewmv,
}

pub(crate) struct InterFilterInputs {
    pub(crate) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    pub(crate) cdef_grid: crate::filters::cdef::CdefUnitGrid,
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    pub(crate) lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    pub(crate) tx_skip_records: Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(crate) motion_field: TemporalMotionField,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_inter_blocks<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    _qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    initial_cdfs: FrameCdfSubset,
) -> Result<(FrameCdfSubset, InterFilterInputs)> {
    let offset = frame_envelope.offset;
    let frame_is_intra = core.frame_is_intra == Some(true);
    let mut tile_plan = if frame_is_intra {
        crate::pipeline::derive_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            core,
            options,
        )?
    } else {
        crate::pipeline::derive_inter_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            core,
            options,
            initial_cdfs,
        )?
    };
    let work_units = tile_plan.work_units_mut();
    let Some(first_tile) = work_units.first() else {
        return Err(inter_cap!(
            "inter_walk_unexpected_tile_work_units",
            offset,
            "inter.tile_count == 0",
            SPEC_MODE_INFO
        ));
    };
    let first_tile_offset = first_tile.tile_byte_span().start;
    let mut frame_cdfs = first_tile.frame_cdfs();
    let mut saved_cdfs = SavedCdfSubset::from_frame(&frame_cdfs);

    let max_drl_bits_minus_1 = if frame_is_intra {
        0
    } else {
        core.inter
            .as_ref()
            .and_then(|inter| inter.max_drl_bits_minus_1)
            .ok_or_else(|| {
                inter_missing!(
                    "inter_missing_max_drl_bits",
                    offset,
                    "inter.max_drl_bits_minus_1",
                    SPEC_MODE_INFO
                )
            })?
    };

    let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(|_| {
        inter_missing!(
            "inter_mi_dimensions",
            offset,
            "inter.mi_dimensions",
            SPEC_MODE_INFO
        )
    })?;
    let coded_size = workspace.info().coded_luma_size();
    let current_order_hint = core.order_hint_lsb.unwrap_or(0);
    let sb_h4 = superblock_h4(sequence, core).ok_or_else(|| {
        inter_missing!(
            "inter_sb_size",
            offset,
            "inter.superblock_size",
            SPEC_MODE_INFO
        )
    })?;
    let projection_step = tip::tmvp_projection_step(core);
    let temporal_config = TemporalProjectionConfig {
        frame_size: (coded_size.width(), coded_size.height()),
        step: projection_step,
        unit_size8: tip::tmvp_unit_size8(projection_step, sb_h4),
        enable_tip: sequence
            .inter
            .as_ref()
            .is_some_and(|tools| tools.enable_tip),
        enable_trajectory: sequence
            .inter
            .as_ref()
            .is_some_and(|tools| tools.enable_mv_traj),
        reduced: sequence
            .inter
            .as_ref()
            .is_some_and(|tools| tools.reduced_ref_frame_mvs_mode),
    };
    let mut temporal_context = TemporalMvContext::from_references(
        (mi_rows, mi_cols),
        current_order_hint,
        temporal_config,
        ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
        &reference.ref_motion_fields,
    )
    .ok_or_else(|| {
        inter_cap!(
            "inter_temporal_motion_context",
            offset,
            "inter.temporal_motion_context",
            SPEC_MODE_INFO
        )
    })?;
    let mut motion_field = TemporalMotionField::new(mi_rows, mi_cols).ok_or_else(|| {
        inter_cap!(
            "inter_temporal_motion_field",
            offset,
            "inter.temporal_motion_field",
            SPEC_MODE_INFO
        )
    })?;
    motion_field.set_reference_metadata(
        !frame_is_intra,
        temporal_config.frame_size,
        temporal_context.reference_order_hints(),
    );
    let mut cdef_state = CdefState::new(mi_rows, mi_cols, sequence, first_tile_offset)?;
    let mut ccso_state = CcsoState::new(
        mi_rows,
        mi_cols,
        sequence,
        core,
        ref_frame_idx,
        &reference.ref_ccso_unit_grids,
        first_tile_offset,
    )?;
    let (chroma_smooth_rows, chroma_smooth_cols) =
        chroma_smooth_grid_dimensions(mi_rows, mi_cols, sequence.general.chroma_format_idc);
    tip::prepare_motion_field(&mut temporal_context, core, sb_h4);

    let residual_tool_policy = if frame_is_intra {
        crate::pipeline::general_intra::general_intra_transform_tool_residual_policy(sequence)
    } else {
        transform_tool_residual_policy(sequence)
    };
    let residual_quantizer_deltas_are_zero = core
        .quantization_params
        .as_ref()
        .is_some_and(|quant| effective_quantizer_deltas_are_zero(sequence, quant));
    let enable_adaptive_mvd = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_adaptive_mvd);
    let allow_bawp = core.inter_tail.as_ref().is_some_and(|tail| tail.allow_bawp);
    let allow_warpmv_mode = core
        .inter_tail
        .as_ref()
        .is_some_and(|tail| tail.allow_warpmv_mode);
    let frame_is_switch = core.frame_type == Some(FrameType::Switch);

    let mut deblock_blocks: Vec<crate::filters::deblock::DeblockBlock> = Vec::new();
    let mut chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2] =
        [Vec::new(), Vec::new()];
    let mut tx_skip_records = Vec::new();
    let mut decoded_any = false;
    let limits = options.limits();
    let (mut active_source_blocks, mut unit_filters) = (Vec::new(), Vec::new());
    for tile in work_units {
        let tile_offset = tile.tile_byte_span().start;
        let (tile_num, save_policy) = (tile.tile_num(), tile.cdf().save_policy());
        let chroma = sequence.general.chroma_format_idc;
        let mut coeff_ctx =
            TileCoeffContextState::new_chroma(mi_rows, mi_cols, chroma).map_err(|_| {
                inter_cap!(
                    "inter_coeff_context_state",
                    tile_offset,
                    "inter.residual_context_state",
                    SPEC_MODE_INFO
                )
            })?;
        let mut delta_q_state = DeltaQState::new(sequence, core, tile_offset)?;
        let mut intrabc_state =
            TileIntrabcPreludeState::new(mi_rows, mi_cols, sequence, tile_offset)?;
        let mut segment_id_state = TileSegmentIdState::new(mi_rows, mi_cols).map_err(|_| {
            inter_missing!(
                "inter_segment_id_grid",
                tile_offset,
                "inter.segment_id_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols).ok_or_else(|| {
            inter_cap!(
                "inter_mv_grid",
                tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut y_smooth = crate::prediction::intra_edge::TileYSmoothGrid::new(mi_rows, mi_cols)
            .ok_or_else(|| {
                inter_cap!(
                    "inter_y_smooth_grid",
                    tile_offset,
                    "inter.y_smooth_grid",
                    SPEC_MODE_INFO
                )
            })?;
        let mut chroma_smooth = crate::prediction::intra_edge::TileChromaSmoothGrid::new(
            chroma_smooth_rows,
            chroma_smooth_cols,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_chroma_smooth_grid",
                tile_offset,
                "inter.chroma_smooth_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut ref_mv_bank = sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank)
            .then(super::find_mv_stack::RefMvBank::new);
        let mut warp_param_bank = super::find_mv_stack::WarpParamBank::new();
        let walk = decode_general_intra_multiblock_tree_with_lr_source_blocks(
            tile,
            sequence,
            core,
            limits,
            |work_unit,
             symbols,
             frontier,
             joint_modes,
             uses_mrls,
             use_dip,
             fsc_modes,
             palette_state,
             is_cfl_ctx,
             block_decoded| {
                let leaf = decode_block(
                    work_unit,
                    symbols,
                    frontier,
                    sequence,
                    core,
                    &mut coeff_ctx,
                    &mut cdef_state,
                    &mut ccso_state,
                    &mut delta_q_state,
                    &mut intrabc_state,
                    &mut segment_id_state,
                    &mut mv_grid,
                    &temporal_context,
                    &mut motion_field,
                    &mut y_smooth,
                    &mut chroma_smooth,
                    &mut ref_mv_bank,
                    &mut warp_param_bank,
                    sb_h4,
                    mi_rows,
                    mi_cols,
                    max_drl_bits_minus_1,
                    frame_interpolation_filter,
                    residual_tool_policy,
                    residual_quantizer_deltas_are_zero,
                    num_total_refs,
                    reference_select,
                    num_same_ref_compound,
                    joint_modes,
                    uses_mrls,
                    use_dip,
                    fsc_modes,
                    palette_state,
                    is_cfl_ctx,
                    block_decoded,
                    workspace,
                    &mut deblock_blocks,
                    &mut chroma_deblock_blocks,
                    &mut tx_skip_records,
                    luma_use_tcq,
                    residual_use_ddt,
                    ref_frame_idx,
                    reference,
                    bit_depth,
                    enable_adaptive_mvd,
                    allow_bawp,
                    allow_warpmv_mode,
                    frame_is_switch,
                    current_order_hint,
                    tile_offset,
                )?;
                decoded_any = true;
                Ok(leaf)
            },
        )
        .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;
        let crate::bitstream::tile_payload::GeneralIntraMultiblockOutput {
            symbols,
            active_source_blocks: tile_source_blocks,
            unit_filters: tile_unit_filters,
        } = walk;
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
        active_source_blocks.extend(tile_source_blocks);
        unit_filters.extend(tile_unit_filters);
        saved_cdfs.apply_completed_tile(tile_num, tile.cdf().tile_cdfs(), save_policy);
    }
    if !decoded_any {
        return Err(inter_missing!(
            "inter_no_decoded_block",
            first_tile_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }
    frame_cdfs.frame_end_update_from_saved(&saved_cdfs);
    let filter_inputs = InterFilterInputs {
        deblock_blocks,
        chroma_deblock_blocks,
        cdef_grid: cdef_state.into_grid(first_tile_offset)?,
        ccso_grid: ccso_state.into_grid(first_tile_offset)?,
        lr_source_blocks: active_source_blocks,
        lr_unit_filters: unit_filters,
        tx_skip_records,
        motion_field,
    };
    Ok((frame_cdfs, filter_inputs))
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

fn chroma_smooth_grid_dimensions(
    mi_rows: usize,
    mi_cols: usize,
    chroma: splot_core::headers::sequence::ChromaFormatIdc,
) -> (usize, usize) {
    let (sub_x, sub_y) = chroma_subsampling(chroma);
    (
        if sub_y { mi_rows.div_ceil(2) } else { mi_rows },
        if sub_x { mi_cols.div_ceil(2) } else { mi_cols },
    )
}

#[allow(clippy::too_many_arguments)]
fn read_segment_id(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    segment_id_state: &TileSegmentIdState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    skip_flag: bool,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let Some(seg) = core
        .segmentation_params
        .as_ref()
        .filter(|s| s.segmentation_enabled)
    else {
        return Ok(0);
    };
    let (r, c) = (frontier.r, frontier.c);
    let (avail_u, avail_l) = segment_neighbour_availability(
        r,
        c,
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
    );
    let (pred, ctx) = segment_id_state.predictor_and_ctx(r, c, avail_u, avail_l);
    let has_lossless = core
        .lossless_info
        .as_ref()
        .is_some_and(|l| l.has_lossless_segment);
    if skip_flag && !has_lossless {
        return Ok(pred);
    }
    let enable_ext_seg = sequence.segment.as_ref().is_some_and(|s| s.enable_ext_seg);
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let seg_id_ext_flag = if enable_ext_seg {
        cdfs.read_block_symbol_trace(TileCdfSelector::SegIdExtFlag { ctx }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
    } else {
        0
    };
    let ext = seg_id_ext_flag != 0;
    let raw = cdfs
        .read_block_symbol_trace(TileCdfSelector::SegmentId { ctx, ext }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    let coded = i64::from(raw) + if ext { 8 } else { 0 };
    let segment_id = neg_deinterleave(
        coded,
        i64::from(pred),
        i64::from(seg.last_active_seg_id) + 1,
    );
    validate_segment_id(segment_id, seg.last_active_seg_id, tile_offset)
}

fn validate_segment_id(segment_id: i64, last_active: u8, tile_offset: ByteOffset) -> Result<u8> {
    u8::try_from(segment_id)
        .ok()
        .filter(|&id| id <= last_active)
        .ok_or_else(|| {
            inter_cap!(
                "inter_segment_id_out_of_range",
                tile_offset,
                "inter.segment_id out of range",
                "5.20.5.8"
            )
        })
}

pub(super) const fn segment_neighbour_availability(
    r: usize,
    c: usize,
    tile_mi_row_start: usize,
    tile_mi_col_start: usize,
) -> (bool, bool) {
    (r > tile_mi_row_start, c > tile_mi_col_start)
}

fn segment_block_qindex(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    segment_id: usize,
    current_qindex: u32,
) -> u32 {
    let Some(seg) = core
        .segmentation_params
        .as_ref()
        .filter(|s| s.segmentation_enabled)
    else {
        return current_qindex;
    };
    let Some(tq) = sequence.transform_quant_entropy.as_ref() else {
        return current_qindex;
    };
    let quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
    get_qindex(&quant, current_qindex, seg, segment_id)
}

fn current_residual_lossless(work_unit: &DecodeTileWorkUnit<'_>) -> bool {
    work_unit
        .coeff_frame_facts()
        .lossless_for_segment(current_frame_qm_segment_id())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn decode_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    coeff_ctx: &mut TileCoeffContextState,
    cdef_state: &mut CdefState,
    ccso_state: &mut CcsoState,
    delta_q_state: &mut DeltaQState,
    intrabc_state: &mut TileIntrabcPreludeState,
    segment_id_state: &mut TileSegmentIdState,
    mv_grid: &mut NeighbourMvGrid,
    temporal_context: &TemporalMvContext,
    motion_field: &mut TemporalMotionField,
    y_smooth: &mut crate::prediction::intra_edge::TileYSmoothGrid,
    chroma_smooth: &mut crate::prediction::intra_edge::TileChromaSmoothGrid,
    ref_mv_bank: &mut Option<super::find_mv_stack::RefMvBank>,
    warp_param_bank: &mut super::find_mv_stack::WarpParamBank,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    residual_quantizer_deltas_are_zero: bool,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    use_dip: &crate::bitstream::tile_payload::TileUseDipState,
    fsc_modes: &TileFscModeState,
    palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    block_decoded: &mut TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
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
            "inter_block_geometry_height",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO
        )
    })?;
    let mi_row = frontier.r;
    let mi_col = frontier.c;
    let placed_geometry = placed_inter_geometry(
        frontier,
        n4w,
        n4h,
        sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
        tile_offset,
    )?;
    let placed_block = |block| PlacedInterBlock {
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
        interintra_chroma: placed_geometry.interintra_chroma,
        block,
    };

    let mut block_ctx = MvBlockContext {
        mi_row,
        mi_col,
        bw4: n4w,
        bh4: n4h,
        sb_h4,
        ref_frame0: SINGLE_REF_FRAME0,
        ref_frame1: None,
        mi_rows,
        mi_cols,
    };

    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.reset_for_leaf(mv_grid, mi_row, mi_col, sb_h4);
    }
    warp_param_bank.reset_for_leaf(mv_grid, mi_row, mi_col, sb_h4);
    let comp_ref_allowed = is_comp_ref_allowed(n4w, n4h);
    let drl_reorder = sequence
        .inter
        .as_ref()
        .map_or(DrlReorder::Disabled, |inter| inter.drl_reorder);
    let refs_one_sided = {
        let mut has_past = false;
        let mut has_future = false;
        for list_ref in 0..num_total_refs {
            let Some(hint) = ref_frame_idx
                .get(list_ref)
                .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
            else {
                continue;
            };
            let dist = super::get_relative_dist(
                current_order_hint as i32,
                i32::try_from(*hint).unwrap_or(i32::MAX),
            );
            if dist > 0 {
                has_past = true;
            } else if dist < 0 {
                has_future = true;
            }
        }
        !has_past || !has_future
    };
    let use_temporal = core
        .inter
        .as_ref()
        .and_then(|inter| inter.use_ref_frame_mvs)
        == Some(true);
    let temporal_stack_context = use_temporal.then_some(temporal_context);
    let tip_ref_pair = temporal_context
        .tip_references()
        .map(|references| (references.past_ref, references.future_ref));
    let temporal_first_frame = drl_reorder != DrlReorder::Always && use_temporal && refs_one_sided;
    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);
    let skip_mode = read_skip_mode_syntax(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        core.inter_tail
            .as_ref()
            .is_some_and(|tail| tail.skip_mode_present),
        frontier,
        comp_ref_allowed,
        neighbour_ctx.skip_mode_ctx,
        tile_offset,
    )?;

    let is_inter = if core.frame_is_intra == Some(true)
        || frontier.is_luma_part()
        || frontier.is_chroma_part()
    {
        0
    } else if skip_mode == 1 || frontier.shared_mixed_chroma_ref_forces_inter() {
        1
    } else {
        let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
        cdfs.read_block_symbol_trace(
            TileCdfSelector::IsInter {
                ctx: neighbour_ctx.is_inter_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get()
    };
    if is_inter == 0 {
        let seg_pre_skip = core
            .segmentation_params
            .as_ref()
            .is_some_and(|seg| seg.segmentation_enabled && seg.seg_id_pre_skip);
        let mut segment_id: u8 = if frontier.is_chroma_part() {
            segment_id_state.cell(frontier.r, frontier.c).unwrap_or(0)
        } else {
            0
        };
        let mut prelude = IntrabcBlockPrelude::from_use_skip(
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: false,
            },
            None,
        );
        if !frontier.is_chroma_part() {
            if seg_pre_skip {
                segment_id = read_segment_id(
                    work_unit,
                    symbols,
                    segment_id_state,
                    sequence,
                    core,
                    frontier,
                    false,
                    tile_offset,
                )?;
            }
            intrabc_state.prepare_for_block(frontier.r, frontier.c);
            let use_skip = read_intrabc_use_and_skip(
                work_unit.cdf_mut().tile_cdfs_mut(),
                symbols,
                intrabc_state,
                core,
                IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
                tile_offset,
            )?;
            if !seg_pre_skip {
                segment_id = read_segment_id(
                    work_unit,
                    symbols,
                    segment_id_state,
                    sequence,
                    core,
                    frontier,
                    use_skip.skip_flag,
                    tile_offset,
                )?;
            }
            cdef_state.read_for_block(
                work_unit,
                symbols,
                core,
                frontier,
                n4w,
                n4h,
                use_skip.skip_flag,
                tile_offset,
            )?;
            ccso_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
            delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
            let intrabc = if use_skip.use_intrabc {
                Some(read_intrabc_info(
                    work_unit.cdf_mut().tile_cdfs_mut(),
                    symbols,
                    intrabc_state,
                    sequence,
                    core,
                    IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
                    tile_offset,
                )?)
            } else {
                None
            };
            prelude = IntrabcBlockPrelude::from_use_skip(use_skip, intrabc);
        }
        if !frontier.is_chroma_part() {
            segment_id_state.record_block(frontier.r, frontier.c, n4w, n4h, segment_id);
        }
        if prelude.use_intrabc {
            let info = prelude.intrabc.ok_or_else(|| {
                inter_missing!(
                    "inter_intrabc_info",
                    tile_offset,
                    "inter.intrabc.info",
                    SPEC_MODE_INFO
                )
            })?;
            let (sub_x, sub_y) = chroma_subsampling(sequence.general.chroma_format_idc);
            reconstruct_intrabc_predictor(
                workspace,
                core,
                frontier,
                n4w,
                n4h,
                info,
                (u32::from(sub_x), u32::from(sub_y)),
                tile_offset,
            )?;
            let block_qindex = segment_block_qindex(
                sequence,
                core,
                usize::from(segment_id),
                delta_q_state.qindex_u32(),
            );
            let lossless = work_unit
                .coeff_frame_facts()
                .lossless_for_segment(usize::from(segment_id))
                .unwrap_or(false);
            let residual = if prelude.skip_flag {
                reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
                None
            } else {
                let _segment_scope = FrameQmSegmentScope::install(usize::from(segment_id));
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
                    lossless,
                    InterResidualLumaTxSizeMode::Intrabc,
                    residual_tool_policy,
                    tile_offset,
                )?)
            };
            record_inter_deblock_geometry(
                deblock_blocks,
                chroma_deblock_blocks,
                tx_skip_records,
                frontier,
                (n4w, n4h),
                sequence.general.chroma_format_idc,
                residual.as_ref(),
                None,
                block_qindex,
                lossless,
                tile_offset,
            )?;
            if let Some(residual) = residual.as_ref() {
                super::add_inter_residual_to_workspace(
                    workspace,
                    residual,
                    block_qindex,
                    luma_use_tcq,
                    residual_use_ddt,
                    bit_depth,
                    tile_offset,
                )?;
            }
            mv_grid.record_block(
                mi_row,
                mi_col,
                n4w,
                n4h,
                true,
                -1,
                None,
                NeighbourYMode::Other,
                Mv::ZERO,
                prelude.skip_flag,
                interp_filter_no_neighbour_ctx(false) as u8,
                false,
                BlockPrecisionRecord::explicit(frame_mv_precision(core, tile_offset)?),
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            if let Some(bank) = ref_mv_bank.as_mut() {
                bank.update_count_for_non_inter(mi_row, mi_col, n4w, n4h, sb_h4);
            }
            return Ok(non_intra_leaf_mode(frontier).mark_intrabc());
        }
        let block_qindex = segment_block_qindex(
            sequence,
            core,
            usize::from(segment_id),
            delta_q_state.qindex_u32(),
        );
        ensure_intra_leaf_quantizer_delta_scope(
            core.frame_is_intra == Some(true),
            residual_quantizer_deltas_are_zero,
            tile_offset,
        )?;
        let leaf = crate::pipeline::general_intra::decode_one_general_intra_block::<T>(
            work_unit,
            symbols,
            frontier,
            sequence,
            Some(&*y_smooth),
            Some(chroma_smooth),
            core,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_state,
            is_cfl_ctx,
            segment_id,
            block_decoded,
            workspace,
            coeff_ctx,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            block_qindex,
            luma_use_tcq,
            residual_tool_policy,
            mi_cols,
            mi_rows,
            bit_depth,
            tile_offset,
        )?;
        if !frontier.is_chroma_part() {
            y_smooth.record(mi_row, mi_col, n4w, n4h, leaf.y_mode_is_smooth());
            mv_grid.record_block(
                mi_row,
                mi_col,
                n4w,
                n4h,
                false,
                -1,
                None,
                NeighbourYMode::Other,
                Mv::ZERO,
                false,
                interp_filter_no_neighbour_ctx(false) as u8,
                false,
                BlockPrecisionRecord::explicit(frame_mv_precision(core, tile_offset)?),
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            if let Some(bank) = ref_mv_bank.as_mut() {
                bank.update_count_for_non_inter(mi_row, mi_col, n4w, n4h, sb_h4);
            }
        }
        return Ok(leaf);
    }
    if is_inter != 1 {
        return Err(inter_cap!(
            "inter_block_is_intra",
            tile_offset,
            "inter.block.is_inter out of range",
            SPEC_MODE_INFO
        ));
    }
    let skip = {
        let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
        cdfs.read_block_symbol_trace(
            TileCdfSelector::Skip {
                ctx: inter_skip_txfm_ctx(neighbour_ctx.skip_ctx, skip_mode == 1),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
    };
    let skip = skip.get();
    if skip != 0 && skip != 1 {
        return Err(inter_cap!(
            "inter_block_unexpected_skip",
            tile_offset,
            "inter.block.skip out of range",
            SPEC_MODE_INFO
        ));
    }

    let segment_id = if frontier.is_chroma_part() {
        segment_id_state.cell(frontier.r, frontier.c).unwrap_or(0)
    } else {
        let segment_id = read_segment_id(
            work_unit,
            symbols,
            segment_id_state,
            sequence,
            core,
            frontier,
            skip == 1,
            tile_offset,
        )?;
        segment_id_state.record_block(frontier.r, frontier.c, n4w, n4h, segment_id);
        segment_id
    };
    let _segment_scope = FrameQmSegmentScope::install(usize::from(segment_id));

    cdef_state.read_for_block(
        work_unit,
        symbols,
        core,
        frontier,
        n4w,
        n4h,
        skip == 1,
        tile_offset,
    )?;
    ccso_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    let block_qindex = segment_block_qindex(
        sequence,
        core,
        usize::from(segment_id),
        delta_q_state.qindex_u32(),
    );
    if skip_mode == 1 {
        return compound_path::decode_skip_mode_inter_block(
            work_unit,
            symbols,
            coeff_ctx,
            sequence,
            core,
            frontier,
            workspace,
            block_decoded,
            mv_grid,
            temporal_stack_context,
            motion_field,
            &mut block_ctx,
            &neighbour_ctx,
            ref_mv_bank,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            intrabc_state,
            ref_frame_idx,
            reference,
            num_total_refs,
            skip,
            n4w,
            n4h,
            mi_row,
            mi_col,
            mi_rows,
            mi_cols,
            sb_h4,
            max_drl_bits_minus_1,
            drl_reorder,
            residual_quantizer_deltas_are_zero,
            residual_tool_policy,
            block_qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        );
    }
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let tip_frame_mode = core
        .inter
        .as_ref()
        .and_then(|inter| inter.tip_frame_mode)
        .unwrap_or(TipFrameMode::Disabled);
    let tip_ref = tip::read_reference(
        cdfs,
        symbols,
        tip_frame_mode,
        frontier,
        &neighbour_ctx,
        (n4w, n4h),
        tile_offset,
    )?;
    let uses_compound = if !tip_ref && reference_select && is_comp_ref_allowed(n4w, n4h) {
        compound_path::read_reference_mode(
            cdfs,
            symbols,
            &neighbour_ctx,
            ref_frame_idx,
            &reference.ref_order_hint,
            current_order_hint,
            tile_offset,
        )?
    } else {
        false
    };
    if uses_compound {
        return compound_path::decode_compound_inter_block(
            work_unit,
            symbols,
            coeff_ctx,
            sequence,
            core,
            frontier,
            workspace,
            block_decoded,
            mv_grid,
            temporal_stack_context,
            tip_ref_pair,
            motion_field,
            &mut block_ctx,
            &neighbour_ctx,
            ref_mv_bank,
            warp_param_bank,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            intrabc_state,
            ref_frame_idx,
            reference,
            num_total_refs,
            num_same_ref_compound,
            skip,
            n4w,
            n4h,
            mi_row,
            mi_col,
            mi_rows,
            mi_cols,
            sb_h4,
            max_drl_bits_minus_1,
            drl_reorder,
            temporal_first_frame,
            enable_adaptive_mvd,
            residual_quantizer_deltas_are_zero,
            residual_tool_policy,
            block_qindex,
            frame_interpolation_filter,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        );
    }

    let ref_frame0: i8 = if tip_ref {
        TIP_REF_FRAME
    } else if num_total_refs >= 2 {
        let decisions = num_total_refs - 1;
        let mut contexts = [0usize; 6];
        for (ref_idx, ctx) in contexts.iter_mut().take(decisions).enumerate() {
            *ctx = neighbour_ctx
                .single_ref_ctx(ref_idx, num_total_refs)
                .ok_or_else(|| {
                    inter_missing!(
                        "inter_block_single_ref_ctx",
                        tile_offset,
                        "inter.single_ref.context",
                        SPEC_MODE_INFO
                    )
                })?;
        }
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
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);
    let force_integer_mv = effective_force_integer_mv(core);
    let warp_mode = if tip_ref {
        None
    } else {
        read_warp_inter_mode_syntax(
            cdfs,
            symbols,
            allow_warpmv_mode,
            force_integer_mv,
            n4w,
            n4h,
            mode_ctx.warp_mv_count,
            tile_offset,
        )?
    };
    if let Some(warp_mode) = warp_mode {
        let derive_wrl = n4w >= 2 && n4h >= 2;
        let use_temporal_first = temporal_first_frame
            && block_ref_within_temporal_distance(
                reference,
                ref_frame_idx,
                current_order_hint,
                ref_frame0,
            );
        let stack = find_mv_stack_with_temporal(
            mv_grid,
            &block_ctx,
            Mv::ZERO,
            ref_mv_bank
                .as_ref()
                .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2)),
            warp_param_bank,
            derive_wrl,
            drl_reorder,
            temporal_stack_context,
            use_temporal_first,
        );
        let mv_config = inter_mv_read_config(core, tile_offset)?;
        let motion_mode = if warp_mode == WarpInterMode::WarpNewmv {
            read_warp_newmv_motion_mode_syntax(
                cdfs,
                symbols,
                core,
                &neighbour_ctx,
                mode_ctx.warp_sample_found,
                tile_offset,
            )?
        } else {
            MotionMode::DeltaWarp
        };
        let warp = match (warp_mode, motion_mode) {
            (WarpInterMode::WarpNewmv, MotionMode::ExtendWarp | MotionMode::LocalWarp) => {
                read_warp_extend_syntax(
                    cdfs,
                    symbols,
                    sequence,
                    core,
                    &neighbour_ctx,
                    mv_config,
                    mv_grid,
                    &block_ctx,
                    &mode_ctx,
                    motion_mode,
                    mi_row,
                    mi_col,
                    n4w,
                    n4h,
                    &stack,
                    mode_ctx.new_mv_context,
                    max_drl_bits_minus_1,
                    tile_offset,
                )?
            }
            (WarpInterMode::WarpNewmv, _) => read_warp_newmv_delta_syntax(
                cdfs,
                symbols,
                sequence,
                core,
                &neighbour_ctx,
                mv_config,
                frontier.b_size.index(),
                mi_row,
                mi_col,
                n4w,
                n4h,
                &stack,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?,
            (WarpInterMode::Warpmv, _) => read_warpmv_delta_syntax(
                cdfs,
                symbols,
                mv_config,
                frontier.b_size.index(),
                mi_row,
                mi_col,
                n4w,
                n4h,
                &stack,
                tile_offset,
            )?,
        };
        let warp_inter_intra = if warp_mode == WarpInterMode::Warpmv {
            read_warp_inter_intra_syntax(
                cdfs,
                symbols,
                frontier.b_size.index(),
                n4w,
                n4h,
                tile_offset,
            )?
        } else {
            WarpInterIntraSyntax::default()
        };
        let warp_interintra_mode = interintra_prediction_mode(warp_inter_intra, tile_offset)?;
        let residual = if skip == 0 {
            if !residual_quantizer_deltas_are_zero {
                return Err(inter_cap!(
                    "inter_block_residual_quantizer_delta_warp",
                    tile_offset,
                    "inter.residual.nonzero_quantizer_delta",
                    SPEC_MODE_INFO
                ));
            }
            if !inter_residual_geometry_supported(frontier) {
                return Err(inter_cap!(
                    "inter_block_chroma_partitioned_residual_warp",
                    tile_offset,
                    "inter.residual.chroma_partition_geometry",
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
            None,
            block_qindex,
            current_residual_lossless(work_unit),
            tile_offset,
        )?;
        mv_grid.record_warp_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            ref_frame0,
            NeighbourYMode::Other,
            warp.mv,
            skip == 1,
            interp_filter_symbol(ReconInterpolationFilter::EightTap),
            false,
            motion_mode,
            warp.warp_params,
            warp.block_precision,
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
            core.order_hint_lsb.unwrap_or(0),
            ref_frame0,
            None,
            warp.mv,
            Mv::ZERO,
            [Some(warp.warp_params), None],
        );
        warp_param_bank.update(ref_frame0, warp.warp_params);
        if let Some(bank) = ref_mv_bank.as_mut() {
            bank.update_for_block(
                ref_frame0,
                None,
                warp.mv,
                None,
                mc::CWP_EQUAL,
                mi_row,
                mi_col,
                n4w,
                n4h,
                sb_h4,
            );
        }
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

        let placed = placed_block(InterBlock {
            ref_frame0,
            ref_frame1: None,
            mv: warp.mv,
            mv1: Mv::ZERO,
            interp: ReconInterpolationFilter::EightTap,
            warp_params: [Some(warp.warp_params), None],
            bawp: BawpSyntax::default(),
            interintra: warp_interintra_mode,
            compound_blend: mc::CompoundBlend::default(),
            optflow_distances: None,
            residual,
        });
        prediction::reconstruct_placed_inter_block(
            workspace,
            &placed,
            false,
            block_decoded,
            ref_frame_idx,
            reference,
            block_qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            sequence_enables_ibp(sequence),
            tile_offset,
        )
        .map(drop)?;
        return Ok(non_intra_leaf_mode(frontier));
    }

    let single_mode = if tip_ref {
        if cdfs
            .read_block_symbol_trace(TileCdfSelector::TipPredMode, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            == 0
        {
            SINGLE_MODE_NEARMV
        } else {
            SINGLE_MODE_NEWMV
        }
    } else {
        cdfs.read_block_symbol_trace(
            TileCdfSelector::SingleMode {
                ctx: mode_ctx.new_mv_context,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get()
    };
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
    let use_amvd = read_use_amvd_syntax(
        cdfs,
        symbols,
        enable_adaptive_mvd,
        single_mode,
        neighbour_ctx.amvd_ctx(ref_frame0),
        tile_offset,
    )?;
    let bawp = if tip_ref {
        BawpSyntax::default()
    } else {
        read_bawp_syntax(
            cdfs,
            symbols,
            BawpParseInput {
                allow_bawp,
                frame_is_switch,
                single_mode,
                use_amvd,
                n4w,
                n4h,
                has_chroma: frontier.has_chroma,
            },
            tile_offset,
        )?
    };
    let bawp = if bawp.enabled {
        let slot = usize::try_from(ref_frame0)
            .ok()
            .and_then(|list_ref| ref_frame_idx.get(list_ref).copied())
            .unwrap_or(0);
        let ref_hint = reference
            .ref_order_hint
            .get(slot as usize)
            .copied()
            .map_or(0, |hint| i32::try_from(hint).unwrap_or(i32::MAX));
        BawpSyntax {
            ref_dist_gt4: super::get_relative_dist(ref_hint, current_order_hint as i32).abs() > 4,
            ..bawp
        }
    } else {
        bawp
    };
    let interintra = if tip_ref {
        None
    } else if !bawp.enabled {
        let syntax = read_inter_intra_syntax(
            cdfs,
            symbols,
            core,
            frontier.b_size.index(),
            n4w,
            n4h,
            tile_offset,
        )?;
        interintra_prediction_mode(syntax, tile_offset)?
    } else {
        None
    };
    let use_temporal_first = !tip_ref
        && temporal_first_frame
        && block_ref_within_temporal_distance(
            reference,
            ref_frame_idx,
            current_order_hint,
            ref_frame0,
        );
    let stack = find_mv_stack_with_temporal(
        mv_grid,
        &block_ctx,
        Mv::ZERO,
        ref_mv_bank
            .as_ref()
            .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2)),
        warp_param_bank,
        false,
        drl_reorder,
        temporal_stack_context,
        use_temporal_first,
    );

    let ref_mv_idx = if single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV {
        if tip_ref {
            read_tip_drl_idx(cdfs, symbols, max_drl_bits_minus_1, tile_offset)?
        } else {
            read_drl_idx(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?
        }
    } else {
        0
    };
    let frame_mv_config = inter_mv_read_config(core, tile_offset)?;
    let precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        &neighbour_ctx,
        frame_mv_config.precision(),
        single_mode == SINGLE_MODE_NEWMV,
        use_amvd,
        tile_offset,
    )?;

    let pred_mv = stack.candidate(ref_mv_idx);
    let mv = match single_mode {
        SINGLE_MODE_GLOBALMV => Mv::ZERO,
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let config = MvReadConfig::inter(precision.mv_precision);
            let diff = if use_amvd {
                let magnitude = read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?;
                apply_inter_mvd_signs(magnitude, symbols, tile_offset, config, false, 1)?
            } else {
                let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?;
                apply_inter_mvd_signs(
                    magnitude,
                    symbols,
                    tile_offset,
                    config,
                    inter_mvd_sign_derivation_allowed(
                        sequence,
                        core,
                        single_mode,
                        use_amvd,
                        frame_mv_config,
                        config,
                    ),
                    1,
                )?
            };
            let pred_mv = if use_amvd {
                pred_mv
            } else {
                lowered_pred_mv(precision, pred_mv)
            };
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
    };

    let interp = if tip_ref {
        ReconInterpolationFilter::EightTapSharp
    } else {
        let interp_ctx = neighbour_ctx.interp_filter_ctx(ref_frame0, false);
        resolve_interp_filter(
            cdfs,
            symbols,
            frame_interpolation_filter,
            single_inter_needs_interp_filter(n4w, n4h, single_mode),
            interp_ctx,
            tile_offset,
        )?
    };
    let residual = if skip == 0 {
        if !residual_quantizer_deltas_are_zero {
            return Err(inter_cap!(
                "inter_block_residual_quantizer_delta",
                tile_offset,
                "inter.residual.nonzero_quantizer_delta",
                SPEC_MODE_INFO
            ));
        }
        if !inter_residual_geometry_supported(frontier) {
            return Err(inter_cap!(
                "inter_block_chroma_partitioned_residual",
                tile_offset,
                "inter.residual.chroma_partition_geometry",
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
    let tip_uses_16x16_units = tip_ref
        && tip::reference_uses_16x16_units(
            n4w,
            n4h,
            sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.enable_tip_refinemv),
        );
    record_inter_deblock_geometry(
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        frontier,
        (n4w, n4h),
        sequence.general.chroma_format_idc,
        residual.as_ref(),
        tip_ref.then_some(if tip_uses_16x16_units { 16 } else { 8 }),
        block_qindex,
        current_residual_lossless(work_unit),
        tile_offset,
    )?;
    let y_mode = if single_mode == SINGLE_MODE_NEWMV {
        NeighbourYMode::NewMv
    } else {
        NeighbourYMode::Other
    };
    if tip_ref {
        mv_grid.record_tip_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            y_mode,
            mv,
            skip == 1,
            interp_filter_symbol(interp),
            use_amvd,
            tip_uses_16x16_units,
            precision,
        );
    } else {
        mv_grid.record_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            true,
            ref_frame0,
            None,
            y_mode,
            mv,
            skip == 1,
            interp_filter_symbol(interp),
            use_amvd,
            precision,
        );
    }
    if !tip_ref {
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
            core.order_hint_lsb.unwrap_or(0),
            ref_frame0,
            None,
            mv,
            Mv::ZERO,
            [None, None],
        );
    }
    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.update_for_block(
            ref_frame0,
            None,
            mv,
            None,
            mc::CWP_EQUAL,
            mi_row,
            mi_col,
            n4w,
            n4h,
            sb_h4,
        );
    }
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

    let placed = placed_block(InterBlock {
        ref_frame0,
        ref_frame1: None,
        mv,
        mv1: Mv::ZERO,
        interp,
        warp_params: [None, None],
        bawp,
        interintra,
        compound_blend: mc::CompoundBlend::default(),
        optflow_distances: None,
        residual,
    });
    if tip_ref {
        tip::reconstruct(
            workspace,
            &placed,
            temporal_context,
            sequence,
            core,
            ref_frame_idx,
            reference,
            Some(motion_field),
            block_qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        )?;
        return Ok(non_intra_leaf_mode(frontier));
    }
    prediction::reconstruct_placed_inter_block(
        workspace,
        &placed,
        false,
        block_decoded,
        ref_frame_idx,
        reference,
        block_qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        sequence_enables_ibp(sequence),
        tile_offset,
    )
    .map(drop)?;
    Ok(non_intra_leaf_mode(frontier))
}

const fn inter_skip_txfm_ctx(neighbour_skip_ctx: usize, skip_mode: bool) -> usize {
    neighbour_skip_ctx + if skip_mode { 3 } else { 0 }
}

fn non_intra_leaf_mode(frontier: &DecodeBlockFrontier) -> GeneralIntraLeafMode {
    let leaf = GeneralIntraLeafMode::no_luma_mode();
    if frontier.has_chroma {
        return leaf.with_uv_cfl(false);
    }
    leaf
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_intrabc_predictor<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    info: IntrabcInfo,
    (sub_x, sub_y): (u32, u32),
    tile_offset: ByteOffset,
) -> Result<()> {
    let prediction = derive_intrabc_luma_prediction_geometry(
        core,
        IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
        info,
        tile_offset,
    )?;
    if prediction.fractional {
        super::mc::intrabc_predict_fractional_luma_into(
            workspace,
            prediction.target,
            prediction.scaling,
        )?;
    } else if prediction.source.size() != prediction.target.size() {
        return Err(inter_cap!(
            "inter_intrabc_fractional_predictor",
            tile_offset,
            "inter.intrabc.fractional_predictor",
            SPEC_MODE_INFO
        ));
    } else {
        workspace
            .copy_rect_within_plane(ReconPlaneId::Y, prediction.source, prediction.target)
            .map_err(|_| {
                inter_cap!(
                    "inter_intrabc_copy",
                    tile_offset,
                    "inter.intrabc.copy",
                    SPEC_MODE_INFO
                )
            })?;
    }
    if info.morph_pred {
        super::bawp::apply_intrabc_morph_pred(
            workspace,
            prediction.target,
            Mv {
                row: info.block_mv.row,
                col: info.block_mv.col,
            },
            tile_offset,
        )?;
    }
    if !frontier.has_chroma {
        return Ok(());
    }
    let chroma_prediction = if frontier.chroma_offset {
        let chroma_ref = frontier.chroma_ref_geometry();
        derive_intrabc_luma_prediction_geometry(
            core,
            IntrabcBlockGeometry::from_chroma_ref(
                chroma_ref.row(),
                chroma_ref.col(),
                chroma_ref.size(),
                tile_offset,
            )?,
            info,
            tile_offset,
        )?
    } else {
        prediction
    };
    let luma = chroma_prediction.target;
    for plane in [ReconPlaneId::U, ReconPlaneId::V] {
        let (cx, cy) = (luma.x() >> sub_x, luma.y() >> sub_y);
        let (cw, ch) = (luma.width() >> sub_x, luma.height() >> sub_y);
        if cw == 0 || ch == 0 {
            continue;
        }
        let scaling = super::mv_scaling::derive_plane_scaling(
            cx as i64,
            cy as i64,
            i64::from(info.block_mv.row),
            i64::from(info.block_mv.col),
            sub_x,
            sub_y,
            chroma_prediction.ref_mi_cols,
            chroma_prediction.ref_mi_rows,
            cw as i64,
            ch as i64,
        );
        let target = splot_recon::PlaneRect::new(cx, cy, cw, ch).map_err(|_| {
            inter_cap!(
                "inter_intrabc_chroma_geometry",
                tile_offset,
                "inter.intrabc.chroma.geometry",
                SPEC_MODE_INFO
            )
        })?;
        super::mc::intrabc_predict_subpel_plane_into(workspace, plane, target, scaling)?;
    }
    Ok(())
}

pub(crate) fn is_comp_ref_allowed(n4w: usize, n4h: usize) -> bool {
    n4w.min(n4h) >= 2 || (n4w == 1 && n4h >= 4) || (n4h == 1 && n4w >= 4)
}

fn inter_residual_geometry_supported(frontier: &DecodeBlockFrontier) -> bool {
    inter_residual_geometry_supported_flags(frontier.is_luma_part(), frontier.is_chroma_part())
}

const fn inter_residual_geometry_supported_flags(is_luma_part: bool, is_chroma_part: bool) -> bool {
    !is_luma_part && !is_chroma_part
}

fn sequence_enables_ibp(sequence: &SequenceHeader) -> bool {
    sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp)
}

fn interintra_cardinal_edge<T: ReconSample>(
    mode: InterIntraMode,
    edges: &CurrentFrameIntraEdges<T>,
    len: usize,
    bit_depth: BitDepth,
) -> splot_recon::Result<Cow<'_, [T]>> {
    let sample = |above: bool| {
        if above {
            edges
                .left_samples()
                .and_then(|left| left.first().copied())
                .map_or_else(|| no_neighbour_above(bit_depth), Ok)
        } else {
            edges
                .above_samples()
                .and_then(|above_edge| above_edge.first().copied())
                .map_or_else(|| no_neighbour_left(bit_depth), Ok)
        }
    };
    match mode {
        InterIntraMode::Vertical => edges.above_samples().map_or_else(
            || sample(true).map(|value| Cow::Owned(vec![value; len])),
            |above| Ok(Cow::Borrowed(above)),
        ),
        InterIntraMode::Horizontal => edges.left_samples().map_or_else(
            || sample(false).map(|value| Cow::Owned(vec![value; len])),
            |left| Ok(Cow::Borrowed(left)),
        ),
        InterIntraMode::Dc | InterIntraMode::Smooth => Ok(Cow::Borrowed(&[])),
    }
}

fn no_neighbour_above<T: ReconSample>(bit_depth: BitDepth) -> splot_recon::Result<T> {
    let midpoint = 1u16 << (u32::from(bit_depth.bits()) - 1);
    T::try_from_u16(midpoint - 1)
}

fn no_neighbour_left<T: ReconSample>(bit_depth: BitDepth) -> splot_recon::Result<T> {
    let midpoint = 1u16 << (u32::from(bit_depth.bits()) - 1);
    T::try_from_u16(midpoint + 1)
}

struct InterIntraPlanePrediction<T> {
    plane: ReconPlaneId,
    sub_x: u32,
    sub_y: u32,
    x: usize,
    y: usize,
    size: IntraRectBlockSize,
    samples: Vec<T>,
}

fn predict_interintra_planes<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    block_decoded: &TileBlockDecodedState,
    mode: InterIntraMode,
    enable_ibp: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Vec<InterIntraPlanePrediction<T>>> {
    let geometry_error = || {
        inter_diag!(
            "inter_interintra_geometry",
            tile_offset,
            "invalid interintra plane geometry",
            "5.20.7.22"
        )
    };
    let mut planes = Vec::with_capacity(mc::YUV420_MC_PLANES.len());
    for (plane, sub_x, sub_y) in mc::YUV420_MC_PLANES {
        if plane != ReconPlaneId::Y && !placed.interintra_chroma {
            continue;
        }
        let (luma_x, luma_y, luma_w, luma_h) = if plane == ReconPlaneId::Y {
            (placed.luma_x, placed.luma_y, placed.luma_w, placed.luma_h)
        } else {
            (
                placed.chroma_luma_x,
                placed.chroma_luma_y,
                placed.chroma_luma_w,
                placed.chroma_luma_h,
            )
        };
        let x = luma_x >> sub_x;
        let y = luma_y >> sub_y;
        let w = luma_w >> sub_x;
        let h = luma_h >> sub_y;
        if !w.is_power_of_two() || !h.is_power_of_two() {
            return Err(geometry_error());
        }
        let log2_w = u8::try_from(w.trailing_zeros()).map_err(|_| geometry_error())?;
        let log2_h = u8::try_from(h.trailing_zeros()).map_err(|_| geometry_error())?;
        let size = IntraRectBlockSize::new(log2_w, log2_h).map_err(|_| geometry_error())?;
        let edges = workspace
            .intra_dc_edges_for_rect(plane, x, y, size)
            .map_err(|_| geometry_error())?;
        let mut samples = vec![T::default(); w * h];
        match mode {
            InterIntraMode::Dc => {
                let dc = predict_intra_dc_rect_value(bit_depth, size, edges.as_dc_edges())
                    .map_err(|_| geometry_error())?;
                samples.fill(dc);
                if enable_ibp && !(w == 4 && h == 4) {
                    apply_intra_ibp_dc_rect(bit_depth, size, edges.as_dc_edges(), &mut samples, w)
                        .map_err(|_| geometry_error())?;
                }
            }
            InterIntraMode::Vertical | InterIntraMode::Horizontal => {
                let (direction, edge) = if mode == InterIntraMode::Vertical {
                    (
                        IntraCardinalDirection::Vertical,
                        interintra_cardinal_edge(mode, &edges, w, bit_depth)
                            .map_err(|_| geometry_error())?,
                    )
                } else {
                    (
                        IntraCardinalDirection::Horizontal,
                        interintra_cardinal_edge(mode, &edges, h, bit_depth)
                            .map_err(|_| geometry_error())?,
                    )
                };
                let prepared = if mode == InterIntraMode::Vertical {
                    IntraDirectionalAngleEdges::above(edge.as_ref())
                } else {
                    IntraDirectionalAngleEdges::left(edge.as_ref())
                };
                predict_intra_cardinal_directional_rect_into(
                    bit_depth,
                    size,
                    direction,
                    prepared,
                    &mut samples,
                    w,
                )
                .map_err(|_| geometry_error())?;
            }
            InterIntraMode::Smooth => {
                let x4 = x / MI_SIZE;
                let y4 = y / MI_SIZE;
                let w4 = (w / MI_SIZE).max(1);
                let h4 = (h / MI_SIZE).max(1);
                samples = predict_intra_smooth_over_available_edges(
                    workspace,
                    SmoothIntraPredictionRequest {
                        plane_id: plane,
                        x,
                        y,
                        block_size: size,
                        mode: IntraSmoothMode::Smooth,
                        available_left_samples: None,
                        available_above_samples: None,
                        num4_above_right: block_decoded.count_top_right_avail(
                            plane.index(),
                            x4,
                            y4,
                            w4,
                        ),
                        num4_below_left: block_decoded.count_bottom_left_avail(
                            plane.index(),
                            x4,
                            y4,
                            h4,
                        ),
                        bit_depth,
                    },
                )
                .map_err(|_| geometry_error())?;
            }
        }
        planes.push(InterIntraPlanePrediction {
            plane,
            sub_x,
            sub_y,
            x,
            y,
            size,
            samples,
        });
    }
    Ok(planes)
}

mod compound_path;
mod filter_records;
mod prediction;
mod residual;
mod syntax;
mod temporal;
pub(super) mod tip;
mod warp;

use self::filter_records::record_inter_deblock_geometry;
use self::prediction::placed_inter_geometry;
pub(crate) use self::syntax::interp_filter_no_neighbour_ctx;
use self::syntax::{
    effective_force_integer_mv, frame_mv_precision, interp_filter_symbol, lowered_pred_mv,
    read_block_mv_precision_syntax, read_drl_idx, read_drl_idx_from, read_skip_drl_idx,
    read_skip_mode_syntax, read_tip_drl_idx, read_use_amvd_syntax, resolve_interp_filter,
};
use self::temporal::{block_ref_within_temporal_distance, record_temporal_motion_block};
#[cfg(test)]
pub(super) use self::tip::tip_allowed_for_block_indices;
use self::warp::{
    WarpInterIntraSyntax, inter_mv_read_config, inter_mvd_sign_derivation_allowed,
    interintra_prediction_mode, local_warp_estimation, read_warp_extend_syntax,
    read_warp_inter_intra_syntax, read_warp_inter_mode_syntax, read_warp_newmv_delta_syntax,
    read_warp_newmv_motion_mode_syntax, read_warpmv_delta_syntax, read_wedge_mode_syntax,
};

use self::residual::{
    InterResidualLumaTxSizeMode, read_inter_residual, reset_inter_skip_coeff_contexts,
    transform_tool_residual_policy,
};

fn read_inter_intra_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    b_size: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<WarpInterIntraSyntax> {
    let frame_enables_interintra = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .is_some_and(|modes| modes[splot_core::headers::frame::INTERINTRA]);
    read_inter_intra_syntax_enabled(
        cdfs,
        symbols,
        frame_enables_interintra,
        b_size,
        n4w,
        n4h,
        tile_offset,
    )
}

fn read_inter_intra_syntax_enabled(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_enables_interintra: bool,
    b_size: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<WarpInterIntraSyntax> {
    if !frame_enables_interintra || b_size < BLOCK_8X8 || n4w.max(n4h) > CHUNK_64_N4 {
        return Ok(WarpInterIntraSyntax::default());
    }
    let bsize_group = *SIZE_GROUP_LOOKUP.get(b_size).ok_or_else(|| {
        inter_cap!(
            "inter_interintra_bsize_group",
            tile_offset,
            "inter.inter_intra block size out of range",
            SPEC_MODE_INFO
        )
    })?;
    let inter_intra = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntra { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if inter_intra.get() > 1 {
        return Err(inter_cap!(
            "inter_interintra_symbol",
            tile_offset,
            "inter.inter_intra symbol out of range",
            "5.20.7.15"
        ));
    }
    if inter_intra.get() == 0 {
        return Ok(WarpInterIntraSyntax::default());
    }

    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntraMode { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if mode >= INTERINTRA_MODES {
        return Err(inter_cap!(
            "inter_interintra_mode_symbol",
            tile_offset,
            "inter.interintra_mode symbol out of range",
            "5.20.7.15"
        ));
    }

    let use_wedge = if WEDGE_USED_BY_BSIZE.get(b_size).copied().unwrap_or(false) {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeInterIntra, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol > 1 {
            return Err(inter_cap!(
                "inter_simple_interintra_wedge_symbol",
                tile_offset,
                "inter.use_wedge_interintra symbol out of range",
                "5.20.7.15"
            ));
        }
        symbol != 0
    } else {
        false
    };
    let wedge_index = if use_wedge {
        Some(read_wedge_mode_syntax(cdfs, symbols, tile_offset)?)
    } else {
        None
    };

    Ok(WarpInterIntraSyntax {
        enabled: true,
        mode: Some(mode),
        use_wedge,
        wedge_index,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BawpParseInput {
    allow_bawp: bool,
    frame_is_switch: bool,
    single_mode: u8,
    use_amvd: bool,
    n4w: usize,
    n4h: usize,
    has_chroma: bool,
}

fn read_bawp_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: BawpParseInput,
    tile_offset: ByteOffset,
) -> Result<BawpSyntax> {
    if !input.allow_bawp
        || input.frame_is_switch
        || input.single_mode == SINGLE_MODE_GLOBALMV
        || input.n4w < 2
        || input.n4h < 2
    {
        return Ok(BawpSyntax::default());
    }

    let use_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseBawp, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if use_bawp.get() == 0 {
        return Ok(BawpSyntax::default());
    }

    let list_index = explicit_bawp_context(input.single_mode, input.use_amvd);
    let explicit_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::ExplicitBawp { ctx: list_index }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    let explicit = explicit_bawp.get() != 0;
    let explicit_scale_positive = if explicit {
        cdfs.read_block_symbol_trace(TileCdfSelector::ExplicitBawpScale, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            != 0
    } else {
        false
    };
    let chroma = if input.has_chroma {
        cdfs.read_block_symbol_trace(TileCdfSelector::UseBawpChroma, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            != 0
    } else {
        false
    };

    Ok(BawpSyntax {
        enabled: true,
        explicit,
        explicit_scale_positive,
        list_index: list_index as u8,
        ref_dist_gt4: false,
        chroma,
    })
}

fn explicit_bawp_context(single_mode: u8, use_amvd: bool) -> usize {
    if single_mode == SINGLE_MODE_NEARMV {
        0
    } else if single_mode == SINGLE_MODE_NEWMV && use_amvd {
        1
    } else {
        2
    }
}

fn single_inter_needs_interp_filter(n4w: usize, n4h: usize, single_mode: u8) -> bool {
    !(n4w >= 2 && n4h >= 2 && single_mode == SINGLE_MODE_GLOBALMV)
}

fn symbol_read_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_missing!(
        "inter_block_mode_parse",
        tile_offset,
        "inter.block.mode_info_symbols",
        SPEC_MODE_INFO
    )
}

fn ensure_intra_leaf_quantizer_delta_scope(
    frame_is_intra: bool,
    residual_quantizer_deltas_are_zero: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    if !frame_is_intra && !residual_quantizer_deltas_are_zero {
        return Err(inter_cap!(
            "inter_block_intra_leaf_nonzero_quantizer_delta",
            tile_offset,
            "inter.residual.nonzero_quantizer_delta",
            SPEC_MODE_INFO
        ));
    }
    Ok(())
}

fn map_inter_multiblock_error(
    error: GeneralIntraMultiblockError<crate::error::DecodeError>,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    match error {
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => crate::error::DecodeError::Limit { source },
        _ => inter_cap!(
            "inter_partition_walk",
            tile_offset,
            "inter.partition_walk",
            SPEC_MODE_INFO
        ),
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
