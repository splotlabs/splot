// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::Arc;

use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameType, GlobalMotionRef, GmType, MvPrecision,
    TipFrameMode, TxMode, get_qindex,
};
use splot_core::headers::sequence::{ChromaFormatIdc, DrlReorder, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, TX_HEIGHT, TX_HEIGHT_LOG2, TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::PlaneId as ReconPlaneId;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IDENTITY_WARP_PARAMS, InterIntraMode,
    InterpolationFilter as ReconInterpolationFilter, ReconSample,
};

use super::find_mv_stack::{
    BlockNeighbourContext, BlockPrecisionRecord, DEFAULT_WARP_PARAMS, MotionMode, MvBlockContext,
    NON_INTER_FLAG_SYNTAX, NeighbourFlagSyntax, NeighbourMvGrid, TIP_REF_FRAME,
    TemporalMotionBlock, TemporalMotionField, TemporalMvContext, TemporalProjectionConfig,
    TipReferencePair, ZERO_NEIGHBOUR_MOTION_VALUES, block_neighbour_ctx,
    find_compound_mv_stack_with_temporal, find_mode_ctx, find_mode_ctx_with_tip,
    find_mv_stack_with_temporal, warp_predicted_mv,
};
use super::read_mv::{
    MV_PRECISION_EIGHTH_PEL, MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL,
    MvReadConfig, apply_inter_mvd_signs, mv_clamp_to_integer, read_newmv_amvd_block_mvd,
    read_newmv_block_mvd_magnitude_with_config as read_newmv_block_mvd_magnitude,
};
use super::{
    BawpSyntax, InterBlock, InterIntraPrediction, InterReferenceState, InterResidual,
    InterResidualBlock, Mv, PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, SPEC_MODE_INFO, mc, unsupported_at, unsupported_compound_at,
};
use crate::bitstream::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, BlockSize, CoeffContextReset,
    DecodeBlockFrontier, DecodeTileWorkUnit, DecodedLeafPublication, FrameCdfSubset,
    FrameQmSegmentScope, FrameQuantizerSnapshot, GeneralIntraLeafMode,
    GeneralIntraMultiblockCursor, GeneralIntraMultiblockError, GeneralIntraTreeWalkError,
    IsCflContext, LumaCoeffBlock, SavedCdfSubset, TileBlockDecodedState, TileCdfSelector,
    TileCdfSubset, TileCoeffContextState, TileFscModeState, TileIntraJointModeState,
    TilePartitionTraversalError, TileSegmentIdState, TileUsesMrlsState,
    TransformToolResidualPolicy, chroma_subsampling, current_frame_qm_segment_id,
    decode_general_intra_plane_coeffs, frame_mi_dimensions, get_plane_residual_size,
    is_cctx_geometry_allowed, neg_deinterleave, read_lossless_tx_size,
};
use crate::filters::wienerns_lr::intrabc_records::{
    IntrabcBlockGeometry, IntrabcBlockPrelude, IntrabcUseSkip, TileIntrabcPreludeState,
    read_intrabc_info, read_intrabc_use_and_skip,
};
use crate::filters::wienerns_lr::tx_records::{
    CdefState, DeltaQState, SelectableLumaTxRecord, ccso::CcsoState,
    derive_inter_luma_tx_records_for_block, gdf::GdfState,
};
use crate::pipeline::effective_allow_screen_content_tools;
use crate::{DecodeOptions, Result};

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

fn global_motion_model(core: &FrameHeaderCore, logical_ref: i8) -> GlobalMotionRef {
    usize::try_from(logical_ref)
        .ok()
        .and_then(|index| {
            core.inter_tail
                .as_ref()?
                .global_motion
                .references
                .get(index)
        })
        .copied()
        .unwrap_or_else(GlobalMotionRef::identity)
}

fn global_motion_mv(
    core: &FrameHeaderCore,
    logical_ref: i8,
    block: &MvBlockContext,
    precision: u8,
) -> Mv {
    warp_predicted_mv(
        global_motion_model(core, logical_ref).gm_params,
        block,
        precision,
    )
}

