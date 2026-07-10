// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, QuantizationParams, RESTRICTED_OH,
    TipFrameMode, get_relative_dist, parse_frame_header_core,
};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, InterpolationFilter as ReconInterpolationFilter,
    PixelFormat, PlaneId as ReconPlaneId, PlaneRect, QuantizerDeltas, ReconSample,
    ReferenceFrameStore, ReferenceSlot,
};

use crate::bitstream::tile_payload::{
    FrameCdfSubset, GeneralIntraResidualError,
    reconstruct_general_intra_chroma_cctx_pair_with_predictions,
};
use crate::error::DecodeError;
use crate::pipeline::ensure_runtime_limits;
use crate::reference::buffer::ReferenceMetadata;
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

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
pub(crate) struct Mv {
    pub(crate) row: i32,
    pub(crate) col: i32,
}

impl Mv {
    const ZERO: Self = Self { row: 0, col: 0 };
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_inter_frame<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<'_, T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;

    if frame_envelope.header.obu_type == ObuType::RegularTip {
        return decode_tip_output_frame(
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
        );
    }
    if frame_envelope.header.obu_type != ObuType::RegularTileGroup {
        return Err(inter_cap!(
            "inter_unexpected_obu_type",
            offset,
            "inter.obu_type != regular_tile_group",
            SPEC_HEADER
        ));
    }
    if !matches!(
        sequence.general.chroma_format_idc,
        ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
    ) {
        return Err(inter_cap!(
            "inter_non_420_chroma",
            offset,
            "inter.chroma_format != monochrome_or_4:2:0",
            SPEC_MC
        ));
    }

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
    if !(1..=7).contains(&num_total_refs) {
        return Err(inter_cap!(
            "inter_unsupported_num_total_refs",
            offset,
            "inter.single_ref.num_total_refs not in 1..=7",
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
    if block_reference_select {
        validate_compound_sequence_subset(sequence, &core, offset)?;
    }
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
        let mut tile_plan = crate::pipeline::derive_inter_tile_plan(
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
    ensure_runtime_limits(
        limits,
        frame_width,
        frame_height,
        tile_size,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;

    let interpolation_filter = inter.interpolation_filter.ok_or_else(|| {
        inter_missing!(
            "inter_missing_interpolation_filter",
            offset,
            "inter.interpolation_filter",
            SPEC_MC
        )
    })?;

    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<T>(
        frame_width as usize,
        frame_height as usize,
        bit_depth,
        PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?,
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
        sequence
            .inter
            .as_ref()
            .map_or(0, |seq_inter| seq_inter.num_same_ref_compound)
            .min(u8::try_from(num_total_refs).unwrap_or(u8::MAX)),
        &ref_frame_idx,
        reference,
        &mut workspace,
        qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        initial_cdfs,
    )?;
    let motion_field = filter_inputs.motion_field.clone();

    let mut filter_sink = crate::filters::wienerns_lr::recon_final_filter_sink(
        workspace,
        frame_width as usize,
        frame_height as usize,
        bit_depth,
    );
    filter_sink.set_gdf_reference_context(Some(
        crate::filters::gdf::GdfReferenceContext::from_reference_list(
            core.order_hint_lsb.unwrap_or(0),
            &ref_frame_idx,
            &reference.ref_order_hint,
        ),
    ));
    filter_sink.set_deblock_blocks(
        filter_inputs.deblock_blocks,
        filter_inputs.chroma_deblock_blocks,
    );
    filter_sink.set_cdef_grid(Some(filter_inputs.cdef_grid));
    let ccso_grid = filter_inputs.ccso_grid.clone();
    filter_sink.set_ccso_grid(filter_inputs.ccso_grid);
    filter_sink.set_cfl_ds_filter_index(
        sequence
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index),
    );
    filter_sink.set_tx_skip_records(filter_inputs.tx_skip_records);
    filter_sink.set_lr_source_blocks(filter_inputs.lr_source_blocks);
    filter_sink.set_lr_unit_filters(filter_inputs.lr_unit_filters);
    let frame = filter_sink.into_filtered_frame(
        &core,
        crate::pipeline::deblock_quant_deltas(sequence, &core),
        offset,
    )?;

    Ok((frame, core, frame_cdfs, ccso_grid, motion_field))
}

fn decode_tip_output_frame<T: ReconSample>(
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<'_, T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;
    if !matches!(
        sequence.general.chroma_format_idc,
        ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
    ) {
        return Err(inter_cap!(
            "tip_output_non_420_chroma",
            offset,
            "inter.tip_output.chroma_format != monochrome_or_4:2:0",
            SPEC_MC
        ));
    }
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "tip_output_state",
            offset,
            "inter.tip_output.state",
            SPEC_HEADER
        )
    })?;
    let order_hint_bits = sequence
        .inter
        .as_ref()
        .map_or(0, |seq_inter| u32::from(seq_inter.order_hint_bits));
    if !order_hint_history_unwrapped(
        &reference.ref_valid,
        &reference.ref_order_hint,
        order_hint_bits,
        core.order_hint_lsb.unwrap_or(0),
    ) {
        return Err(inter_cap!(
            "tip_output_order_hint_wrapped",
            offset,
            "inter.tip_output.order_hint.wrapped_reference_history",
            SPEC_REFERENCE
        ));
    }
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let frame_cdfs = resolve_initial_frame_cdfs(&core, sequence, reference, offset)?;
    let (frame, motion_field) =
        block::tip::reconstruct_output(sequence, &core, reference, bit_depth, offset)?;
    Ok((frame, core, frame_cdfs, None, motion_field))
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
            primary,
            blend: Some(blend),
        } => {
            let mut cdfs = reference.cdfs_for_slot(primary, offset)?;
            let blend_cdfs = reference.cdfs_for_slot(blend, offset)?;
            cdfs.blend_from_saved(&blend_cdfs);
            Ok(cdfs)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::prediction::inter) fn resolve_inter_block_params<'a, T: ReconSample>(
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
            placed.block.compound_blend,
        )
        .with_optflow_distances(placed.block.optflow_distances)
        .with_chroma(placed.has_chroma)
    } else if let Some(warp_params) = placed.block.warp_params {
        mc::InterBlockParams::single_warp(ref_frame0, rect, warp_params)
            .with_chroma(placed.has_chroma)
    } else {
        mc::InterBlockParams::single(ref_frame0, rect, placed.block.mv, placed.block.interp)
            .with_chroma(placed.has_chroma)
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
            "inter_missing_block_reference_frame",
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
    if sequence.inter.is_none() {
        return Err(compound_missing!(
            "compound_missing_sequence_inter",
            offset,
            "inter.sequence_tools",
            SPEC_MODE_INFO
        ));
    }
    let tip_frame_mode = core.inter.as_ref().and_then(|inter| inter.tip_frame_mode);
    if !matches!(
        tip_frame_mode,
        Some(TipFrameMode::Disabled | TipFrameMode::AsRef)
    ) {
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
    ref_order_hint: &[u32],
    pair: (i8, i8),
    current_order_hint: i32,
    offset: ByteOffset,
) -> Result<usize> {
    let order_hint_of = |ref_frame: i8| -> Result<i32> {
        let slot = usize::try_from(ref_frame)
            .ok()
            .and_then(|ref_idx| ref_frame_idx.get(ref_idx))
            .copied()
            .ok_or_else(|| {
                compound_cap!(
                    "compound_ref_frame_idx_out_of_range",
                    offset,
                    "inter.compound.ref_frame out of range",
                    SPEC_MODE_INFO
                )
            })?;
        ref_order_hint
            .get(slot as usize)
            .copied()
            .map(|hint| {
                if hint == u32::MAX {
                    RESTRICTED_OH
                } else {
                    i32::try_from(hint).unwrap_or(i32::MAX)
                }
            })
            .ok_or_else(|| {
                compound_missing!(
                    "compound_reference_order_hint",
                    offset,
                    "inter.compound.reference_order_hint",
                    SPEC_REFERENCE
                )
            })
    };
    let first_order_hint = order_hint_of(pair.0)?;
    let second_order_hint = order_hint_of(pair.1)?;
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
    let one_restricted =
        (first_order_hint == RESTRICTED_OH) != (second_order_hint == RESTRICTED_OH);
    usize::from(same_side || first_dist != second_dist || one_restricted)
}
#[allow(clippy::too_many_arguments)]
pub(in crate::prediction::inter) fn add_inter_residual_to_workspace(
    workspace: &mut CurrentFrameWorkspace<impl ReconSample>,
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
    let mut paired = vec![false; residual.blocks.len()];
    for (index, block) in residual.blocks.iter().enumerate() {
        if paired[index] {
            continue;
        }
        let cctx_type = block.coeffs.cctx_type.unwrap_or(0);
        if block.plane == ReconPlaneId::U && cctx_type != 0 {
            let Some((v_index, v_block)) = find_inter_residual_chroma_pair(residual, block) else {
                return Err(inter_missing!(
                    "inter_residual_cctx_pair",
                    offset,
                    "inter.residual.cctx_pair",
                    SPEC_MC
                ));
            };
            reconstruct_inter_residual_chroma_cctx_pair(
                workspace, block, v_block, qindex, cctx_type, bit_depth,
            )
            .map_err(map_recon)?;
            paired[index] = true;
            paired[v_index] = true;
            continue;
        }
        let use_tcq = block.plane == ReconPlaneId::Y && luma_use_tcq;
        crate::pipeline::reconstruct::reconstruct_inter_block_residual_rect_into(
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
        paired[index] = true;
    }
    Ok(())
}

fn find_inter_residual_chroma_pair<'a>(
    residual: &'a InterResidual,
    u: &InterResidualBlock,
) -> Option<(usize, &'a InterResidualBlock)> {
    residual
        .blocks
        .iter()
        .enumerate()
        .find(|(_, block)| is_matching_inter_residual_v_block(u, block))
}

fn is_matching_inter_residual_v_block(u: &InterResidualBlock, v: &InterResidualBlock) -> bool {
    v.plane == ReconPlaneId::V
        && u.x == v.x
        && u.y == v.y
        && u.tx_size == v.tx_size
        && u.log2_width == v.log2_width
        && u.log2_height == v.log2_height
}

fn reconstruct_inter_residual_chroma_cctx_pair<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    u: &InterResidualBlock,
    v: &InterResidualBlock,
    qindex: u32,
    cctx_type: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let u_prediction = read_inter_residual_prediction(workspace, u)?;
    let v_prediction = read_inter_residual_prediction(workspace, v)?;
    let (u_out, v_out) = reconstruct_general_intra_chroma_cctx_pair_with_predictions(
        &u.coeffs,
        &u_prediction,
        &v.coeffs,
        &v_prediction,
        qindex,
        u.log2_width,
        u.log2_height,
        cctx_type,
        bit_depth,
    )?;
    write_inter_residual_block(workspace, u, &u_out)?;
    write_inter_residual_block(workspace, v, &v_out)?;
    Ok(())
}

