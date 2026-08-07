// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use std::sync::Arc;

use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, QuantizationParams, RESTRICTED_OH,
    SefTrailingBits, SlotFrameFilterTaps, TipFrameMode, get_relative_dist, parse_frame_header_core,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::hls::MultiFrameHeaderRecord;
use splot_core::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, DecodedFrame, InterpolationFilter as ReconInterpolationFilter,
    PlaneId as ReconPlaneId, PlaneRect, QuantizerDeltas, ReconSample, ReferenceFrameStore,
    ReferenceSlot,
};

use crate::bitstream::tile_payload::{
    FrameCdfSubset, FrameQuantizerDeltasScope, GeneralIntraResidualError,
    reconstruct_general_intra_chroma_cctx_pair_into,
};
use crate::error::DecodeError;
use crate::pipeline::frame_engine::finish::{FilterSinkSetup, FrameWalk, WalkStage};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{derive_visible_luma_rect, ensure_runtime_limits};
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
fn completed_walk<T: ReconSample>(output: InterDecodeOutput<T>) -> FrameWalk<T> {
    let (frame, core, frame_cdfs, ccso_grid, motion_field) = output;
    FrameWalk {
        stage: WalkStage::complete(frame),
        core: Arc::new(core),
        frame_cdfs,
        ccso_grid,
        motion_field,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_inter_frame<T: ReconSample>(
    scratch: &mut InterDecodeScratch<T>,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<FrameWalk<T>> {
    let offset = frame_envelope.offset;

    if frame_envelope.header.obu_type == ObuType::BridgeFrame {
        reference
            .pixel_reference_gate(named_pixel_reference_slots(&core))
            .wait("arm=bridge")?;
        return decode_bridge_frame(
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
        )
        .map(completed_walk);
    }
    if frame_envelope.header.obu_type.is_tip_frame() {
        reference
            .pixel_reference_gate(named_pixel_reference_slots(&core))
            .wait("arm=tip")?;
        return decode_tip_output_frame(
            scratch,
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
        )
        .map(completed_walk);
    }
    if !matches!(
        frame_envelope.header.obu_type,
        ObuType::LeadingTileGroup | ObuType::RegularTileGroup | ObuType::Switch | ObuType::RasFrame
    ) {
        return Err(inter_cap!(
            "inter_unexpected_obu_type",
            offset,
            "inter.obu_type not in the inter tile-group family",
            SPEC_HEADER
        ));
    }
    if let Some(inter) = core.inter.as_ref() {
        for dependency in reference.motion_dependencies(&inter.ref_frame_idx) {
            dependency.wait_field();
        }
    }
    let frame_walk::InterWalkPrologue {
        tile_plan,
        mut workspace,
        setup,
        facts,
        ref_frame_idx,
        quantizer_deltas,
    } = frame_walk::derive_inter_walk_prologue(
        plan,
        candidate,
        bytes,
        frame_envelope,
        &core,
        sequence,
        options,
        reference,
        bit_depth,
    )?;
    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let (frame_cdfs, filter_inputs) = decode_inter_blocks(
        scratch,
        tile_plan,
        frame_envelope,
        sequence,
        &core,
        options,
        facts,
        &ref_frame_idx,
        reference,
        &mut workspace,
    )?;
    let core = Arc::new(core);
    Ok(setup.frame_walk(workspace, filter_inputs, core, frame_cdfs, true))
}

pub(crate) fn decode_tip_output_frame<T: ReconSample>(
    scratch: &mut InterDecodeScratch<T>,
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "tip_output_state",
            offset,
            "inter.tip_output.state",
            SPEC_HEADER
        )
    })?;
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
        block::tip::reconstruct_output(scratch, sequence, &core, reference, bit_depth, offset)?;
    let mut frame_cdfs = (*frame_cdfs).clone();
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(core.quantization_params.map_or(0, |q| q.base_q_idx))
        .map_err(|_| {
            inter_cap!(
                "tip_output_coefficient_cdf_context",
                offset,
                "inter.cdf.reference_coefficient_context",
                SPEC_REFERENCE
            )
        })?;
    Ok((frame, core, Arc::new(frame_cdfs), None, motion_field))
}

fn decode_bridge_frame<T: ReconSample>(
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "bridge_missing_frame_size",
            offset,
            "inter.bridge.frame_size",
            SPEC_HEADER
        )
    })?;
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let ref_slot = core.bridge_frame_ref_idx.ok_or_else(|| {
        inter_missing!(
            "bridge_missing_reference_slot",
            offset,
            "inter.bridge.reference_slot",
            SPEC_HEADER
        )
    })?;
    let source = reference.hold_slot(ref_slot).ok_or_else(|| {
        inter_missing!(
            "bridge_missing_reference_frame",
            offset,
            "inter.bridge.reference_frame",
            SPEC_REFERENCE
        )
    })?;
    let reference_order_hint = reference
        .ref_order_hint
        .get(ref_slot as usize)
        .copied()
        .ok_or_else(|| {
            inter_missing!(
                "bridge_missing_reference_order_hint",
                offset,
                "inter.bridge.reference_order_hint",
                SPEC_REFERENCE
            )
        })?;
    let motion_field = bridge::motion_field(
        frame_size,
        core.display_order_hint().unwrap_or(0),
        reference_order_hint,
    )
    .ok_or_else(|| {
        inter_cap!(
            "bridge_motion_field_dimensions",
            offset,
            "inter.bridge.motion_field_dimensions",
            SPEC_REFERENCE
        )
    })?;
    let frame_cdfs = resolve_initial_frame_cdfs(&core, sequence, reference, offset)?;
    let visible = derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    let frame = bridge::reconstruct(source.samples()?, frame_size, visible, 0, offset)?;
    let mut frame_cdfs = (*frame_cdfs).clone();
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(core.quantization_params.map_or(0, |q| q.base_q_idx))
        .map_err(|_| {
            inter_cap!(
                "bridge_coefficient_cdf_context",
                offset,
                "inter.cdf.reference_coefficient_context",
                SPEC_REFERENCE
            )
        })?;
    Ok((frame, core, Arc::new(frame_cdfs), None, motion_field))
}

