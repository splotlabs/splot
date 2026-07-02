// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, QuantizationParams, TipFrameMode,
    parse_frame_header_core,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, DecodedFrame, InterpolationFilter as ReconInterpolationFilter,
    PlaneId as ReconPlaneId, ReconSample, ReferenceFrameStore, ReferenceSlot,
};

use super::{
    DecodeOptions, DecodePlannedObu, DecodeStreamPlan, IvfHeader, Result, ensure_runtime_limits,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature};
use crate::tile_payload::FrameCdfSubset;
const FEATURE_ID: &str = "DECODE-FIRST-INTER-FRAME-FRONTIER";
const MATRIX_ROW: &str = "first-inter-frame-frontier";
const TIER_ID: &str = "general-inter-8bit420-frontier-v1";
const REMEDIATION: &str = "Inter decode is limited to the current 8-bit 4:2:0 capability set.";
const COMPOUND_FEATURE_ID: &str = "DECODE-INTER-COMPOUND-AVERAGE";
const COMPOUND_MATRIX_ROW: &str = "inter-compound-average";
const COMPOUND_REMEDIATION: &str =
    "Compound inter decode is limited to two-reference COMPOUND_AVERAGE.";

macro_rules! inter_cap {
    ($reason:literal, $offset:expr, $capability:literal, $spec_section:expr $(,)?) => {
        unsupported_at(
            $reason,
            $offset,
            concat!("unsupported capability: ", $capability),
            $spec_section,
        )
    };
}

macro_rules! inter_missing {
    ($reason:literal, $offset:expr, $input:literal, $spec_section:expr $(,)?) => {
        unsupported_at(
            $reason,
            $offset,
            concat!("missing required input: ", $input),
            $spec_section,
        )
    };
}

macro_rules! inter_diag {
    ($reason:literal, $offset:expr, $message:literal, $spec_section:expr $(,)?) => {
        unsupported_at($reason, $offset, $message, $spec_section)
    };
}

macro_rules! compound_cap {
    ($reason:literal, $offset:expr, $capability:literal, $spec_section:expr $(,)?) => {
        unsupported_compound_at(
            $reason,
            $offset,
            concat!("unsupported capability: ", $capability),
            $spec_section,
        )
    };
}

macro_rules! compound_missing {
    ($reason:literal, $offset:expr, $input:literal, $spec_section:expr $(,)?) => {
        unsupported_compound_at(
            $reason,
            $offset,
            concat!("missing required input: ", $input),
            $spec_section,
        )
    };
}

const SPEC_HEADER: &str = "5.18.2";
const SPEC_MODE_INFO: &str = "5.20.7.6";
const SPEC_MV: &str = "7.11";
const SPEC_MC: &str = "7.13.3.18";
const SPEC_REFERENCE: &str = "7.23";
const SINGLE_MODE_NEARMV: u8 = 0;
const SINGLE_MODE_GLOBALMV: u8 = 1;
const SINGLE_MODE_NEWMV: u8 = 2;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Mv {
    pub(super) row: i32,
    pub(super) col: i32,
}