fn read_inter_residual_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: &InterResidualBlock,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let rect = inter_residual_block_rect(block)?;
    let mut prediction = Vec::with_capacity(rect.width() * rect.height());
    for row in workspace.rect_rows(block.plane, rect)? {
        prediction.extend_from_slice(row);
    }
    Ok(prediction)
}

fn write_inter_residual_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &InterResidualBlock,
    samples: &[T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let rect = inter_residual_block_rect(block)?;
    workspace.write_rect(block.plane, rect, samples, rect.width())?;
    Ok(())
}

fn inter_residual_block_rect(
    block: &InterResidualBlock,
) -> core::result::Result<PlaneRect, GeneralIntraResidualError> {
    PlaneRect::new(
        block.x,
        block.y,
        1usize << block.log2_width,
        1usize << block.log2_height,
    )
    .map_err(GeneralIntraResidualError::from)
}
#[derive(Clone, Debug)]
pub(crate) struct InterBlock {
    pub(crate) ref_frame0: i8,
    pub(crate) ref_frame1: Option<i8>,
    pub(crate) mv: Mv,
    pub(crate) mv1: Mv,
    pub(crate) interp: ReconInterpolationFilter,
    pub(crate) warp_params: Option<[i64; 6]>,
    pub(crate) bawp: BawpSyntax,
    pub(crate) interintra: Option<InterIntraPrediction>,
    pub(crate) compound_blend: mc::CompoundBlend,
    pub(crate) optflow_distances: Option<[i32; 2]>,
    pub(crate) residual: Option<InterResidual>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterIntraPrediction {
    SmoothMask {
        mode: splot_recon::InterIntraMode,
    },
    WedgeMask {
        mode: splot_recon::InterIntraMode,
        wedge_index: u8,
    },
}

impl InterIntraPrediction {
    pub(crate) const fn mode(self) -> splot_recon::InterIntraMode {
        match self {
            Self::SmoothMask { mode } | Self::WedgeMask { mode, .. } => mode,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BawpSyntax {
    pub(crate) enabled: bool,
    pub(crate) explicit: bool,
    pub(crate) explicit_scale_positive: bool,
    pub(crate) list_index: u8,
    pub(crate) ref_dist_gt4: bool,
    pub(crate) chroma: bool,
}

pub(crate) type InterDecodeOutput<T> = (
    DecodedFrame<T>,
    FrameHeaderCore,
    FrameCdfSubset,
    Option<crate::filters::ccso::CcsoUnitGrid>,
    TemporalMotionField,
);

#[derive(Clone, Debug)]
pub(crate) struct PlacedInterBlock {
    pub(crate) luma_x: usize,
    pub(crate) luma_y: usize,
    pub(crate) luma_w: usize,
    pub(crate) luma_h: usize,
    pub(crate) chroma_luma_x: usize,
    pub(crate) chroma_luma_y: usize,
    pub(crate) chroma_luma_w: usize,
    pub(crate) chroma_luma_h: usize,
    pub(crate) has_chroma: bool,
    pub(crate) interintra_chroma: bool,
    pub(crate) block: InterBlock,
}
#[derive(Clone, Debug)]
pub(crate) struct InterResidual {
    pub(crate) blocks: Vec<InterResidualBlock>,
}
#[derive(Clone, Debug)]
pub(crate) struct InterResidualBlock {
    pub(crate) plane: ReconPlaneId,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: usize,
    pub(crate) log2_width: u32,
    pub(crate) log2_height: u32,
    pub(crate) coeffs: crate::bitstream::tile_payload::LumaCoeffBlock,
}
pub(crate) struct InterReferenceState<'a, T: ReconSample> {
    pub(crate) store: &'a ReferenceFrameStore<&'a DecodedFrame<T>>,
    pub(crate) ref_valid: Vec<bool>,
    pub(crate) ref_order_hint: Vec<u32>,
    pub(crate) ref_frame_width: Vec<u32>,
    pub(crate) ref_frame_height: Vec<u32>,
    pub(crate) ref_base_q_idx: Vec<u32>,
    pub(crate) ref_delta_q_u_ac: Vec<i32>,
    pub(crate) ref_delta_q_v_ac: Vec<i32>,
    pub(crate) ref_is_inter: Vec<bool>,
    #[allow(dead_code)]
    pub(crate) ref_adapted: Vec<bool>,
    pub(crate) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    pub(crate) lr_frame_filter_taps: Vec<[Vec<Vec<i16>>; 3]>,
    pub(crate) ref_frame_cdfs: Vec<Option<FrameCdfSubset>>,
    pub(crate) ref_ccso_params: Vec<Option<splot_core::headers::frame::CcsoParams>>,
    pub(crate) ref_ccso_unit_grids: Vec<Option<crate::filters::ccso::CcsoUnitGrid>>,
    pub(crate) ref_motion_fields: Vec<Option<TemporalMotionField>>,
}

impl<'a, T: ReconSample> InterReferenceState<'a, T> {
    pub(crate) fn empty(store: &'a ReferenceFrameStore<&'a DecodedFrame<T>>) -> Self {
        Self {
            store,
            ref_valid: Vec::new(),
            ref_order_hint: Vec::new(),
            ref_frame_width: Vec::new(),
            ref_frame_height: Vec::new(),
            ref_base_q_idx: Vec::new(),
            ref_delta_q_u_ac: Vec::new(),
            ref_delta_q_v_ac: Vec::new(),
            ref_is_inter: Vec::new(),
            ref_adapted: Vec::new(),
            lr_frame_filter_class_counts: Vec::new(),
            lr_frame_filter_taps: Vec::new(),
            ref_frame_cdfs: Vec::new(),
            ref_ccso_params: Vec::new(),
            ref_ccso_unit_grids: Vec::new(),
            ref_motion_fields: Vec::new(),
        }
    }

    pub(crate) fn from_metadata(
        store: &'a ReferenceFrameStore<&'a DecodedFrame<T>>,
        metadata: ReferenceMetadata,
    ) -> Self {
        Self {
            store,
            ref_valid: metadata.ref_valid,
            ref_order_hint: metadata.ref_order_hint,
            ref_frame_width: metadata.ref_frame_width,
            ref_frame_height: metadata.ref_frame_height,
            ref_base_q_idx: metadata.ref_base_q_idx,
            ref_delta_q_u_ac: metadata.ref_delta_q_u_ac,
            ref_delta_q_v_ac: metadata.ref_delta_q_v_ac,
            ref_is_inter: metadata.ref_is_inter,
            ref_adapted: metadata.ref_adapted,
            lr_frame_filter_class_counts: metadata.lr_frame_filter_class_counts,
            lr_frame_filter_taps: metadata.lr_frame_filter_taps,
            ref_frame_cdfs: metadata.ref_frame_cdfs,
            ref_ccso_params: metadata.ref_ccso_params,
            ref_ccso_unit_grids: metadata.ref_ccso_unit_grids,
            ref_motion_fields: metadata.ref_motion_fields,
        }
    }

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

    fn ccso_params_for_slot(
        &self,
        slot: u32,
        offset: ByteOffset,
    ) -> Result<splot_core::headers::frame::CcsoParams> {
        self.ref_ccso_params
            .get(slot as usize)
            .and_then(Clone::clone)
            .ok_or_else(|| {
                inter_missing!(
                    "inter_missing_reference_ccso_params",
                    offset,
                    "inter.ccso.saved_params",
                    "7.23"
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
        .with_lr_frame_filter_taps(&self.lr_frame_filter_taps)
    }
}
pub(crate) fn parse_validated_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
) -> Result<FrameHeaderCore> {
    let mut core = if envelope.header.obu_type == ObuType::RegularTip {
        parse_tip_output_frame_core(envelope, sequence, reference)?
    } else {
        parse_inter_frame_core(envelope, sequence, reference)?
    };
    if envelope.header.obu_type == ObuType::RegularTip {
        infer_tip_output_quantization(&mut core, sequence, reference, envelope.offset)?;
        validate_tip_output_frame_core(&core, envelope.offset)?;
    } else {
        resolve_ccso_reference_reuse(&mut core, reference, envelope.offset)?;
        validate_inter_frame_core(&core, sequence, envelope.offset)?;
    }
    Ok(core)
}

fn parse_tip_output_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
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
            "tip_output_frame_header_parse",
            envelope.offset,
            "inter.tip_output.frame_header_core",
            SPEC_HEADER
        )
    })
}

fn infer_tip_output_quantization(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_, impl ReconSample>,
    offset: ByteOffset,
) -> Result<()> {
    if core.quantization_params.is_some()
        || sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_tip_explicit_qp)
    {
        return Ok(());
    }
    let inter = core.inter.as_ref().ok_or_else(|| {
        inter_missing!(
            "tip_output_missing_control",
            offset,
            "inter.tip_output.control",
            SPEC_HEADER
        )
    })?;
    let hints = find_mv_stack::reference_order_hints(
        &inter.ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
    );
    let pair =
        find_mv_stack::tip_reference_pair_from_hints(core.order_hint_lsb.unwrap_or(0), &hints);
    let list_slot = |list_ref: i8| {
        usize::try_from(list_ref)
            .ok()
            .and_then(|index| inter.ref_frame_idx.get(index))
            .and_then(|&slot| usize::try_from(slot).ok())
    };
    let slots = pair.map(|pair| [list_slot(pair.past_ref), list_slot(pair.future_ref)]);
    let values = slots.and_then(|[past, future]| {
        let (past, future) = (past?, future?);
        Some((
            *reference.ref_base_q_idx.get(past)?,
            *reference.ref_base_q_idx.get(future)?,
            *reference.ref_delta_q_u_ac.get(past)?,
            *reference.ref_delta_q_u_ac.get(future)?,
            *reference.ref_delta_q_v_ac.get(past)?,
            *reference.ref_delta_q_v_ac.get(future)?,
        ))
    });
    let Some((past_q, future_q, past_u, future_u, past_v, future_v)) = values else {
        return Err(inter_missing!(
            "tip_output_reference_quantizer",
            offset,
            "inter.tip_output.reference_quantizer",
            SPEC_HEADER
        ));
    };
    core.quantization_params = Some(QuantizationParams::inferred_tip(
        (past_q + future_q + 1) >> 1,
        ((i64::from(past_u) + i64::from(future_u) + 1) >> 1) as i32,
        ((i64::from(past_v) + i64::from(future_v) + 1) >> 1) as i32,
    ));
    Ok(())
}