fn resolve_initial_frame_cdfs(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
) -> Result<Arc<FrameCdfSubset>> {
    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint =
        i32::try_from(core.display_order_hint().unwrap_or(0)).unwrap_or(i32::MAX);
    let default_cdfs = || {
        FrameCdfSubset::default_for_base_q(current_base_q_idx)
            .map(Arc::new)
            .map_err(|_| {
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
        cdf_blending_enabled(enable_avg_cdf, inter_ctrl.tip_frame_mode),
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
            let mut cdfs = (*reference.cdfs_for_slot(primary, offset)?).clone();
            let blend_cdfs = reference.cdfs_for_slot(blend, offset)?;
            cdfs.blend_from_saved(&blend_cdfs);
            Ok(Arc::new(cdfs))
        }
    }
}

/// Exact pending entropy products one frame's tile parse may consume.
pub(crate) struct EntropyDependencies {
    cdfs: Vec<FrameCdfHandle>,
    ccso_grids: Vec<CcsoGridHandle>,
}

impl EntropyDependencies {
    /// Admission conditions for every selected CDF and CCSO source.
    pub(crate) fn conditions(&self) -> Vec<splot_parallel::Condition<'_>> {
        self.cdfs
            .iter()
            .map(FrameCdfHandle::condition)
            .chain(self.ccso_grids.iter().map(CcsoGridHandle::condition))
            .collect()
    }
}

/// Owned product handles that gate one asynchronous TIP output frame.
pub(crate) struct TipOutputDependencies<T: ReconSample> {
    samples: Vec<RefFrameSlot<T>>,
    entropy: EntropyDependencies,
    motion: Vec<MotionFieldHandle>,
}

impl<T: ReconSample> TipOutputDependencies<T> {
    pub(crate) fn conditions(&self) -> Vec<splot_parallel::Condition<'_>> {
        self.samples
            .iter()
            .map(RefFrameSlot::settled_condition)
            .chain(self.entropy.conditions())
            .chain(self.motion.iter().map(MotionFieldHandle::field_condition))
            .collect()
    }
}

/// Resolves entropy-product identities before the current frame refreshes slots.
pub(crate) fn entropy_dependencies(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
) -> EntropyDependencies {
    let mut cdfs = Vec::new();
    if let Some(inter_ctrl) = core.inter.as_ref() {
        let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
        let current_order_hint =
            i32::try_from(core.display_order_hint().unwrap_or(0)).unwrap_or(i32::MAX);
        let (enable_avg_cdf, avg_cdf_type) = sequence
            .transform_quant_entropy
            .as_ref()
            .map_or((false, 1u8), |tq| (tq.enable_avg_cdf, tq.avg_cdf_type));
        if let ResolvedCdfLoad::LoadSlot { primary, blend } = resolve_cdf_load(
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
            cdf_blending_enabled(enable_avg_cdf, inter_ctrl.tip_frame_mode),
            avg_cdf_type,
        ) {
            for slot in [Some(primary), blend].into_iter().flatten() {
                if let Some(handle) = reference
                    .ref_frame_cdfs
                    .get(slot as usize)
                    .and_then(Option::as_ref)
                {
                    cdfs.push(handle.clone());
                }
            }
        }
    }

    let mut ccso_grids = Vec::new();
    if sequence
        .filter
        .as_ref()
        .is_some_and(|filter| filter.enable_ccso)
        && core
            .ccso_params
            .as_ref()
            .and_then(|ccso| ccso.ccso_frame_flag)
            == Some(true)
        && let (Some(inter), Some(ccso)) = (core.inter.as_ref(), core.ccso_params.as_ref())
    {
        for plane in &ccso.planes {
            if !plane.sb_reuse_ccso {
                continue;
            }
            let ref_index = plane.ccso_ref_idx.unwrap_or(0) as usize;
            let Some(slot) = inter.ref_frame_idx.get(ref_index) else {
                continue;
            };
            if let Some(handle) = reference
                .ref_ccso_unit_grids
                .get(*slot as usize)
                .and_then(Option::as_ref)
            {
                ccso_grids.push(handle.clone());
            }
        }
    }
    EntropyDependencies { cdfs, ccso_grids }
}

/// Resolves every sample, entropy, and motion product a TIP output may read.
pub(crate) fn tip_output_dependencies<T: ReconSample>(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<T>,
) -> TipOutputDependencies<T> {
    let samples = reference
        .pixel_reference_gate(named_pixel_reference_slots(core))
        .shared_slots();
    let entropy = entropy_dependencies(core, sequence, reference);
    let motion = core.inter.as_ref().map_or_else(Vec::new, |inter| {
        reference.motion_dependencies(&inter.ref_frame_idx)
    });
    TipOutputDependencies {
        samples,
        entropy,
        motion,
    }
}

const fn cdf_blending_enabled(enable_avg_cdf: bool, tip_frame_mode: Option<TipFrameMode>) -> bool {
    enable_avg_cdf && !matches!(tip_frame_mode, Some(TipFrameMode::AsOutput))
}

/// The reference frames one inter block reads, borrowed for the block only.
///
/// A still-filtering reference is readable only while its shared borrow lives,
/// so the borrows are taken once per block, kept on the reconstructing thread's
/// stack, and released before it waits on anything.
pub(in crate::prediction::inter) struct HeldInterBlockReferences<'a, T: ReconSample> {
    reference0: reference::HeldFrameSamples<'a, T>,
    /// The second list's borrow, absent when it names the first list's slot.
    reference1: Option<reference::HeldFrameSamples<'a, T>>,
    compound: bool,
}