impl Mv {
    const ZERO: Self = Self { row: 0, col: 0 };
}
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_minimal_inter_frame<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    _header: IvfHeader,
    reference: &InterReferenceState<'_, T>,
    bit_depth: BitDepth,
) -> Result<(DecodedFrame<T>, FrameHeaderCore, FrameCdfSubset)> {
    let offset = frame_envelope.offset;

    if frame_envelope.header.obu_type != ObuType::RegularTileGroup {
        return Err(inter_cap!(
            "inter_unexpected_obu_type",
            offset,
            "inter.obu_type != regular_tile_group",
            SPEC_HEADER
        ));
    }

    let current_order_hint = i32::try_from(core.order_hint_lsb.unwrap_or(0)).unwrap_or(i32::MAX);
    let initial_cdfs = resolve_initial_frame_cdfs(&core, sequence, reference, offset)?;

    let order_hint_bits = sequence
        .inter
        .as_ref()
        .map_or(0, |seq_inter| u32::from(seq_inter.order_hint_bits));
    let this_order_hint = core.order_hint_lsb.unwrap_or(0);
    if !order_hint_history_unwrapped(
        &reference.ref_valid,
        &reference.ref_order_hint,
        order_hint_bits,
        this_order_hint,
    ) {
        return Err(inter_cap!(
            "inter_order_hint_wrapped",
            offset,
            "inter.order_hint.wrapped_reference_history",
            SPEC_REFERENCE
        ));
    }

    let uses_temporal_mvs = core
        .inter
        .as_ref()
        .and_then(|inter| inter.use_ref_frame_mvs)
        == Some(true);
    let has_retained_inter_reference = reference.ref_is_inter.iter().any(|&is_inter| is_inter);
    if uses_temporal_mvs && has_retained_inter_reference {
        return Err(inter_cap!(
            "inter_temporal_mvs_unmodeled",
            offset,
            "inter.temporal_mvs.with_retained_inter_reference",
            SPEC_MV
        ));
    }

    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "inter_missing_frame_size",
            offset,
            "inter.frame_size",
            SPEC_HEADER
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    let inter = core.inter.as_ref().ok_or_else(|| {
        inter_missing!(
            "inter_missing_control_region",
            offset,
            "inter.control_region",
            SPEC_HEADER
        )
    })?;
    let tail = core
        .inter_tail
        .as_ref()
        .ok_or_else(|| inter_missing!("inter_missing_tail", offset, "inter.tail", SPEC_HEADER))?;
    let num_total_refs = inter.num_total_refs.unwrap_or(0);
    if num_total_refs != 1 && num_total_refs != 2 {
        return Err(inter_cap!(
            "inter_unsupported_num_total_refs",
            offset,
            "inter.single_ref.num_total_refs not in 1..=2",
            SPEC_MODE_INFO
        ));
    }
    let ref_frame_idx = inter.ref_frame_idx.clone();
    if ref_frame_idx.len() != num_total_refs as usize || ref_frame_idx.is_empty() {
        return Err(inter_missing!(
            "inter_missing_ref_frame_idx",
            offset,
            "inter.ref_frame_idx",
            SPEC_HEADER
        ));
    }

    let block_reference_select = tail.reference_select;
    let compound_is_joint_ctx = if block_reference_select && ref_frame_idx.len() == 2 {
        validate_compound_sequence_subset(sequence, &core, offset)?;
        Some(compound_is_joint_context(
            &ref_frame_idx,
            reference,
            current_order_hint,
            offset,
        )?)
    } else {
        None
    };
    if tail.use_global_motion {
        return Err(inter_cap!(
            "inter_use_global_motion",
            offset,
            "inter.global_motion",
            SPEC_MV
        ));
    }

    for &slot in &ref_frame_idx {
        let ref_frame = reference.frame_for_slot(slot).ok_or_else(|| {
            inter_missing!(
                "inter_missing_reference_frame",
                offset,
                "inter.reference_frame",
                SPEC_REFERENCE
            )
        })?;
        let ref_luma = ref_frame.y();
        if ref_luma.visible_size().width() != frame_width as usize
            || ref_luma.visible_size().height() != frame_height as usize
        {
            return Err(inter_cap!(
                "inter_reference_resolution_mismatch",
                offset,
                "inter.reference_scaling",
                SPEC_MC
            ));
        }
    }

    let limits = options.limits();
    let tile_size = {
        let mut tile_plan = super::derive_inter_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            &core,
            options,
            initial_cdfs.clone(),
        )?;
        let [tile] = tile_plan.work_units_mut() else {
            return Err(inter_cap!(
                "inter_unexpected_tile_work_units",
                offset,
                "inter.tile_count != 1",
                SPEC_HEADER
            ));
        };
        tile.tile_size()
    };
    ensure_runtime_limits(limits, frame_width, frame_height, tile_size, bit_depth)?;

    let interpolation_filter = inter.interpolation_filter.ok_or_else(|| {
        inter_missing!(
            "inter_missing_interpolation_filter",
            offset,
            "inter.interpolation_filter",
            SPEC_MC
        )
    })?;

    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace::<T>(
        frame_width as usize,
        frame_height as usize,
        bit_depth,
    )?;
    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            unsupported_at(
                "inter_missing_base_q",
                offset,
                "minimal inter residual decode requires a parsed base_q_idx",
                SPEC_HEADER,
            )
        })?;
    let luma_use_tcq = core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.allow_tcq);
    let residual_use_ddt = sequence
        .transform_quant_entropy
        .as_ref()
        .is_some_and(|tq| tq.enable_inter_ddt);

    let (frame_cdfs, filter_inputs) = decode_inter_blocks(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        &core,
        options,
        interpolation_filter,
        num_total_refs as usize,
        block_reference_select,
        compound_is_joint_ctx,
        sequence
            .inter
            .as_ref()
            .map_or(0, |seq_inter| seq_inter.num_same_ref_compound),
        &ref_frame_idx,
        reference,
        &mut workspace,
        qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        initial_cdfs,
    )?;

    let mut filter_sink = super::wienerns_lr::recon_final_filter_sink(
        workspace,
        frame_width as usize,
        frame_height as usize,
        bit_depth,
    );
    filter_sink.set_deblock_blocks(
        filter_inputs.deblock_blocks,
        filter_inputs.chroma_deblock_blocks,
    );
    filter_sink.set_cdef_grid(Some(filter_inputs.cdef_grid));
    filter_sink.set_ccso_grid(filter_inputs.ccso_grid);
    filter_sink.set_lr_source_blocks(filter_inputs.lr_source_blocks);
    filter_sink.set_lr_unit_filters(filter_inputs.lr_unit_filters);
    let frame = filter_sink.into_filtered_frame(
        &core,
        super::deblock_quant_deltas(sequence, &core),
        offset,
    )?;

    Ok((frame, core, frame_cdfs))
}