fn validate_tip_output_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    let complete = core.status == FrameHeaderParseStatus::InterHeaderComplete
        && core.obu_type == ObuType::RegularTip
        && core.frame_is_intra == Some(false)
        && core
            .inter
            .as_ref()
            .is_some_and(|inter| inter.tip_frame_mode == Some(TipFrameMode::AsOutput))
        && core
            .frame_size
            .is_some_and(|size| size.width != 0 && size.height != 0)
        && core.quantization_params.is_some();
    if !complete {
        return Err(inter_cap!(
            "tip_output_incomplete_state",
            offset,
            "inter.tip_output.complete_state",
            SPEC_HEADER
        ));
    }
    Ok(())
}

fn resolve_ccso_reference_reuse(
    core: &mut FrameHeaderCore,
    reference: &InterReferenceState<'_, impl ReconSample>,
    offset: ByteOffset,
) -> Result<()> {
    let Some(inter) = core.inter.as_ref() else {
        return Ok(());
    };
    let ref_frame_idx = &inter.ref_frame_idx;
    let Some(ccso) = core.ccso_params.as_mut() else {
        return Ok(());
    };
    for plane_index in 0..ccso.planes.len() {
        if !ccso.planes[plane_index].reuse_ccso {
            continue;
        }
        let ref_index = ccso.planes[plane_index].ccso_ref_idx.unwrap_or(0);
        let slot = ref_frame_idx
            .get(ref_index as usize)
            .copied()
            .ok_or_else(|| {
                inter_cap!(
                    "inter_ccso_reuse_unimplemented",
                    offset,
                    "inter.ccso.reference_reuse",
                    "5.18.7.12"
                )
            })?;
        let ref_ccso = reference.ccso_params_for_slot(slot, offset)?;
        let Some(ref_plane) = ref_ccso.planes.get(plane_index) else {
            return Err(inter_missing!(
                "inter_missing_reference_ccso_plane",
                offset,
                "inter.ccso.saved_plane",
                "7.23"
            ));
        };
        let plane = &mut ccso.planes[plane_index];
        plane.ccso_bo_only = ref_plane.ccso_bo_only;
        plane.ccso_scale_idx = ref_plane.ccso_scale_idx;
        plane.ccso_quant_idx = ref_plane.ccso_quant_idx;
        plane.ccso_ext_filter = ref_plane.ccso_ext_filter;
        plane.ccso_edge_clf = ref_plane.ccso_edge_clf;
        plane.ccso_max_band_log2 = ref_plane.ccso_max_band_log2;
        plane.ccso_offset_idx.clone_from(&ref_plane.ccso_offset_idx);
    }
    Ok(())
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
            "inter_zero_dimension_frame_size",
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
    let unsupported_tools = core.quantization_params.is_none()
        || core
            .segmentation_params
            .as_ref()
            .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .delta_q_params
            .is_none_or(|delta| delta.delta_q_present)
        || core.lossless_info.is_none()
        || sequence.inter.is_none()
        || core.deblocking_filter_params.is_none()
        || core.gdf_params.is_none()
        || core.cdef_params.is_none()
        || core.lr_params.is_none()
        || core.ccso_params.is_none()
        || core.inter_tail.as_ref().is_none_or(|tail| tail.apply_grain);
    if unsupported_tools {
        return Err(inter_cap!(
            "inter_unsupported_frame_tools",
            offset,
            "inter.frame_tools",
            SPEC_HEADER
        ));
    }
    Ok(())
}