/// Borrows the reference frames one placed inter block names.
///
/// Two lists that name the same slot share one borrow, since borrowing a
/// still-filtering frame twice at once would deadlock against its filter phase.
///
/// # Errors
///
/// Returns a missing-reference diagnostic when a named slot holds no frame.
pub(in crate::prediction::inter) fn hold_inter_block_references<'a, T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &'a InterReferenceState<T>,
    placed: &PlacedInterBlock,
    offset: ByteOffset,
) -> Result<HeldInterBlockReferences<'a, T>> {
    let slot0 = block_reference_slot(ref_frame_idx, placed.block.ref_frame0, offset)?;
    let slot1 = placed
        .block
        .ref_frame1
        .map(|ref_frame1| block_reference_slot(ref_frame_idx, ref_frame1, offset))
        .transpose()?;
    let (reference0, reference1) = hold_reference_pair(reference, slot0, slot1, offset)?;
    Ok(HeldInterBlockReferences {
        reference0,
        reference1,
        compound: slot1.is_some(),
    })
}

impl<T: ReconSample> HeldInterBlockReferences<'_, T> {
    /// Builds the § 7.13.3 prediction parameters for the held block.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when a held reference's samples are gone.
    pub(in crate::prediction::inter) fn block_params(
        &self,
        placed: &PlacedInterBlock,
        rect: mc::McBlockRect,
    ) -> Result<mc::InterBlockParams<'_, T>> {
        let ref_frame0 = self.reference0.samples()?;
        Ok(if self.compound {
            let ref_frame1 = self.reference1.as_ref().unwrap_or(&self.reference0);
            mc::InterBlockParams::compound_average(
                ref_frame0,
                ref_frame1.samples()?,
                rect,
                placed.block.mv,
                placed.block.mv1,
                placed.block.interp,
                placed.block.compound_blend,
            )
            .with_optflow_distances(placed.block.optflow_distances)
            .with_compound_warp(placed.block.warp_params)
            .with_sub8x8_chroma(placed.sub8x8_chroma)
            .with_chroma(placed.predict_chroma)
        } else if let Some(warp_params) = placed.block.warp_params[0] {
            mc::InterBlockParams::single_warp(
                ref_frame0,
                rect,
                placed.block.mv,
                placed.block.interp,
                warp_params,
            )
            .with_sub8x8_chroma(placed.sub8x8_chroma)
            .with_chroma(placed.predict_chroma)
        } else {
            mc::InterBlockParams::single(ref_frame0, rect, placed.block.mv, placed.block.interp)
                .with_chroma(placed.predict_chroma)
        })
    }
}

fn block_reference_slot(ref_frame_idx: &[u32], ref_frame: i8, offset: ByteOffset) -> Result<u32> {
    ref_frame_idx
        .get(ref_frame as usize)
        .copied()
        .ok_or_else(|| block_reference_out_of_range(offset))
}

fn block_reference_out_of_range(offset: ByteOffset) -> DecodeError {
    inter_cap!(
        "inter_block_ref_frame_out_of_range",
        offset,
        "inter.block.ref_frame out of range",
        SPEC_MODE_INFO
    )
}

fn hold_reference_slot<T: ReconSample>(
    reference: &InterReferenceState<T>,
    slot: u32,
    offset: ByteOffset,
) -> Result<reference::HeldFrameSamples<'_, T>> {
    reference.hold_slot(slot).ok_or_else(|| {
        inter_missing!(
            "inter_missing_block_reference_frame",
            offset,
            "inter.block.reference_frame",
            SPEC_REFERENCE
        )
    })
}