fn resolve_initial_frame_cdfs(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
    offset: ByteOffset,
) -> Result<FrameCdfSubset> {
    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint = i32::try_from(core.order_hint_lsb.unwrap_or(0)).unwrap_or(i32::MAX);
    let default_cdfs = || {
        FrameCdfSubset::default_for_base_q(current_base_q_idx).map_err(|_| {
            inter_cap!(
                "inter_cdf_default_init",
                offset,
                "inter.cdf.default_init",
                SPEC_HEADER
            )
        })
    };
    let Some(inter_ctrl) = core.inter.as_ref() else {
        return default_cdfs();
    };
    let (enable_avg_cdf, avg_cdf_type) = sequence
        .transform_quant_entropy
        .as_ref()
        .map_or((false, 1u8), |tq| (tq.enable_avg_cdf, tq.avg_cdf_type));
    let cdf_load = resolve_cdf_load(
        inter_ctrl.signal_primary_ref_frame,
        inter_ctrl.primary_ref_frame,
        inter_ctrl.disable_cross_frame_cdf_init,
        &inter_ctrl.ref_frame_idx,
        &reference.ref_is_inter,
        &reference.ref_base_q_idx,
        &reference.ref_order_hint,
        &reference.ref_frame_width,
        &reference.ref_frame_height,
        current_base_q_idx,
        current_order_hint,
        enable_avg_cdf,
        avg_cdf_type,
    );
    trace_initial_frame_cdfs(
        current_base_q_idx,
        current_order_hint,
        inter_ctrl.signal_primary_ref_frame,
        inter_ctrl.primary_ref_frame,
        inter_ctrl.disable_cross_frame_cdf_init,
        enable_avg_cdf,
        avg_cdf_type,
        &inter_ctrl.ref_frame_idx,
        reference,
        cdf_load,
    );
    match cdf_load {
        ResolvedCdfLoad::Default => default_cdfs(),
        ResolvedCdfLoad::OutOfRangePrimary => Err(inter_cap!(
            "inter_primary_ref_out_of_range",
            offset,
            "inter.primary_ref_frame out of range",
            SPEC_HEADER
        )),
        ResolvedCdfLoad::LoadSlot {
            primary,
            blend: None,
        } => reference.cdfs_for_slot(primary, offset),
        ResolvedCdfLoad::LoadSlot {
            primary: _,
            blend: Some(_),
        } => Err(inter_cap!(
            "inter_blend_cdf_unmodeled",
            offset,
            "inter.cdf.blend_saved",
            SPEC_HEADER
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn trace_initial_frame_cdfs<T: ReconSample>(
    current_base_q_idx: u32,
    current_order_hint: i32,
    signal_primary_ref_frame: Option<bool>,
    primary_ref_frame: Option<u8>,
    disable_cross_frame_cdf_init: Option<bool>,
    enable_avg_cdf: bool,
    avg_cdf_type: u8,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    cdf_load: ResolvedCdfLoad,
) {
    if std::env::var_os("SPLOT_TRACE_CDF_LIFECYCLE").is_none() {
        return;
    }
    let ref_size: Vec<_> = reference
        .ref_frame_width
        .iter()
        .zip(&reference.ref_frame_height)
        .map(|(w, h)| (*w, *h))
        .collect();
    let ref_has_cdfs: Vec<_> = reference
        .ref_frame_cdfs
        .iter()
        .map(Option::is_some)
        .collect();
    eprintln!(
        "cdf lifecycle base_q={} order_hint={} signal_primary_ref_frame={:?} primary_ref_frame={:?} disable_cross_frame_cdf_init={:?} enable_avg_cdf={} avg_cdf_type={} ref_frame_idx={:?} ref_valid={:?} ref_is_inter={:?} ref_base_q_idx={:?} ref_order_hint={:?} ref_size={:?} ref_has_cdfs={:?} load={:?}",
        current_base_q_idx,
        current_order_hint,
        signal_primary_ref_frame,
        primary_ref_frame,
        disable_cross_frame_cdf_init,
        enable_avg_cdf,
        avg_cdf_type,
        ref_frame_idx,
        reference.ref_valid,
        reference.ref_is_inter,
        reference.ref_base_q_idx,
        reference.ref_order_hint,
        ref_size,
        ref_has_cdfs,
        cdf_load,
    );
}

pub(in crate::runtime_minimal::inter) fn resolve_inter_block_params<'a, T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &'a InterReferenceState<'a, T>,
    placed: &PlacedInterBlock,
    rect: mc::McBlockRect,
    offset: ByteOffset,
) -> Result<mc::InterBlockParams<'a, T>> {
    let ref_frame0 =
        resolve_block_reference_frame(ref_frame_idx, reference, placed.block.ref_frame0, offset)?;
    Ok(if let Some(ref_frame1) = placed.block.ref_frame1 {
        let ref_frame1 =
            resolve_block_reference_frame(ref_frame_idx, reference, ref_frame1, offset)?;
        mc::InterBlockParams::compound_average(
            ref_frame0,
            ref_frame1,
            rect,
            placed.block.mv,
            placed.block.mv1,
            placed.block.interp,
        )
    } else if let Some(warp_params) = placed.block.warp_params {
        mc::InterBlockParams::single_warp(ref_frame0, rect, warp_params)
    } else {
        mc::InterBlockParams::single(ref_frame0, rect, placed.block.mv, placed.block.interp)
    })
}

fn resolve_block_reference_frame<'a, T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &'a InterReferenceState<'a, T>,
    ref_frame: i8,
    offset: ByteOffset,
) -> Result<&'a DecodedFrame<T>> {
    let ref_slot = ref_frame_idx
        .get(ref_frame as usize)
        .copied()
        .ok_or_else(|| {
            inter_cap!(
                "inter_block_ref_frame_out_of_range",
                offset,
                "inter.block.ref_frame out of range",
                SPEC_MODE_INFO
            )
        })?;
    reference.frame_for_slot(ref_slot).ok_or_else(|| {
        inter_missing!(
            "inter_missing_reference_frame",
            offset,
            "inter.block.reference_frame",
            SPEC_REFERENCE
        )
    })
}