fn global_motion_warp(
    core: &FrameHeaderCore,
    logical_ref: i8,
    force_integer_mv: bool,
    n4w: usize,
    n4h: usize,
) -> Option<[i32; 6]> {
    let model = global_motion_model(core, logical_ref);
    (!force_integer_mv && n4w >= 2 && n4h >= 2 && model.gm_type != GmType::Identity)
        .then_some(model.gm_params)
}
pub(crate) const WARP_PARAM_REDUCE_BITS: u32 = 6;
const WARP_TRANS_INTEGER_BITS: u32 = 12;
const WARP_DELTA_STEP_BITS: u32 = 10;
pub(crate) const WARPEDMODEL_TRANS_CLAMP: i32 =
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
    pub(crate) records: crate::filters::wienerns_lr::FrameFilterRecords,
    pub(crate) cdef_grid: crate::filters::cdef::CdefUnitGrid,
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    motion_field: TemporalMotionField,
}

impl InterFilterInputs {
    /// Takes the walk-derived temporal motion field, leaving an empty one.
    pub(crate) fn take_motion_field(&mut self) -> TemporalMotionField {
        core::mem::replace(&mut self.motion_field, TemporalMotionField::empty())
    }
}

#[derive(Default)]
pub(crate) struct InterDecodeScratch<T: ReconSample> {
    tile: tile::TileDecodeScratch<T>,
    temporal_context: Option<TemporalMvContext>,
    frame_filter_records: crate::filters::wienerns_lr::FrameFilterRecords,
}

impl<T: ReconSample> InterDecodeScratch<T> {
    pub(crate) fn recycle_frame_filter_records(
        &mut self,
        records: crate::filters::wienerns_lr::FrameFilterRecords,
    ) {
        self.frame_filter_records = records;
    }

    #[cfg(test)]
    pub(crate) fn frame_filter_records_capacity(&self) -> usize {
        self.frame_filter_records.deblock_blocks.capacity()
    }
}

enum ReconCommand {
    GeneralIntra(crate::pipeline::general_intra::GeneralIntraReconCommand),
    Intrabc(intrabc::IntrabcReconCommand),
    Inter(deferred_recon::InterReconCommand),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ReconDependency {
    ReferenceOnly,
    CurrentFrame,
    GlobalIntrabcFence,
}

impl ReconCommand {
    fn dependency(&self) -> ReconDependency {
        match self {
            Self::Intrabc(command) if command.requires_global_fence() => {
                ReconDependency::GlobalIntrabcFence
            }
            Self::GeneralIntra(_) | Self::Intrabc(_) => ReconDependency::CurrentFrame,
            Self::Inter(command) if command.reads_current_frame() => ReconDependency::CurrentFrame,
            Self::Inter(_) => ReconDependency::ReferenceOnly,
        }
    }