/// Borrows two named references in a stable lock order while returning them in
/// list order. This prevents two compound readers from each holding one live
/// frame's progress lock while waiting behind the other frame's writer.
pub(in crate::prediction::inter) fn hold_reference_pair<T: ReconSample>(
    reference: &InterReferenceState<T>,
    first: u32,
    second: Option<u32>,
    offset: ByteOffset,
) -> Result<(
    reference::HeldFrameSamples<'_, T>,
    Option<reference::HeldFrameSamples<'_, T>>,
)> {
    let Some(second) = second.filter(|second| *second != first) else {
        return Ok((hold_reference_slot(reference, first, offset)?, None));
    };
    let first_progress = reference.slot(first).and_then(RefFrameSlot::progress);
    let second_progress = reference.slot(second).and_then(RefFrameSlot::progress);
    if first_progress
        .zip(second_progress)
        .is_some_and(|(first, second)| core::ptr::eq(first, second))
    {
        return Ok((hold_reference_slot(reference, first, offset)?, None));
    }
    let first_key = first_progress.map_or(0, |progress| core::ptr::from_ref(progress).addr());
    let second_key = second_progress.map_or(0, |progress| core::ptr::from_ref(progress).addr());
    if first_key <= second_key {
        let first = hold_reference_slot(reference, first, offset)?;
        let second = hold_reference_slot(reference, second, offset)?;
        Ok((first, Some(second)))
    } else {
        let second = hold_reference_slot(reference, second, offset)?;
        let first = hold_reference_slot(reference, first, offset)?;
        Ok((first, Some(second)))
    }
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
pub(in crate::prediction::inter) fn add_inter_residual_to_workspace<T: ReconSample>(
    scratch: &mut InterResidualReconScratch<T>,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    residual: &InterResidual,
    residual_blocks: &[InterResidualBlock],
    qindex: u32,
    luma_use_tcq: bool,
    enable_inter_ddt: bool,
    use_intrabc: bool,
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
    let use_ddt = enable_inter_ddt && !use_intrabc;
    let blocks = residual.blocks(residual_blocks).ok_or_else(|| {
        inter_missing!(
            "inter_residual_block_range",
            offset,
            "inter.residual.block_range",
            SPEC_MC
        )
    })?;
    for (index, block) in blocks.iter().enumerate() {
        if block.cctx_pair_delta < 0 {
            continue;
        }
        let cctx_type = block.coeffs.cctx_type.unwrap_or(0);
        if block.plane == ReconPlaneId::U && cctx_type != 0 {
            let v_block = inter_residual_chroma_pair(blocks, index, block).ok_or_else(|| {
                inter_missing!(
                    "inter_residual_cctx_pair",
                    offset,
                    "inter.residual.cctx_pair",
                    SPEC_MC
                )
            })?;
            reconstruct_inter_residual_chroma_cctx_pair(
                scratch,
                sink,
                [block, v_block],
                qindex,
                cctx_type,
                use_ddt,
                bit_depth,
            )
            .map_err(map_recon)?;
            continue;
        }
        let use_tcq = block.plane == ReconPlaneId::Y && luma_use_tcq;
        crate::pipeline::reconstruct::reconstruct_inter_block_residual_rect_into(
            sink,
            &block.coeffs,
            block.plane,
            block.x,
            block.y,
            block.log2_width,
            block.log2_height,
            qindex,
            use_tcq,
            use_ddt,
            bit_depth,
        )
        .map_err(map_recon)?;
    }
    Ok(())
}

fn inter_residual_chroma_pair<'a>(
    blocks: &'a [InterResidualBlock],
    u_index: usize,
    u: &InterResidualBlock,
) -> Option<&'a InterResidualBlock> {
    let delta = usize::try_from(u.cctx_pair_delta).ok()?;
    let v = blocks.get(u_index.checked_add(delta)?)?;
    is_matching_inter_residual_v_block(u, v).then_some(v)
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
    scratch: &mut InterResidualReconScratch<T>,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    [u, v]: [&InterResidualBlock; 2],
    qindex: u32,
    cctx_type: usize,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    read_inter_residual_prediction(sink, u, &mut scratch.u_prediction)?;
    read_inter_residual_prediction(sink, v, &mut scratch.v_prediction)?;
    reconstruct_general_intra_chroma_cctx_pair_into(
        &u.coeffs,
        &scratch.u_prediction,
        &v.coeffs,
        &scratch.v_prediction,
        qindex,
        u.log2_width,
        u.log2_height,
        cctx_type,
        use_ddt,
        bit_depth,
        &mut scratch.u_output,
        &mut scratch.v_output,
    )?;
    write_inter_residual_block(sink, u, &scratch.u_output)?;
    write_inter_residual_block(sink, v, &scratch.v_output)?;
    Ok(())
}

fn read_inter_residual_prediction<T: ReconSample>(
    sink: &mc::WorkspaceSink<'_, '_, T>,
    block: &InterResidualBlock,
    prediction: &mut Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let rect = inter_residual_block_rect(block)?;
    prediction.clear();
    prediction.reserve(rect.width() * rect.height());
    for row in sink.rect_rows(block.plane, rect)? {
        prediction.extend_from_slice(row);
    }
    Ok(())
}