fn validate_compound_sequence_subset(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    let Some(seq_inter) = sequence.inter.as_ref() else {
        return Err(compound_missing!(
            "compound_missing_sequence_inter",
            offset,
            "inter.sequence_tools",
            SPEC_MODE_INFO
        ));
    };
    for (enabled, reason, message, spec_section) in [
        (
            seq_inter.enable_masked_compound,
            "compound_masked_compound_enabled",
            "unsupported capability: inter.compound.masked",
            SPEC_MODE_INFO,
        ),
        (
            seq_inter.enable_cwp,
            "compound_cwp_enabled",
            "unsupported capability: inter.compound.cwp",
            SPEC_MODE_INFO,
        ),
        (
            seq_inter.enable_imp_msk_bld,
            "compound_implicit_mask_enabled",
            "unsupported capability: inter.compound.implicit_mask",
            SPEC_MC,
        ),
        (
            seq_inter.enable_opfl_refine != 0,
            "compound_opfl_refine_enabled",
            "unsupported capability: inter.compound.opfl_refine",
            SPEC_MODE_INFO,
        ),
        (
            seq_inter.enable_refinemv,
            "compound_refinemv_enabled",
            "unsupported capability: inter.compound.refinemv",
            SPEC_MODE_INFO,
        ),
        (
            seq_inter.enable_tip,
            "compound_tip_enabled",
            "unsupported capability: inter.tip",
            SPEC_MODE_INFO,
        ),
    ] {
        if enabled {
            return Err(unsupported_compound_at(
                reason,
                offset,
                message,
                spec_section,
            ));
        }
    }
    let tip_frame_mode = core.inter.as_ref().and_then(|inter| inter.tip_frame_mode);
    if tip_frame_mode != Some(TipFrameMode::Disabled) {
        return Err(compound_cap!(
            "compound_active_tip_frame_mode",
            offset,
            "inter.tip.active_frame_mode",
            SPEC_MODE_INFO
        ));
    }
    Ok(())
}