pub(crate) fn effective_quantizer_deltas_are_zero(
    sequence: &SequenceHeader,
    quantization: &QuantizationParams,
) -> bool {
    effective_quantizer_deltas(sequence, quantization).is_some_and(|deltas| {
        deltas.y_dc == 0
            && deltas.u_dc == 0
            && deltas.v_dc == 0
            && deltas.u_ac == 0
            && deltas.v_ac == 0
    })
}

pub(crate) fn effective_quantizer_deltas(
    sequence: &SequenceHeader,
    quantization: &QuantizationParams,
) -> Option<QuantizerDeltas> {
    let tq = sequence.transform_quant_entropy.as_ref()?;
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
    let chroma = seq_quant.num_planes != 1;
    Some(QuantizerDeltas {
        y_dc: quantization.delta_q_y_dc + seq_quant.base_y_dc_delta_q,
        u_dc: if chroma {
            quantization.delta_q_u_dc + seq_quant.base_uv_dc_delta_q
        } else {
            0
        },
        v_dc: if chroma {
            quantization.delta_q_v_dc + seq_quant.base_uv_dc_delta_q
        } else {
            0
        },
        u_ac: if chroma {
            quantization.delta_q_u_ac + seq_quant.base_uv_ac_delta_q
        } else {
            0
        },
        v_ac: if chroma {
            quantization.delta_q_v_ac + seq_quant.base_uv_ac_delta_q
        } else {
            0
        },
    })
}

mod bawp;
mod block;
mod compound;
mod cross_frame;
mod find_mv_stack;
mod mc;
pub(crate) mod mv_scaling;
pub(crate) mod read_mv;
mod single_ref;

pub(crate) use block::decode_inter_blocks;
use cross_frame::{ResolvedCdfLoad, order_hint_history_unwrapped, resolve_cdf_load};
pub(crate) use find_mv_stack::TemporalMotionField;

#[cfg(test)]
#[path = "inter/test_support_tests.rs"]
mod test_support;

#[cfg(test)]
mod tests;

fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    crate::pipeline::unsupported_with_spec(reason, Some(byte_offset), message, spec_section)
}

fn unsupported_compound_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    unsupported_at(reason, byte_offset, message, spec_section)
}