fn write_inter_residual_block<T: ReconSample>(
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    block: &InterResidualBlock,
    samples: &[T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let rect = inter_residual_block_rect(block)?;
    sink.write_rect(block.plane, rect, samples, rect.width())?;
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
    /// Per-list § 7.13.3.23 warp models; slot 1 is only compound LOCALWARP and a
    /// `None` slot uses translational motion compensation.
    pub(crate) warp_params: [Option<[i32; 6]>; 2],
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
    /// § 7.13.3.25 `AvailU`, which `is_inside` scopes to the current tile.
    pub(crate) avail_up: bool,
    /// § 7.13.3.25 `AvailL`, which `is_inside` scopes to the current tile.
    pub(crate) avail_left: bool,
}

pub(crate) type InterDecodeOutput<T> = (
    DecodedFrame<T>,
    FrameHeaderCore,
    Arc<FrameCdfSubset>,
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
    pub(crate) predict_chroma: bool,
    pub(crate) sub8x8_chroma: bool,
    pub(crate) interintra_chroma: bool,
    pub(crate) block: InterBlock,
}

impl PlacedInterBlock {
    pub(in crate::prediction::inter) const fn motion_compensation_rect(&self) -> mc::McBlockRect {
        mc::McBlockRect::from_luma_rect(self.luma_x, self.luma_y, self.luma_w, self.luma_h)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct InterResidual {
    pub(crate) block_range: core::ops::Range<usize>,
}

impl InterResidual {
    pub(crate) fn blocks<'a>(
        &self,
        arena: &'a [InterResidualBlock],
    ) -> Option<&'a [InterResidualBlock]> {
        arena.get(self.block_range.clone())
    }
}

#[derive(Debug, Default)]
pub(in crate::prediction::inter) struct InterResidualReconScratch<T: ReconSample> {
    u_prediction: Vec<T>,
    v_prediction: Vec<T>,
    u_output: Vec<T>,
    v_output: Vec<T>,
}
#[derive(Clone, Debug)]
pub(crate) struct InterResidualBlock {
    pub(crate) plane: ReconPlaneId,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: usize,
    pub(crate) log2_width: u32,
    pub(crate) log2_height: u32,
    pub(crate) cctx_pair_delta: i16,
    pub(crate) coeffs: crate::bitstream::tile_payload::LumaCoeffBlock,
}
pub(crate) struct InterReferenceState<T: ReconSample> {
    pub(crate) store: ReferenceFrameStore<RefFrameSlot<T>>,
    pub(crate) ref_valid: Vec<bool>,
    pub(crate) ref_order_hint: Vec<u32>,
    pub(crate) ref_order_hint_lsbs: Vec<u32>,
    pub(crate) ref_implicit_output_frame: Vec<bool>,
    pub(crate) ref_immediate_output_frame: Vec<bool>,
    pub(crate) ref_frame_width: Vec<u32>,
    pub(crate) ref_frame_height: Vec<u32>,
    pub(crate) ref_base_q_idx: Vec<u32>,
    pub(crate) ref_counter: Vec<u32>,
    pub(crate) ref_delta_q_u_ac: Vec<i32>,
    pub(crate) ref_delta_q_v_ac: Vec<i32>,
    ref_chroma_ac_deltas: Vec<[i32; 2]>,
    pub(crate) ref_is_inter: Vec<bool>,
    pub(crate) ref_long_term_id: Vec<Option<u32>>,
    #[allow(dead_code)]
    pub(crate) ref_adapted: Vec<bool>,
    pub(crate) ref_num_total_refs: Vec<u32>,
    pub(crate) saved_global_motion_order_hints:
        Vec<splot_core::headers::frame::SavedGlobalMotionOrderHints>,
    pub(crate) saved_global_motion_params: Vec<splot_core::headers::frame::SavedGlobalMotionParams>,
    pub(crate) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    pub(crate) lr_frame_filter_taps: Vec<SlotFrameFilterTaps>,
    pub(crate) ref_frame_cdfs: Vec<Option<FrameCdfHandle>>,
    pub(crate) ref_ccso_params: Vec<Option<Arc<splot_core::headers::frame::CcsoParams>>>,
    pub(crate) ref_ccso_unit_grids: Vec<Option<CcsoGridHandle>>,
    pub(crate) ref_motion_fields: Vec<Option<MotionFieldHandle>>,
}

impl<T: ReconSample> InterReferenceState<T> {
    /// Builds a reference-free state over a minimal empty store.
    ///
    /// # Errors
    /// Returns [`splot_recon::ReconError`] when the minimal store capacity is
    /// rejected.
    pub(crate) fn empty() -> splot_recon::Result<Self> {
        Ok(Self {
            store: ReferenceFrameStore::with_capacity(1)?,
            ref_valid: Vec::new(),
            ref_order_hint: Vec::new(),
            ref_order_hint_lsbs: Vec::new(),
            ref_implicit_output_frame: Vec::new(),
            ref_immediate_output_frame: Vec::new(),
            ref_frame_width: Vec::new(),
            ref_frame_height: Vec::new(),
            ref_base_q_idx: Vec::new(),
            ref_counter: Vec::new(),
            ref_delta_q_u_ac: Vec::new(),
            ref_delta_q_v_ac: Vec::new(),
            ref_chroma_ac_deltas: Vec::new(),
            ref_is_inter: Vec::new(),
            ref_long_term_id: Vec::new(),
            ref_adapted: Vec::new(),
            ref_num_total_refs: Vec::new(),
            saved_global_motion_order_hints: Vec::new(),
            saved_global_motion_params: Vec::new(),
            lr_frame_filter_class_counts: Vec::new(),
            lr_frame_filter_taps: Vec::new(),
            ref_frame_cdfs: Vec::new(),
            ref_ccso_params: Vec::new(),
            ref_ccso_unit_grids: Vec::new(),
            ref_motion_fields: Vec::new(),
        })
    }

    /// Shares the selected reference slots' published § 7.9 motion fields.
    ///
    /// A selected slot whose frame has not reconstructed yet yields `None` for
    /// the whole list: the § 7.9 projection would otherwise read that reference
    /// as motionless and silently decode a different frame. Unselected slots
    /// are not projection inputs and need not stall the frame.
    pub(crate) fn resolve_motion_fields(
        &self,
        ref_frame_idx: &[u32],
    ) -> Option<Vec<Option<Arc<TemporalMotionField>>>> {
        self.ref_motion_fields
            .iter()
            .enumerate()
            .map(|(index, slot)| match slot {
                Some(handle)
                    if ref_frame_idx
                        .iter()
                        .any(|&selected| selected as usize == index) =>
                {
                    handle.field().map(|field| Some(Arc::clone(field)))
                }
                None | Some(_) => Some(None),
            })
            .collect()
    }

    pub(crate) fn from_metadata(
        store: ReferenceFrameStore<RefFrameSlot<T>>,
        metadata: ReferenceMetadata,
    ) -> Self {
        let ref_chroma_ac_deltas = metadata
            .ref_delta_q_u_ac
            .iter()
            .copied()
            .zip(metadata.ref_delta_q_v_ac.iter().copied())
            .map(|(u, v)| [u, v])
            .collect();
        Self {
            store,
            ref_valid: metadata.ref_valid,
            ref_order_hint: metadata.ref_order_hint,
            ref_order_hint_lsbs: metadata.ref_order_hint_lsbs,
            ref_implicit_output_frame: metadata.ref_implicit_output_frame,
            ref_immediate_output_frame: metadata.ref_immediate_output_frame,
            ref_frame_width: metadata.ref_frame_width,
            ref_frame_height: metadata.ref_frame_height,
            ref_base_q_idx: metadata.ref_base_q_idx,
            ref_counter: metadata.ref_counter,
            ref_delta_q_u_ac: metadata.ref_delta_q_u_ac,
            ref_delta_q_v_ac: metadata.ref_delta_q_v_ac,
            ref_chroma_ac_deltas,
            ref_is_inter: metadata.ref_is_inter,
            ref_long_term_id: metadata.ref_long_term_id,
            ref_adapted: metadata.ref_adapted,
            ref_num_total_refs: metadata.ref_num_total_refs,
            saved_global_motion_order_hints: metadata.saved_global_motion_order_hints,
            saved_global_motion_params: metadata.saved_global_motion_params,
            lr_frame_filter_class_counts: metadata.lr_frame_filter_class_counts,
            lr_frame_filter_taps: metadata.lr_frame_filter_taps,
            ref_frame_cdfs: metadata.ref_frame_cdfs,
            ref_ccso_params: metadata.ref_ccso_params,
            ref_ccso_unit_grids: metadata.ref_ccso_unit_grids,
            ref_motion_fields: metadata.ref_motion_fields,
        }
    }
}

impl<T: ReconSample> Drop for InterReferenceState<T> {
    fn drop(&mut self) {
        let meta = crate::reference::buffer::ReferenceMetadata {
            ref_valid: std::mem::take(&mut self.ref_valid),
            ref_order_hint: std::mem::take(&mut self.ref_order_hint),
            ref_order_hint_lsbs: std::mem::take(&mut self.ref_order_hint_lsbs),
            ref_implicit_output_frame: std::mem::take(&mut self.ref_implicit_output_frame),
            ref_immediate_output_frame: std::mem::take(&mut self.ref_immediate_output_frame),
            ref_frame_width: std::mem::take(&mut self.ref_frame_width),
            ref_frame_height: std::mem::take(&mut self.ref_frame_height),
            ref_base_q_idx: std::mem::take(&mut self.ref_base_q_idx),
            ref_counter: std::mem::take(&mut self.ref_counter),
            ref_delta_q_u_ac: std::mem::take(&mut self.ref_delta_q_u_ac),
            ref_delta_q_v_ac: std::mem::take(&mut self.ref_delta_q_v_ac),
            ref_is_inter: std::mem::take(&mut self.ref_is_inter),
            ref_long_term_id: std::mem::take(&mut self.ref_long_term_id),
            ref_adapted: std::mem::take(&mut self.ref_adapted),
            ref_num_total_refs: std::mem::take(&mut self.ref_num_total_refs),
            saved_global_motion_order_hints: std::mem::take(
                &mut self.saved_global_motion_order_hints,
            ),
            saved_global_motion_params: std::mem::take(&mut self.saved_global_motion_params),
            lr_frame_filter_class_counts: std::mem::take(&mut self.lr_frame_filter_class_counts),
            lr_frame_filter_taps: std::mem::take(&mut self.lr_frame_filter_taps),
            ref_frame_cdfs: std::mem::take(&mut self.ref_frame_cdfs),
            ref_ccso_params: std::mem::take(&mut self.ref_ccso_params),
            ref_ccso_unit_grids: std::mem::take(&mut self.ref_ccso_unit_grids),
            ref_motion_fields: std::mem::take(&mut self.ref_motion_fields),
        };
        crate::reference::buffer::recycle_reference_metadata(meta);
    }
}

/// The settled state of the reference frames one frame's prediction reads.
///
/// A frame's entropy walk parses without reference samples; only reconstruction
/// reads them. A caller polls [`Self::is_ready`] before enqueuing work that
/// reads samples, and blocks on [`Self::wait`] when it is about to read them on
/// the driver thread itself.
pub(crate) struct PixelReferenceGate<'a, T: ReconSample> {
    slots: [Option<&'a RefFrameSlot<T>>; ReferenceSlot::MAX_SLOTS],
}

/// The reference slots a parsed frame header names for pixel prediction.
pub(crate) fn named_pixel_reference_slots(core: &FrameHeaderCore) -> impl Iterator<Item = u32> {
    core.inter
        .as_ref()
        .map_or(&[][..], |inter| &inter.ref_frame_idx[..])
        .iter()
        .copied()
        .chain(core.bridge_frame_ref_idx)
}

impl<'a, T: ReconSample> PixelReferenceGate<'a, T> {
    /// Whether every named reference frame has settled.
    pub(crate) fn is_ready(&self) -> bool {
        self.slots.iter().flatten().all(|slot| slot.is_settled())
    }

    /// Returns the admission conditions for every named reference settling.
    pub(crate) fn conditions(&self) -> Vec<splot_parallel::Condition<'a>> {
        self.slots
            .iter()
            .flatten()
            .map(|slot| slot.settled_condition())
            .collect()
    }

    /// Shares the named slots so a scheduler can register their conditions
    /// before moving the complete reference state into the admitted job.
    pub(crate) fn shared_slots(&self) -> Vec<RefFrameSlot<T>> {
        self.slots
            .iter()
            .flatten()
            .map(|slot| slot.share())
            .collect()
    }

    /// Blocks the calling driver thread until every named reference frame has
    /// settled, running pool jobs instead of parking idle.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when a referenced frame's filter phase
    /// failed; the driver replaces it with that frame's own recorded failure.
    pub(crate) fn wait(&self, arm: &str) -> Result<()> {
        let started = crate::timing::start();
        for slot in self.slots.iter().flatten() {
            slot.wait_settled()?;
        }
        crate::timing::report_detail("pipeline_gate_wait", started, arm);
        Ok(())
    }
}