fn compound_is_joint_context(
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, impl ReconSample>,
    current_order_hint: i32,
    offset: ByteOffset,
) -> Result<usize> {
    if ref_frame_idx.len() != 2 {
        return Err(compound_missing!(
            "compound_missing_ref_frame_idx",
            offset,
            "inter.compound.ref_frame_idx[2]",
            SPEC_MODE_INFO
        ));
    }
    let ref_order_hint = |ref_idx: usize| -> Result<i32> {
        let slot = *ref_frame_idx.get(ref_idx).ok_or_else(|| {
            compound_cap!(
                "compound_ref_frame_idx_out_of_range",
                offset,
                "inter.compound.ref_frame out of range",
                SPEC_MODE_INFO
            )
        })?;
        reference
            .ref_order_hint
            .get(slot as usize)
            .copied()
            .map(|hint| i32::try_from(hint).unwrap_or(i32::MAX))
            .ok_or_else(|| {
                compound_missing!(
                    "compound_reference_order_hint",
                    offset,
                    "inter.compound.reference_order_hint",
                    SPEC_REFERENCE
                )
            })
    };
    let first_order_hint = ref_order_hint(0)?;
    let second_order_hint = ref_order_hint(1)?;
    Ok(compound_is_joint_context_from_order_hints(
        first_order_hint,
        second_order_hint,
        current_order_hint,
    ))
}

fn compound_is_joint_context_from_order_hints(
    first_order_hint: i32,
    second_order_hint: i32,
    current_order_hint: i32,
) -> usize {
    let first_side = get_relative_dist(first_order_hint, current_order_hint);
    let second_side = get_relative_dist(second_order_hint, current_order_hint);
    let first_dist = first_side.abs();
    let second_dist = second_side.abs();
    let same_side = (first_side < 0 && second_side < 0) || (first_side > 0 && second_side > 0);
    usize::from(same_side || first_dist != second_dist)
}