    fn temporal_record_capacity(&self) -> usize {
        match self {
            Self::Inter(command) => command.temporal_record_capacity(),
            Self::GeneralIntra(_) | Self::Intrabc(_) => 0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_inter_blocks<T: ReconSample>(
    scratch: &mut InterDecodeScratch<T>,
    mut tile_plan: crate::bitstream::tile_payload::DecodeTilePayloadPlan<'_>,
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<(Arc<FrameCdfSubset>, InterFilterInputs)> {
    let offset = frame_envelope.offset;
    let frame_is_intra = core.frame_is_intra == Some(true);
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
    let frame_cdfs = first_tile.frame_cdfs();
    let mut saved_cdfs: Option<SavedCdfSubset> = None;

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
    let current_order_hint = core.display_order_hint().unwrap_or(0);
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
    let derived_order_hints = super::find_mv_stack::reference_order_hints(
        ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
    );
    let expected_tip_pair = derived_tip_reference_pair(core, &derived_order_hints);
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
        &derived_order_hints,
    );
    let cdef_state = CdefState::new(mi_rows, mi_cols, sequence, first_tile_offset)?;
    let gdf_state = GdfState::new(mi_rows, mi_cols, sequence, core, first_tile_offset)?;
    let ccso_state = CcsoState::new(
        mi_rows,
        mi_cols,
        sequence,
        core,
        ref_frame_idx,
        &reference.ref_ccso_unit_grids,
        first_tile_offset,
    )?;
    let residual_tool_policy = if frame_is_intra {
        crate::pipeline::general_intra::general_intra_transform_tool_residual_policy(sequence)
    } else {
        transform_tool_residual_policy(sequence)
    };
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

    let params = tile::TileWalkParams {
        limits: options.limits(),
        mi_rows,
        mi_cols,
        sb_h4,
        max_drl_bits_minus_1,
        frame_interpolation_filter,
        residual_tool_policy,
        num_total_refs,
        reference_select,
        num_same_ref_compound,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        enable_adaptive_mvd,
        allow_bawp,
        allow_warpmv_mode,
        frame_is_switch,
        current_order_hint,
        tip_ref_pair: expected_tip_pair.map(|pair| (pair.past_ref, pair.future_ref)),
    };
    let mut records = core::mem::take(&mut scratch.frame_filter_records);
    let temporal_context = frame_temporal_context(
        scratch
            .temporal_context
            .get_or_insert_with(TemporalMvContext::empty),
        core,
        sb_h4,
        (mi_rows, mi_cols),
        current_order_hint,
        temporal_config,
        ref_frame_idx,
        reference,
        expected_tip_pair,
        offset,
    )?;
    let walked = tile::decode_tiles(
        &mut scratch.tile,
        &mut records,
        work_units,
        &params,
        sequence,
        core,
        temporal_context,
        reference,
        ref_frame_idx,
        workspace,
        cdef_state,
        gdf_state,
        ccso_state,
        motion_field,
    )?;
    for tile in work_units.iter() {
        SavedCdfSubset::apply_completed_tile(
            &mut saved_cdfs,
            frame_cdfs.as_ref(),
            tile.tile_num(),
            tile.cdf().tile_cdfs(),
            tile.cdf().save_policy(),
        );
    }
    let mut frame_cdfs = FrameCdfSubset::frame_end_updated(frame_cdfs.as_ref(), saved_cdfs);
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(qindex)
        .map_err(|_| {
            inter_cap!(
                "reference_coefficient_cdf_context",
                offset,
                "inter.cdf.reference_coefficient_context",
                "7.23"
            )
        })?;
    let frame_cdfs = Arc::new(frame_cdfs);
    let tile::TileDecodeOutput {
        cdef_state,
        gdf_state,
        ccso_state,
        motion_field,
    } = walked;
    let filter_inputs = InterFilterInputs {
        records,
        cdef_grid: cdef_state.into_grid(first_tile_offset)?,
        ccso_grid: ccso_state.into_grid(first_tile_offset)?,
        gdf_grid: gdf_state.into_grid(first_tile_offset)?,
        motion_field,
    };
    Ok((frame_cdfs, filter_inputs))
}

/// Runs the AV2 § 7.9 temporal prelude: reference motion-field projection and
/// the § 7.11.3 TIP motion field.
///
/// The entropy pass reads the resulting TIP reference pair, and the driver
/// derived that pair from the reference order hints alone before the prelude
/// ran, so a disagreement means the prelude could not build the field the parse
/// pass assumed and the frame fails closed.
#[allow(clippy::too_many_arguments)]
fn frame_temporal_context<'a, T: ReconSample>(
    temporal_context: &'a mut TemporalMvContext,
    core: &FrameHeaderCore,
    sb_h4: usize,
    dimensions: (usize, usize),
    current_order_hint: u32,
    temporal_config: TemporalProjectionConfig,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    expected_tip_pair: Option<TipReferencePair>,
    offset: ByteOffset,
) -> Result<&'a mut TemporalMvContext> {
    let temporal_timer = crate::timing::start();
    temporal_context
        .refresh_from_references(
            dimensions,
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
    crate::timing::report("inter_temporal_refresh", temporal_timer);
    let tip_prepare_timer = crate::timing::start();
    tip::prepare_motion_field(temporal_context, core, sb_h4);
    crate::timing::report("inter_tip_prepare", tip_prepare_timer);
    if temporal_context.tip_references() != expected_tip_pair {
        return Err(inter_cap!(
            "inter_tip_reference_pair_mismatch",
            offset,
            "inter.temporal_motion_context",
            SPEC_MODE_INFO
        ));
    }
    Ok(temporal_context)
}
/// AV2 § 7.12.2 TIP reference pair as the entropy pass sees it.
///
/// [`tip::prepare_motion_field`] leaves the projected field's pair set exactly
/// when the frame enables TIP and the reference order hints admit a pair, and
/// that derivation reads no motion field, so the driver can settle it before
/// the temporal prelude runs. `decode_inter_blocks` fails closed if the two
/// ever disagree.
fn derived_tip_reference_pair(
    core: &FrameHeaderCore,
    ref_order_hints: &[Option<u32>],
) -> Option<super::find_mv_stack::TipReferencePair> {
    let inter = core.inter.as_ref()?;
    if inter.tip_frame_mode == Some(TipFrameMode::Disabled) {
        return None;
    }
    super::find_mv_stack::tip_reference_pair_from_hints(
        core.display_order_hint().unwrap_or(0),
        ref_order_hints,
    )
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

fn chroma_smooth_tile_ranges(
    mi_rows: core::ops::Range<usize>,
    mi_cols: core::ops::Range<usize>,
    chroma: splot_core::headers::sequence::ChromaFormatIdc,
) -> (core::ops::Range<usize>, core::ops::Range<usize>) {
    let (sub_x, sub_y) = chroma_subsampling(chroma);
    (
        (mi_rows.start >> usize::from(sub_y))..if sub_y {
            mi_rows.end.div_ceil(2)
        } else {
            mi_rows.end
        },
        (mi_cols.start >> usize::from(sub_x))..if sub_x {
            mi_cols.end.div_ceil(2)
        } else {
            mi_cols.end
        },
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
    let coded = i32::from(raw) + if ext { 8 } else { 0 };
    let segment_id = neg_deinterleave(
        coded,
        i32::from(pred),
        i32::from(seg.last_active_seg_id) + 1,
    );
    validate_segment_id(segment_id, seg.last_active_seg_id, tile_offset)
}

fn validate_segment_id(segment_id: i32, last_active: u8, tile_offset: ByteOffset) -> Result<u8> {
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

/// AV2 § 6.4 `drl_reorder` mode this sequence signals.
pub(super) fn sequence_drl_reorder(sequence: &SequenceHeader) -> DrlReorder {
    sequence
        .inter
        .as_ref()
        .map_or(DrlReorder::Disabled, |inter| inter.drl_reorder)
}

/// Whether AV2 § 6.8 `use_ref_frame_mvs` admits temporal § 7.12 candidates.
pub(super) fn frame_uses_temporal_mvs(core: &FrameHeaderCore) -> bool {
    core.inter
        .as_ref()
        .and_then(|inter| inter.use_ref_frame_mvs)
        == Some(true)
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
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    gdf_state: &mut GdfState,
    cdef_state: &mut CdefState,
    ccso_state: &mut CcsoState,
    delta_q_state: &mut DeltaQState,
    intrabc_state: &mut TileIntrabcPreludeState,
    segment_id_state: &mut TileSegmentIdState,
    mv_grid: &mut NeighbourMvGrid,
    tip_ref_pair: Option<(i8, i8)>,
    y_smooth: &mut crate::prediction::intra_edge::TileYSmoothGrid,
    chroma_smooth: &mut crate::prediction::intra_edge::TileChromaSmoothGrid,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    use_dip: &crate::bitstream::tile_payload::TileUseDipState,
    fsc_modes: &TileFscModeState,
    palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, ParsedLeaf)> {
    let _block_phase = crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::Block);
    crate::timing::note_walk_block();
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

    let comp_ref_allowed = is_comp_ref_allowed(n4w, n4h);
    let drl_reorder = sequence_drl_reorder(sequence);
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
    let use_temporal = frame_uses_temporal_mvs(core);
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

    let is_inter = if leaf_uses_general_intra(
        core.frame_is_intra == Some(true),
        frontier.is_luma_part(),
        frontier.is_chroma_part(),
    ) {
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
            gdf_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
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
            delta_q_state.read_for_block(
                work_unit,
                symbols,
                frontier,
                use_skip.skip_flag,
                tile_offset,
            )?;
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
            let prediction = intrabc::IntrabcReconPrediction::derive(
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
                    residual_scratch,
                    residual_blocks,
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
                residual_blocks,
                None,
                block_qindex,
                lossless,
                tile_offset,
            )?;
            let command = intrabc::IntrabcReconCommand::new(
                prediction,
                residual,
                segment_id,
                block_qindex,
                luma_use_tcq,
                residual_use_ddt,
                bit_depth,
                tile_offset,
            );
            let precision = frame_mv_precision(core, tile_offset)?;
            mv_grid.record_flags(
                mi_row,
                mi_col,
                n4w,
                n4h,
                NeighbourFlagSyntax {
                    is_inter: true,
                    skip: prelude.skip_flag,
                    interp_filter: interp_filter_no_neighbour_ctx(false) as u8,
                    precision: BlockPrecisionRecord::explicit(precision),
                    ..NON_INTER_FLAG_SYNTAX
                },
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            return Ok((
                non_intra_leaf_mode(frontier).mark_intrabc(),
                ParsedLeaf::recon(
                    ReconCommand::Intrabc(command),
                    non_inter_leaf_motion(mi_row, mi_col, n4w, n4h),
                ),
            ));
        }
        let block_qindex = segment_block_qindex(
            sequence,
            core,
            usize::from(segment_id),
            delta_q_state.qindex_u32(),
        );
        let _intra_phase = crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::IntraLeaf);
        let (leaf, command) = crate::pipeline::general_intra::decode_one_general_intra_block(
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
        let mut motion = LeafMotion {
            mi_row,
            mi_col,
            kind: LeafMotionKind::Reseed,
        };
        if !frontier.is_chroma_part() {
            y_smooth.record(mi_row, mi_col, n4w, n4h, leaf.y_mode_is_smooth());
            mv_grid.record_flags(
                mi_row,
                mi_col,
                n4w,
                n4h,
                NeighbourFlagSyntax {
                    interp_filter: interp_filter_no_neighbour_ctx(false) as u8,
                    precision: BlockPrecisionRecord::explicit(frame_mv_precision(
                        core,
                        tile_offset,
                    )?),
                    ..NON_INTER_FLAG_SYNTAX
                },
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            motion = non_inter_leaf_motion(mi_row, mi_col, n4w, n4h);
        }
        return Ok((
            leaf,
            ParsedLeaf::recon(ReconCommand::GeneralIntra(command), motion),
        ));
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

    gdf_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
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
    delta_q_state.read_for_block(work_unit, symbols, frontier, skip == 1, tile_offset)?;
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
            residual_scratch,
            residual_blocks,
            sequence,
            core,
            frontier,
            mv_grid,
            &mut block_ctx,
            &neighbour_ctx,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            intrabc_state,
            ref_frame_idx,
            reference,
            num_total_refs,
            skip,
            mi_rows,
            mi_cols,
            max_drl_bits_minus_1,
            residual_tool_policy,
            block_qindex,
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
            residual_scratch,
            residual_blocks,
            sequence,
            core,
            frontier,
            mv_grid,
            tip_ref_pair,
            &mut block_ctx,
            &neighbour_ctx,
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
            mi_rows,
            mi_cols,
            max_drl_bits_minus_1,
            temporal_first_frame,
            enable_adaptive_mvd,
            residual_tool_policy,
            block_qindex,
            frame_interpolation_filter,
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
                    motion_mode,
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
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?,
            (WarpInterMode::Warpmv, _) => {
                read_warpmv_delta_syntax(cdfs, symbols, mv_config, tile_offset)?
            }
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
            residual_blocks,
            None,
            block_qindex,
            current_residual_lossless(work_unit),
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
            )
            .mark_inter(),
            tile_offset,
        )?;
        let syntax = InterBlockSyntax {
            block_ctx,
            motion: InterMotionSyntax::Warp(WarpMotionSyntax {
                source: warp.source,
                ref_mv_idx: warp.ref_mv_idx,
                ref_warp_idx: warp.ref_warp_idx,
                mvd: warp.mvd,
                extend_delta: mode_ctx.extend_delta,
                derive_wrl,
                use_temporal_first,
            }),
            interp: ReconInterpolationFilter::EightTap,
            precision: warp.precision,
            skip: skip == 1,
            use_amvd: false,
            tip_size_16x16: false,
            blend: mc::CompoundBlend::default(),
            bawp: BawpSyntax::default(),
            interintra: warp_interintra_mode,
            optflow_distances: None,
            residual,
        };
        mv_grid.record_flags(mi_row, mi_col, n4w, n4h, syntax.flag_syntax());
        return Ok((
            non_intra_leaf_mode(frontier),
            pending_inter_leaf(
                syntax,
                placed_geometry,
                PendingInterKind::Single,
                block_qindex,
                frame_mv_precision(core, tile_offset)?,
            ),
        ));
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
    let reference_scaled = if tip_ref {
        false
    } else {
        block_reference_is_scaled(core, reference, ref_frame_idx, ref_frame0, tile_offset)?
    };
    let bawp = if tip_ref {
        BawpSyntax::default()
    } else {
        read_bawp_syntax(
            cdfs,
            symbols,
            BawpParseInput {
                allow_bawp,
                reference_scaled,
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

    let mvd = if single_mode == SINGLE_MODE_NEWMV {
        let config = MvReadConfig::inter(precision.mv_precision);
        if use_amvd {
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
        }
    } else {
        Mv::ZERO
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
        residual_blocks,
        tip_ref.then_some(crate::filters::deblock::DeblockSubPuSize::square(
            if tip_uses_16x16_units { 16 } else { 8 },
        )),
        block_qindex,
        current_residual_lossless(work_unit),
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
        )
        .mark_inter(),
        tile_offset,
    )?;
    let syntax = InterBlockSyntax {
        block_ctx,
        motion: InterMotionSyntax::Single(SingleMotionSyntax {
            mode: single_mode,
            tip_ref,
            ref_mv_idx,
            mvd,
            use_temporal_first,
            global_warp: (!tip_ref && single_mode == SINGLE_MODE_GLOBALMV)
                .then(|| global_motion_warp(core, ref_frame0, force_integer_mv, n4w, n4h))
                .flatten(),
        }),
        interp,
        precision,
        skip: skip == 1,
        use_amvd,
        tip_size_16x16: tip_ref && tip_uses_16x16_units,
        blend: mc::CompoundBlend::default(),
        bawp,
        interintra,
        optflow_distances: None,
        residual,
    };
    mv_grid.record_flags(mi_row, mi_col, n4w, n4h, syntax.flag_syntax());
    let kind = if tip_ref {
        PendingInterKind::Tip
    } else {
        PendingInterKind::Single
    };
    Ok((
        non_intra_leaf_mode(frontier),
        pending_inter_leaf(
            syntax,
            placed_geometry,
            kind,
            block_qindex,
            frame_mv_precision(core, tile_offset)?,
        ),
    ))
}

/// § 7.12 work of a leaf that publishes a zero motion record.
const fn non_inter_leaf_motion(mi_row: usize, mi_col: usize, n4w: usize, n4h: usize) -> LeafMotion {
    LeafMotion {
        mi_row,
        mi_col,
        kind: LeafMotionKind::NonInter { n4w, n4h },
    }
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

pub(crate) fn is_comp_ref_allowed(n4w: usize, n4h: usize) -> bool {
    n4w.min(n4h) >= 2 || (n4w == 1 && n4h >= 4) || (n4h == 1 && n4w >= 4)
}

const fn leaf_uses_general_intra(
    frame_is_intra: bool,
    is_luma_part: bool,
    is_chroma_part: bool,
) -> bool {
    frame_is_intra || is_luma_part || is_chroma_part
}

fn sequence_enables_ibp(sequence: &SequenceHeader) -> bool {
    sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp)
}

mod compound_path;
mod deferred_recon;
mod filter_records;
mod interintra;
mod intrabc;
mod pixel_commit;
mod prediction;
mod residual;
mod resolve;
mod row_gate;
mod syntax;
mod temporal;
mod tile;
pub(super) mod tip;
mod warp;

use self::filter_records::record_inter_deblock_geometry;
use self::interintra::predict_interintra_planes;
use self::prediction::placed_inter_geometry;
use self::resolve::{
    CompoundJointMvProjection, CompoundMotionSyntax, InterBlockSyntax, InterMotionSyntax,
    LeafMotion, LeafMotionKind, MvResolutionState, ParsedLeaf, PendingInterBlock, PendingInterKind,
    SingleMotionSyntax, WarpDeltaSyntax, WarpModelSource, WarpMotionSyntax, pending_inter_leaf,
    resolve_parsed_leaves,
};
pub(crate) use self::syntax::interp_filter_no_neighbour_ctx;
use self::syntax::{
    effective_force_integer_mv, frame_mv_precision, interp_filter_symbol, lowered_pred_mv,
    read_block_mv_precision_syntax, read_drl_idx, read_drl_idx_from, read_skip_drl_idx,
    read_skip_mode_syntax, read_tip_drl_idx, read_use_amvd_syntax, resolve_interp_filter,
};
use self::temporal::block_ref_within_temporal_distance;
#[cfg(test)]
pub(super) use self::tip::tip_allowed_for_block_indices;
use self::warp::{
    WarpInterIntraSyntax, inter_mv_read_config, inter_mvd_sign_derivation_allowed,
    interintra_prediction_mode, local_warp_estimation, read_warp_extend_syntax,
    read_warp_inter_intra_syntax, read_warp_inter_mode_syntax, read_warp_newmv_delta_syntax,
    read_warp_newmv_motion_mode_syntax, read_warpmv_delta_syntax, read_wedge_mode_syntax,
};

use self::residual::{
    InterResidualLumaTxSizeMode, InterResidualParseScratch, read_inter_residual,
    reset_inter_skip_coeff_contexts, transform_tool_residual_policy,
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
    reference_scaled: bool,
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
        || input.reference_scaled
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

fn block_reference_is_scaled<T: ReconSample>(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "inter_block_missing_frame_size",
            tile_offset,
            "inter.frame_size",
            SPEC_MODE_INFO
        )
    })?;
    let slot = usize::try_from(ref_frame)
        .ok()
        .and_then(|index| ref_frame_idx.get(index))
        .and_then(|&slot| usize::try_from(slot).ok())
        .ok_or_else(|| {
            inter_missing!(
                "inter_block_missing_reference_slot",
                tile_offset,
                "inter.reference_slot",
                SPEC_MODE_INFO
            )
        })?;
    let width = reference
        .ref_frame_width
        .get(slot)
        .copied()
        .ok_or_else(|| {
            inter_missing!(
                "inter_block_missing_reference_width",
                tile_offset,
                "inter.reference_width",
                SPEC_MODE_INFO
            )
        })?;
    let height = reference
        .ref_frame_height
        .get(slot)
        .copied()
        .ok_or_else(|| {
            inter_missing!(
                "inter_block_missing_reference_height",
                tile_offset,
                "inter.reference_height",
                SPEC_MODE_INFO
            )
        })?;
    Ok(super::mv_scaling::reference_is_scaled(
        width as i32,
        height as i32,
        frame_size.width as i32,
        frame_size.height as i32,
    ))
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