impl<T: ReconSample> InterReferenceState<T> {
    /// Shares each distinct motion-field product named by a reference map.
    pub(crate) fn motion_dependencies(&self, ref_frame_idx: &[u32]) -> Vec<MotionFieldHandle> {
        let mut seen = vec![false; self.ref_motion_fields.len()];
        ref_frame_idx
            .iter()
            .filter_map(|&slot| {
                let index = slot as usize;
                if *seen.get(index)? {
                    return None;
                }
                seen[index] = true;
                self.ref_motion_fields.get(index).and_then(Option::as_ref)
            })
            .cloned()
            .collect()
    }

    /// Gates on the stored frames the named reference slots resolve to.
    ///
    /// Slots without a stored frame are left out: reading one is already a
    /// fail-closed diagnostic at the point of use.
    pub(crate) fn pixel_reference_gate(
        &self,
        named: impl IntoIterator<Item = u32>,
    ) -> PixelReferenceGate<'_, T> {
        let mut slots: [Option<&RefFrameSlot<T>>; ReferenceSlot::MAX_SLOTS] =
            [None; ReferenceSlot::MAX_SLOTS];
        for slot in named {
            let Ok(index) = ReferenceSlot::new(slot as usize) else {
                continue;
            };
            let Ok(Some(frame)) = self.store.get(index) else {
                continue;
            };
            if !slots
                .iter()
                .flatten()
                .any(|held| core::ptr::eq(*held, frame))
                && let Some(entry) = slots.iter_mut().find(|entry| entry.is_none())
            {
                *entry = Some(frame);
            }
        }
        PixelReferenceGate { slots }
    }

    /// Borrows one reference slot's samples for the returned handle's lifetime.
    fn hold_slot(&self, slot: u32) -> Option<reference::HeldFrameSamples<'_, T>> {
        self.slot(slot).and_then(RefFrameSlot::hold_samples)
    }

    fn slot(&self, slot: u32) -> Option<&RefFrameSlot<T>> {
        let slot = ReferenceSlot::new(slot as usize).ok()?;
        self.store.get(slot).ok().flatten()
    }

    fn cdfs_for_slot(&self, slot: u32, offset: ByteOffset) -> Result<Arc<FrameCdfSubset>> {
        self.ref_frame_cdfs
            .get(slot as usize)
            .and_then(Option::as_ref)
            .and_then(FrameCdfHandle::product)
            .cloned()
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
    ) -> Result<Arc<splot_core::headers::frame::CcsoParams>> {
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

    pub(crate) fn header_view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.ref_valid,
            &self.ref_order_hint,
            &self.ref_frame_width,
            &self.ref_frame_height,
            &self.ref_base_q_idx,
        )
        .with_quantizer_delta_state(&self.ref_chroma_ac_deltas)
        .with_primary_reference_state(&self.ref_counter, &self.ref_is_inter)
        .with_long_term_id_state(&self.ref_long_term_id)
        .with_global_motion_state(
            &self.ref_num_total_refs,
            &self.saved_global_motion_order_hints,
            &self.saved_global_motion_params,
        )
        .with_single_layer_order_hint_state(
            &self.ref_order_hint_lsbs,
            &self.ref_implicit_output_frame,
            &self.ref_immediate_output_frame,
        )
        .with_lr_frame_filter_class_counts(&self.lr_frame_filter_class_counts)
        .with_lr_frame_filter_taps(&self.lr_frame_filter_taps)
    }
}
pub(crate) fn parse_inter_frame_activation(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
) -> Result<FrameHeaderCore> {
    if envelope.header.obu_type.is_sef() || envelope.header.obu_type.is_tip_frame() {
        parse_sef_or_tip_frame_core(envelope, sequence, reference, first_picture_in_tu, None)
    } else {
        parse_inter_frame_core(envelope, sequence, reference, first_picture_in_tu, None)
    }
}