fn get_relative_dist(a: i32, b: i32) -> i32 {
    (a - b).clamp(-127, 127)
}
#[allow(clippy::too_many_arguments)]
pub(in crate::runtime_minimal::inter) fn add_inter_residual_to_workspace(
    workspace: &mut splot_recon::CurrentFrameWorkspace<impl ReconSample>,
    residual: &InterResidual,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    offset: ByteOffset,
) -> Result<()> {
    let map_recon = |_| {
        inter_cap!(
            "inter_residual_reconstruct",
            offset,
            "inter.residual.reconstruct",
            SPEC_MC
        )
    };
    for block in &residual.blocks {
        let use_tcq = block.plane == ReconPlaneId::Y && luma_use_tcq;
        crate::runtime_minimal_recon::reconstruct_inter_block_residual_rect_into(
            workspace,
            &block.coeffs,
            block.plane,
            block.x,
            block.y,
            block.log2_width,
            block.log2_height,
            qindex,
            use_tcq,
            residual_use_ddt,
            bit_depth,
        )
        .map_err(map_recon)?;
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub(super) struct InterBlock {
    pub(super) ref_frame0: i8,
    pub(super) ref_frame1: Option<i8>,
    pub(super) mv: Mv,
    pub(super) mv1: Mv,
    pub(super) interp: ReconInterpolationFilter,
    pub(super) warp_params: Option<[i64; 6]>,
    #[allow(dead_code)]
    pub(super) bawp: BawpSyntax,
    pub(super) residual: Option<InterResidual>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct BawpSyntax {
    pub(super) luma_flag: u8,
    pub(super) chroma_flag: bool,
}
#[derive(Clone, Debug)]
pub(super) struct PlacedInterBlock {
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    pub(super) luma_w: usize,
    pub(super) luma_h: usize,
    pub(super) block: InterBlock,
}
#[derive(Clone, Debug)]
pub(super) struct InterResidual {
    pub(super) blocks: Vec<InterResidualBlock>,
}
#[derive(Clone, Debug)]
pub(super) struct InterResidualBlock {
    pub(super) plane: ReconPlaneId,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) tx_size: usize,
    pub(super) log2_width: u32,
    pub(super) log2_height: u32,
    pub(super) coeffs: crate::tile_payload::LumaCoeffBlock,
}
pub(super) struct InterReferenceState<'a, T: ReconSample> {
    pub(super) store: &'a ReferenceFrameStore<&'a DecodedFrame<T>>,
    pub(super) ref_valid: Vec<bool>,
    pub(super) ref_order_hint: Vec<u32>,
    pub(super) ref_frame_width: Vec<u32>,
    pub(super) ref_frame_height: Vec<u32>,
    pub(super) ref_base_q_idx: Vec<u32>,
    pub(super) ref_is_inter: Vec<bool>,
    #[allow(dead_code)]
    pub(super) ref_adapted: Vec<bool>,
    pub(super) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    pub(super) ref_frame_cdfs: Vec<Option<FrameCdfSubset>>,
}

impl<T: ReconSample> InterReferenceState<'_, T> {
    fn frame_for_slot(&self, slot: u32) -> Option<&DecodedFrame<T>> {
        let slot = ReferenceSlot::new(slot as usize).ok()?;
        self.store.get(slot).ok().flatten().copied()
    }

    fn cdfs_for_slot(&self, slot: u32, offset: ByteOffset) -> Result<FrameCdfSubset> {
        self.ref_frame_cdfs
            .get(slot as usize)
            .and_then(Clone::clone)
            .ok_or_else(|| {
                inter_missing!(
                    "inter_missing_reference_cdf_context",
                    offset,
                    "inter.cdf.saved_primary",
                    SPEC_HEADER
                )
            })
    }

    fn header_view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.ref_valid,
            &self.ref_order_hint,
            &self.ref_frame_width,
            &self.ref_frame_height,
            &self.ref_base_q_idx,
        )
        .with_lr_frame_filter_class_counts(&self.lr_frame_filter_class_counts)
    }
}
pub(super) fn parse_validated_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
) -> Result<FrameHeaderCore> {
    let core = parse_inter_frame_core(envelope, sequence, reference)?;
    validate_inter_frame_core(&core, sequence, envelope.offset)?;
    Ok(core)
}
fn parse_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        inter_missing!(
            "inter_tile_group_prefix_parse",
            envelope.offset,
            "inter.tile_group_prefix",
            SPEC_HEADER
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(inter_cap!(
            "inter_non_first_tile_group",
            envelope.offset,
            "inter.frame_header_not_in_first_tile_group",
            SPEC_HEADER
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu: false,
        active_sequence: Some(sequence),
        mfh_record: None,
        reference_state: reference.header_view(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        inter_missing!(
            "inter_frame_header_parse",
            envelope.offset,
            "inter.frame_header_core",
            SPEC_HEADER
        )
    })
}
fn validate_inter_frame_core(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    if core.status != FrameHeaderParseStatus::InterHeaderComplete {
        return Err(inter_missing!(
            "inter_incomplete_frame_header",
            offset,
            "inter.frame_header_complete",
            SPEC_HEADER
        ));
    }
    if core.frame_is_intra != Some(false) || core.is_key_frame {
        return Err(inter_cap!(
            "inter_not_inter_frame",
            offset,
            "inter.frame_type",
            SPEC_HEADER
        ));
    }
    if core.show_existing_frame != Some(false) {
        return Err(inter_cap!(
            "inter_unsupported_output_control",
            offset,
            "inter.show_existing_frame",
            SPEC_HEADER
        ));
    }
    let Some(frame_size) = core.frame_size else {
        return Err(inter_missing!(
            "inter_unsupported_frame_size",
            offset,
            "inter.frame_size",
            SPEC_HEADER
        ));
    };
    let width = frame_size.width;
    let height = frame_size.height;
    if width == 0 || height == 0 {
        return Err(inter_cap!(
            "inter_unsupported_frame_size",
            offset,
            "inter.frame_size empty",
            SPEC_HEADER
        ));
    }
    if sequence.partition.is_none() {
        return Err(inter_cap!(
            "inter_unsupported_superblock_size",
            offset,
            "inter.superblock_size unavailable",
            SPEC_HEADER
        ));
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(inter_missing!(
            "inter_missing_tile_info",
            offset,
            "inter.tile_info",
            SPEC_HEADER
        ));
    };
    if tile_info.tile_cols != 1 || tile_info.tile_rows != 1 {
        return Err(inter_cap!(
            "inter_multi_tile_frame",
            offset,
            "inter.tile_count != 1",
            SPEC_HEADER
        ));
    }
    let unsupported_tools = core
        .quantization_params
        .is_none_or(|quant| quant.base_q_idx == 0)
        || core
            .segmentation_params
            .as_ref()
            .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .delta_q_params
            .is_none_or(|delta| delta.delta_q_present)
        || core
            .lossless_info
            .as_ref()
            .is_none_or(|lossless| lossless.coded_lossless)
        || sequence.inter.is_none()
        || core.deblocking_filter_params.is_none()
        || core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable)
        || core.cdef_params.is_none()
        || core.lr_params.is_none()
        || core.ccso_params.is_none()
        || core
            .inter_tail
            .as_ref()
            .is_none_or(|tail| tail.apply_grain || tail.skip_mode_present);
    if std::env::var_os("SPLOT_TRACE_INTER_FRAME_TOOLS").is_some() {
        eprintln!(
            "inter tools offset={} flex_mvres={:?} allow_tcq={:?} inter_ddt={:?} base_q={:?} segmentation={:?} qmatrix={:?} delta_q={:?} lossless={:?} deblock={:?} gdf={:?} cdef={:?} lr={:?} ccso={:?} tail={:?}",
            offset.get(),
            sequence.inter.as_ref().map(|inter| inter.enable_flex_mvres),
            core.lossless_info
                .as_ref()
                .map(|lossless| lossless.allow_tcq),
            sequence
                .transform_quant_entropy
                .as_ref()
                .map(|tq| tq.enable_inter_ddt),
            core.quantization_params.map(|quant| quant.base_q_idx),
            core.segmentation_params
                .as_ref()
                .map(|seg| seg.segmentation_enabled),
            core.setup_qm_params.as_ref().map(|qm| qm.using_qmatrix),
            core.delta_q_params
                .as_ref()
                .map(|delta| delta.delta_q_present),
            core.lossless_info
                .as_ref()
                .map(|lossless| lossless.coded_lossless),
            core.deblocking_filter_params
                .as_ref()
                .map(|filter| filter.apply_deblocking_filter),
            core.gdf_params.as_ref().map(|gdf| gdf.gdf_frame_enable),
            core.cdef_params.as_ref().map(|cdef| cdef.cdef_frame_enable),
            core.lr_params.as_ref().map(|lr| lr.uses_lr),
            core.ccso_params
                .as_ref()
                .map(|ccso| (ccso.ccso_frame_flag, ccso.planes.len())),
            core.inter_tail.as_ref().map(|tail| {
                (
                    tail.apply_grain,
                    tail.tx_mode,
                    tail.reference_select,
                    tail.skip_mode_present,
                    tail.allow_bawp,
                    tail.use_global_motion,
                )
            }),
        );
        if let Some(ccso) = core.ccso_params.as_ref() {
            eprintln!(
                "inter ccso detail offset={} frame_flag={:?} planes={:?}",
                offset.get(),
                ccso.ccso_frame_flag,
                ccso.planes
                    .iter()
                    .map(|plane| (
                        plane.ccso_planes,
                        plane.ccso_bo_only,
                        plane.ccso_scale_idx,
                        plane.ccso_quant_idx,
                        plane.ccso_ext_filter,
                        plane.ccso_edge_clf,
                        plane.ccso_max_band_log2,
                        plane.ccso_offset_idx.len(),
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(lr) = core.lr_params.as_ref() {
            eprintln!(
                "inter lr detail offset={} sizes={:?} planes={:?}",
                offset.get(),
                lr.loop_restoration_size,
                lr.planes
                    .iter()
                    .map(|plane| (
                        plane.restoration_type,
                        plane.frame_filters_on,
                        plane.num_filter_classes,
                        plane
                            .frame_filter_bank
                            .as_ref()
                            .map(|bank| bank.classes.len()),
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(inter) = core.inter.as_ref() {
            eprintln!(
                "inter ref detail offset={} num_total_refs={:?} ref_frame_idx={:?} use_bru={:?} bru_ref={:?} bru_inactive={:?} use_ref_frame_mvs={:?}",
                offset.get(),
                inter.num_total_refs,
                inter.ref_frame_idx,
                inter.use_bru,
                inter.bru_ref,
                inter.bru_inactive,
                inter.use_ref_frame_mvs,
            );
        }
    }
    if unsupported_tools {
        return Err(inter_cap!(
            "inter_unsupported_frame_tools",
            offset,
            "inter.frame_tools",
            SPEC_HEADER
        ));
    }
    if core.ccso_params.as_ref().is_some_and(|ccso| {
        ccso.planes
            .iter()
            .any(|plane| plane.reuse_ccso || plane.sb_reuse_ccso)
    }) {
        return Err(inter_cap!(
            "inter_ccso_reuse_unimplemented",
            offset,
            "inter.ccso.reference_reuse",
            "5.18.7.12"
        ));
    }
    if core
        .cdef_params
        .as_ref()
        .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(false))
    {
        return Err(inter_cap!(
            "inter_cdef_skip_grid_unimplemented",
            offset,
            "inter.cdef.skip_txfm_grid",
            "5.18.7.10"
        ));
    }
    if core.lr_params.as_ref().is_some_and(|lr| {
        lr.planes
            .iter()
            .any(|plane| plane.frame_filters_on && plane.num_filter_classes.unwrap_or(1) > 1)
    }) {
        return Err(inter_cap!(
            "inter_lr_multiclass_tx_skip_unimplemented",
            offset,
            "inter.lr.multiclass_tx_skip_grid",
            "5.18.7.9"
        ));
    }
    Ok(())
}

fn effective_quantizer_deltas_are_zero(
    sequence: &SequenceHeader,
    quantization: &QuantizationParams,
) -> bool {
    let Some(tq) = sequence.transform_quant_entropy.as_ref() else {
        return false;
    };
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);

    quantization.delta_q_y_dc + seq_quant.base_y_dc_delta_q == 0
        && (seq_quant.num_planes == 1
            || (quantization.delta_q_u_dc + seq_quant.base_uv_dc_delta_q == 0
                && quantization.delta_q_v_dc + seq_quant.base_uv_dc_delta_q == 0
                && quantization.delta_q_u_ac + seq_quant.base_uv_ac_delta_q == 0
                && quantization.delta_q_v_ac + seq_quant.base_uv_ac_delta_q == 0))
}

mod block;
mod compound;
mod cross_frame;
mod find_mv_stack;
mod mc;
pub(in crate::runtime_minimal) mod mv_scaling;
pub(super) mod read_mv;
mod single_ref;

use block::decode_inter_blocks;
use cross_frame::{ResolvedCdfLoad, order_hint_history_unwrapped, resolve_cdf_load};

#[cfg(test)]
mod lr_live_storage_tests;
#[cfg(test)]
mod lr_source_read_tests;
#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            TIER_ID,
            MATRIX_ROW,
            FEATURE_ID,
            spec_section,
            message,
            REMEDIATION,
            Some(byte_offset),
        )),
    }
}

fn unsupported_compound_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            TIER_ID,
            COMPOUND_MATRIX_ROW,
            COMPOUND_FEATURE_ID,
            spec_section,
            message,
            COMPOUND_REMEDIATION,
            Some(byte_offset),
        )),
    }
}