pub(crate) fn parse_validated_inter_frame_core_with_mfh(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
) -> Result<FrameHeaderCore> {
    let mut core = if envelope.header.obu_type.is_sef() || envelope.header.obu_type.is_tip_frame() {
        parse_sef_or_tip_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
        )?
    } else {
        parse_inter_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
        )?
    };
    if envelope.header.obu_type.is_sef() {
        validate_sef_frame_core(&core, envelope.offset)?;
    } else if envelope.header.obu_type.is_tip_frame() {
        infer_tip_output_quantization(&mut core, sequence, reference, envelope.offset)?;
        validate_tip_output_frame_core(&core, envelope.offset)?;
    } else {
        resolve_ccso_reference_reuse(&mut core, reference, envelope.offset)?;
        validate_inter_frame_core(&core, sequence, envelope.offset)?;
        validate_ras_reference_ids(&core, reference, envelope.offset)?;
    }
    Ok(core)
}

fn parse_sef_or_tip_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: reference.header_view(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        if envelope.header.obu_type.is_sef() {
            inter_missing!(
                "sef_frame_header_parse",
                envelope.offset,
                "show_existing.frame_header_core",
                SPEC_HEADER
            )
        } else {
            inter_missing!(
                "tip_output_frame_header_parse",
                envelope.offset,
                "inter.tip_output.frame_header_core",
                SPEC_HEADER
            )
        }
    })
}

fn validate_sef_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    let complete = core.status == FrameHeaderParseStatus::ShowExistingFrameComplete
        && core.show_existing_frame == Some(true)
        && core.frame_to_show_map_idx.is_some()
        && core.order_hint.is_some()
        && core.refresh_frame_flags == Some(0)
        && core.immediate_output_frame == Some(true)
        && core.implicit_output_frame == Some(false)
        && core.sef_film_grain.is_some()
        && core.sef_trailing_bits == Some(SefTrailingBits::Valid);
    if !complete {
        return Err(inter_cap!(
            "sef_incomplete_state",
            offset,
            "inter.show_existing.complete_state",
            SPEC_HEADER
        ));
    }
    Ok(())
}

fn infer_tip_output_quantization(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
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
    let pair = find_mv_stack::tip_reference_pair_from_hints(
        core.display_order_hint().unwrap_or(0),
        &hints,
    );
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
    let average_delta = |a: i32, b: i32| a / 2 + b / 2 + ((a % 2 + b % 2 + 1) >> 1);
    core.quantization_params = Some(QuantizationParams::inferred_tip(
        (past_q + future_q + 1) >> 1,
        average_delta(past_u, future_u),
        average_delta(past_v, future_v),
    ));
    Ok(())
}

fn validate_tip_output_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    let complete = core.status == FrameHeaderParseStatus::InterHeaderComplete
        && core.obu_type.is_tip_frame()
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
    reference: &InterReferenceState<impl ReconSample>,
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
        let plane = &ccso.planes[plane_index];
        let reuse_ccso = plane.reuse_ccso;
        let Some(slot) = ccso_reference_slot(
            ref_frame_idx,
            reuse_ccso,
            plane.sb_reuse_ccso,
            plane.ccso_ref_idx.unwrap_or(0),
            offset,
        )?
        else {
            continue;
        };
        if !reuse_ccso {
            continue;
        }
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

fn ccso_reference_slot(
    ref_frame_idx: &[u32],
    reuse_ccso: bool,
    sb_reuse_ccso: bool,
    ref_index: u32,
    offset: ByteOffset,
) -> Result<Option<u32>> {
    if !reuse_ccso && !sb_reuse_ccso {
        return Ok(None);
    }
    ref_frame_idx
        .get(ref_index as usize)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            inter_diag!(
                "inter_ccso_reference_index_out_of_range",
                offset,
                "CCSO reference index is outside NumTotalRefs",
                "6.17.7.8"
            )
        })
}

fn parse_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if envelope.header.obu_type != ObuType::BridgeFrame {
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
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
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
    if core.obu_type == ObuType::BridgeFrame {
        return validate_bridge_frame_core(core, offset);
    }
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
    if core.order_hint.is_none() {
        return Err(inter_missing!(
            "inter_missing_display_order_hint",
            offset,
            "inter.order_hint",
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
    if core.tile_info.is_none() {
        return Err(inter_missing!(
            "inter_missing_tile_info",
            offset,
            "inter.tile_info",
            SPEC_HEADER
        ));
    }
    let unsupported_tools = core.quantization_params.is_none()
        || core.segmentation_params.as_ref().is_none_or(|seg| {
            !inter_segmentation_supported(
                seg.segmentation_enabled,
                seg.segmentation_update_map,
                seg.segmentation_temporal_update,
                &seg.features,
            )
        })
        || core.setup_qm_params.is_none()
        || core.delta_q_params.is_none()
        || core.lossless_info.is_none()
        || sequence.inter.is_none()
        || core.deblocking_filter_params.is_none()
        || core.gdf_params.is_none()
        || core.cdef_params.is_none()
        || core.lr_params.is_none()
        || core.ccso_params.is_none();
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

fn validate_ras_reference_ids(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
) -> Result<()> {
    if core.obu_type != ObuType::RasFrame {
        return Ok(());
    }
    let inter = core.inter.as_ref().ok_or_else(|| {
        inter_missing!(
            "ras_missing_reference_map",
            offset,
            "inter.ras.reference_map",
            "6.17.2"
        )
    })?;
    let count = usize::try_from(inter.num_total_refs.unwrap_or(0)).unwrap_or(usize::MAX);
    for &slot in inter.ref_frame_idx.iter().take(count) {
        let id = reference
            .ref_long_term_id
            .get(slot as usize)
            .copied()
            .flatten();
        if id.is_none_or(|id| !core.ref_long_term_ids.contains(&id)) {
            return Err(inter_cap!(
                "ras_reference_long_term_id_not_listed",
                offset,
                "inter.ras.reference_long_term_id",
                "6.17.2"
            ));
        }
    }
    Ok(())
}

fn validate_bridge_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    let reference_map_complete = core.inter.as_ref().is_some_and(|inter| {
        if core.frame_is_intra == Some(true) {
            inter.num_total_refs == Some(0)
        } else {
            inter.num_total_refs == Some(1)
                && inter.ref_frame_idx.first() == core.bridge_frame_ref_idx.as_ref()
        }
    });
    let complete = core.status == FrameHeaderParseStatus::InterHeaderComplete
        && core.frame_is_intra.is_some()
        && core.show_existing_frame == Some(false)
        && core.order_hint.is_some()
        && core
            .frame_size
            .is_some_and(|size| size.width != 0 && size.height != 0)
        && core.tile_info.is_some()
        && core.quantization_params.is_some()
        && core.bridge_film_grain.is_some()
        && reference_map_complete;
    if !complete {
        return Err(inter_cap!(
            "bridge_incomplete_state",
            offset,
            "inter.bridge.complete_state",
            SPEC_HEADER
        ));
    }
    Ok(())
}

fn inter_segmentation_supported(
    enabled: bool,
    update_map: bool,
    temporal_update: bool,
    features: &[[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS],
) -> bool {
    !enabled
        || (update_map
            && !temporal_update
            && features
                .iter()
                .all(|features| features[1..].iter().all(|feature| !feature.enabled)))
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
mod bridge;
mod compound;
mod cross_frame;
mod find_mv_stack;
mod frame_products;
mod frame_walk;
pub(crate) mod mc;
mod motion_field;
pub(crate) mod mv_scaling;
pub(crate) mod read_mv;
pub(crate) mod reference;
mod single_ref;

#[cfg(test)]
pub(crate) use block::prepare_intrabc_leaf_entry;
pub(crate) use block::{
    InterBlockFacts, InterDecodeScratch, InterFilterInputs, InterFrameParse,
    ScheduledFrameProgress, decode_inter_blocks, parse_inter_frame_blocks,
};
use cross_frame::{ResolvedCdfLoad, resolve_cdf_load};
pub(crate) use find_mv_stack::{MotionFieldLayout, TemporalMotionField, TemporalMvScratch};
pub(crate) use frame_products::{CcsoGridHandle, FrameCdfHandle};
pub(crate) use frame_walk::{
    DeferredInterWalk, ScheduledInterWalk, inter_frame_info, motion_field_layout,
    parse_inter_frame, splittable_inter_frame,
};
pub(crate) use motion_field::MotionFieldHandle;

#[cfg(test)]
#[path = "inter/test_support_tests.rs"]
pub(crate) mod test_support;

#[cfg(test)]
mod global_motion_tests;

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
