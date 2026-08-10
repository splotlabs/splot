// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use std::sync::Arc;

use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, FrameType, QuantizationParams,
    SefTrailingBits, SlotFrameFilterTaps, TipFrameMode, get_relative_dist, parse_frame_header_core,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::hls::MultiFrameHeaderRecord;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, DecodedFrame, InterpolationFilter as ReconInterpolationFilter,
    PlaneId as ReconPlaneId, PlaneRect, QuantizerDeltas, ReconSample, ReferenceFrameStore,
    ReferenceSlot,
};

use crate::bitstream::tile_payload::{
    FrameCdfSubset, FrameQuantizerDeltasScope, FrameSegmentIdMap, GeneralIntraResidualError,
    reconstruct_general_intra_chroma_cctx_pair_into,
};
use crate::error::{DecodeError, DecodeHeaderStateError, DecodeReferenceStateError};
use crate::pipeline::frame_engine::finish::{FilterSinkSetup, FrameWalk, WalkStage};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{derive_visible_luma_rect, ensure_runtime_limits};
use crate::reference::buffer::ReferenceMetadata;
use crate::{
    DecodeIvfFrameContext, DecodeOptions, DecodePlannedObu, DecodeSourceIssue, DecodeStreamPlan,
    Result,
};

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

macro_rules! inter_internal {
    ($reason:literal, $offset:expr $(,)?) => {
        crate::error::DecodeError::InternalState {
            reason: $reason,
            byte_offset: $offset,
        }
    };
}

macro_rules! inter_allocation {
    ($context:literal $(,)?) => {
        crate::error::DecodeError::from(splot_recon::ReconError::WorkspaceAllocationFailed {
            plane: splot_recon::PlaneId::Y,
            context: $context,
        })
    };
}

const SPEC_HEADER: &str = "5.18.2";
const SPEC_HEADER_SEMANTICS: &str = "6.17";
const SPEC_FRAME_HEADER_INFO_SEMANTICS: &str = "6.17.2";
const SPEC_TILE_GROUP: &str = "5.19";
const SPEC_MODE_INFO: &str = "5.20.7.6";
const SPEC_MV: &str = "7.11";
const SPEC_MC: &str = "7.13.3.18";
const SPEC_REFERENCE: &str = "7.23";
const SPEC_TIP_TEMPORAL_SCALE: &str = "7.10.1";
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
    let (frame, core, frame_cdfs, ccso_grid, motion_field, segment_ids) = output;
    FrameWalk {
        stage: WalkStage::complete(frame),
        core: Arc::new(core),
        frame_cdfs,
        ccso_grid,
        segment_ids: Arc::new(segment_ids),
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
    if frame_envelope.header.obu_type == ObuType::BridgeFrame {
        reference
            .pixel_reference_gate(named_pixel_reference_slots(&core))
            .wait("arm=bridge")?;
        return decode_bridge_frame(
            candidate,
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
            candidate,
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
        )
        .map(completed_walk);
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
    let (frame_cdfs, filter_inputs, segment_ids) = decode_inter_blocks(
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
    Ok(setup.frame_walk(
        workspace,
        filter_inputs,
        core,
        frame_cdfs,
        segment_ids,
        true,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_tip_output_frame<T: ReconSample>(
    scratch: &mut InterDecodeScratch<T>,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;
    let frame_size = core
        .frame_size
        .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let frame_cdfs = resolve_initial_frame_cdfs(&core, sequence, reference, candidate, offset)?;
    let (frame, motion_field) =
        block::tip::reconstruct_output(scratch, sequence, &core, reference, bit_depth, offset)?;
    let mut frame_cdfs = (*frame_cdfs).clone();
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(core.quantization_params.map_or(0, |q| q.base_q_idx));
    let segment_ids = empty_segment_id_map(&core)?;
    Ok((
        frame,
        core,
        Arc::new(frame_cdfs),
        None,
        motion_field,
        segment_ids,
    ))
}

fn decode_bridge_frame<T: ReconSample>(
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<InterDecodeOutput<T>> {
    let offset = frame_envelope.offset;
    let frame_size = core
        .frame_size
        .ok_or(inter_internal!("bridge_missing_frame_size", offset))?;
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let ref_slot = core
        .bridge_frame_ref_idx
        .ok_or(inter_internal!("bridge_missing_reference_slot", offset))?;
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
        .ok_or(inter_internal!(
            "bridge_missing_reference_order_hint",
            offset
        ))?;
    let motion_field = bridge::motion_field(
        frame_size,
        core.display_order_hint().unwrap_or(0),
        reference_order_hint,
    )
    .ok_or(inter_internal!("bridge_motion_field_dimensions", offset))?;
    let frame_cdfs = resolve_initial_frame_cdfs(&core, sequence, reference, candidate, offset)?;
    let visible = derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    let frame = bridge::reconstruct(source.samples()?, frame_size, visible, 0, offset)?;
    let mut frame_cdfs = (*frame_cdfs).clone();
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(core.quantization_params.map_or(0, |q| q.base_q_idx));
    let segment_ids = empty_segment_id_map(&core)?;
    Ok((
        frame,
        core,
        Arc::new(frame_cdfs),
        None,
        motion_field,
        segment_ids,
    ))
}

fn empty_segment_id_map(core: &FrameHeaderCore) -> Result<FrameSegmentIdMap> {
    let size = core
        .frame_size
        .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
    let mi_dimension = |samples| {
        usize::try_from(samples)
            .ok()
            .and_then(|samples: usize| samples.div_ceil(8).checked_mul(2))
            .ok_or(DecodeHeaderStateError::MissingSegmentIdMap)
    };
    let mi_rows = mi_dimension(size.height)?;
    let mi_cols = mi_dimension(size.width)?;
    FrameSegmentIdMap::new(mi_rows, mi_cols)
        .map_err(|_| DecodeHeaderStateError::MissingSegmentIdMap.into())
}

fn resolve_initial_frame_cdfs(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    candidate: &DecodePlannedObu,
    offset: ByteOffset,
) -> Result<Arc<FrameCdfSubset>> {
    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint =
        i32::try_from(core.display_order_hint().unwrap_or(0)).unwrap_or(i32::MAX);
    let default_cdfs = || {
        Ok(Arc::new(FrameCdfSubset::default_for_base_q(
            current_base_q_idx,
        )))
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
        &reference.ref_counter,
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
        ResolvedCdfLoad::OutOfRangePrimary {
            index,
            reference_count,
        } => Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                candidate
                    .ivf_frame()
                    .map(DecodeIvfFrameContext::frame_index),
                SPEC_HEADER_SEMANTICS,
                format!(
                    "primary reference index {index} is outside the active \
                     {reference_count}-entry map"
                ),
            ),
        }),
        ResolvedCdfLoad::LoadSlot {
            primary,
            blend: None,
        } => reference.cdfs_for_slot(primary),
        ResolvedCdfLoad::LoadSlot {
            primary,
            blend: Some(blend),
        } => {
            let mut cdfs = (*reference.cdfs_for_slot(primary)?).clone();
            let blend_cdfs = reference.cdfs_for_slot(blend)?;
            cdfs.blend_from_saved(&blend_cdfs);
            Ok(Arc::new(cdfs))
        }
    }
}

/// Exact pending entropy products one frame's tile parse may consume.
pub(crate) struct EntropyDependencies {
    cdfs: Vec<FrameCdfHandle>,
    ccso_grids: Vec<CcsoGridHandle>,
    segment_ids: Vec<SegmentIdMapHandle>,
}

impl EntropyDependencies {
    /// Admission conditions for every selected CDF and CCSO source.
    pub(crate) fn conditions(&self) -> Vec<splot_parallel::Condition<'_>> {
        self.cdfs
            .iter()
            .map(FrameCdfHandle::condition)
            .chain(self.ccso_grids.iter().map(CcsoGridHandle::condition))
            .chain(self.segment_ids.iter().map(SegmentIdMapHandle::condition))
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
            &reference.ref_counter,
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
    let needs_previous_segment_ids = core.segmentation_params.as_ref().is_some_and(|seg| {
        seg.segmentation_enabled
            && (!seg.segmentation_update_map || seg.segmentation_temporal_update)
    });
    let segment_ids = needs_previous_segment_ids
        .then(|| previous_segment_slot(core, reference))
        .flatten()
        .and_then(|slot| reference.ref_segment_ids.get(slot))
        .and_then(Option::as_ref)
        .cloned()
        .into_iter()
        .collect();
    EntropyDependencies {
        cdfs,
        ccso_grids,
        segment_ids,
    }
}

fn previous_segment_slot(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
) -> Option<usize> {
    if core.frame_is_intra == Some(true) || core.frame_type == Some(FrameType::Switch) {
        return None;
    }
    let inter = core.inter.as_ref()?;
    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint =
        i32::try_from(core.display_order_hint().unwrap_or(0)).unwrap_or(i32::MAX);
    let (derived, _) = cross_frame::choose_primary_secondary_ref_frame(
        inter.signal_primary_ref_frame,
        inter.primary_ref_frame,
        &inter.ref_frame_idx,
        &reference.ref_is_inter,
        &reference.ref_counter,
        &reference.ref_base_q_idx,
        &reference.ref_order_hint,
        &reference.ref_frame_width,
        &reference.ref_frame_height,
        current_base_q_idx,
        current_order_hint,
    );
    inter
        .ref_frame_idx
        .get(usize::from(derived))
        .and_then(|&slot| usize::try_from(slot).ok())
}

fn previous_segment_ids<'a>(
    core: &FrameHeaderCore,
    reference: &'a InterReferenceState<impl ReconSample>,
    mi_rows: usize,
    mi_cols: usize,
) -> Option<&'a Arc<FrameSegmentIdMap>> {
    if !core
        .segmentation_params
        .as_ref()
        .is_some_and(|seg| seg.segmentation_enabled)
    {
        return None;
    }
    let slot = previous_segment_slot(core, reference)?;
    reference
        .ref_segment_ids
        .get(slot)?
        .as_ref()?
        .product()
        .filter(|map| map.dimensions() == (mi_rows, mi_cols))
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
/// Returns a typed reference-state diagnostic when a named slot has no readable
/// frame samples.
pub(in crate::prediction::inter) fn hold_inter_block_references<'a, T: ReconSample>(
    ref_frame_idx: &[u32],
    reference: &'a InterReferenceState<T>,
    placed: &PlacedInterBlock,
) -> Result<HeldInterBlockReferences<'a, T>> {
    let slot0 = block_reference_slot(ref_frame_idx, placed.block.ref_frame0)?;
    let slot1 = placed
        .block
        .ref_frame1
        .map(|ref_frame1| block_reference_slot(ref_frame_idx, ref_frame1))
        .transpose()?;
    let (reference0, reference1) = hold_reference_pair(reference, slot0, slot1)?;
    Ok(HeldInterBlockReferences {
        reference0,
        reference1,
        compound: slot1.is_some(),
    })
}

impl<T: ReconSample> HeldInterBlockReferences<'_, T> {
    pub(in crate::prediction::inter) fn reference0_samples(
        &self,
    ) -> Result<reference::ReferenceSamples<'_, T>> {
        self.reference0.samples()
    }

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

fn block_reference_slot(ref_frame_idx: &[u32], ref_frame: i8) -> Result<u32> {
    let list_len = ref_frame_idx.len();
    usize::try_from(ref_frame)
        .ok()
        .and_then(|index| ref_frame_idx.get(index))
        .copied()
        .ok_or_else(|| block_reference_out_of_range(ref_frame, list_len))
}

fn block_reference_out_of_range(index: i8, list_len: usize) -> DecodeError {
    DecodeReferenceStateError::ReferenceListIndexOutOfRange { index, list_len }.into()
}

fn hold_reference_slot<T: ReconSample>(
    reference: &InterReferenceState<T>,
    slot: u32,
) -> Result<reference::HeldFrameSamples<'_, T>> {
    let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
    let slot_count = reference.store.capacity();
    let frame = reference.slot(slot).ok_or({
        if slot_index < slot_count {
            DecodeReferenceStateError::MissingFrame { slot: slot_index }
        } else {
            DecodeReferenceStateError::SlotOutOfRange {
                slot: slot_index,
                slot_count,
            }
        }
    })?;
    frame
        .hold_samples()
        .ok_or(DecodeReferenceStateError::ReferenceSamplesUnavailable { slot: slot_index })
        .map_err(Into::into)
}

/// Borrows two named references in a stable lock order while returning them in
/// list order. This prevents two compound readers from each holding one live
/// frame's progress lock while waiting behind the other frame's writer.
pub(in crate::prediction::inter) fn hold_reference_pair<T: ReconSample>(
    reference: &InterReferenceState<T>,
    first: u32,
    second: Option<u32>,
) -> Result<(
    reference::HeldFrameSamples<'_, T>,
    Option<reference::HeldFrameSamples<'_, T>>,
)> {
    let Some(second) = second.filter(|second| *second != first) else {
        return Ok((hold_reference_slot(reference, first)?, None));
    };
    let first_progress = reference.slot(first).and_then(RefFrameSlot::progress);
    let second_progress = reference.slot(second).and_then(RefFrameSlot::progress);
    if first_progress
        .zip(second_progress)
        .is_some_and(|(first, second)| core::ptr::eq(first, second))
    {
        return Ok((hold_reference_slot(reference, first)?, None));
    }
    let first_key = first_progress.map_or(0, |progress| core::ptr::from_ref(progress).addr());
    let second_key = second_progress.map_or(0, |progress| core::ptr::from_ref(progress).addr());
    if first_key <= second_key {
        let first = hold_reference_slot(reference, first)?;
        let second = hold_reference_slot(reference, second)?;
        Ok((first, Some(second)))
    } else {
        let second = hold_reference_slot(reference, second)?;
        let first = hold_reference_slot(reference, first)?;
        Ok((first, Some(second)))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompoundOrderHint {
    Restricted,
    Value(i64),
}

impl CompoundOrderHint {
    const fn current(value: u32) -> Self {
        Self::Value(value as i64)
    }

    const fn reference(value: u32) -> Self {
        if value == u32::MAX {
            Self::Restricted
        } else {
            Self::Value(value as i64)
        }
    }

    const fn is_restricted(self) -> bool {
        matches!(self, Self::Restricted)
    }

    fn relative_dist(self, other: Self) -> i32 {
        match (self, other) {
            (Self::Restricted, Self::Restricted) => 0,
            (Self::Restricted, _) => 127,
            (_, Self::Restricted) => -127,
            (Self::Value(first), Self::Value(second)) => (first - second).clamp(-127, 127) as i32,
        }
    }

    fn frame_distance_from(self, current: Self) -> i32 {
        let distance = current.relative_dist(self);
        if self.is_restricted() {
            -distance
        } else {
            distance
        }
    }
}

fn compound_is_joint_context(
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    pair: (i8, i8),
    current_order_hint: u32,
) -> Result<usize> {
    let order_hint_of = |ref_frame: i8| -> Result<CompoundOrderHint> {
        let list_len = ref_frame_idx.len();
        let slot = usize::try_from(ref_frame)
            .ok()
            .and_then(|ref_idx| ref_frame_idx.get(ref_idx))
            .copied()
            .ok_or(DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                index: ref_frame,
                list_len,
            })?;
        let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
        let slot_count = ref_order_hint.len();
        ref_order_hint
            .get(slot_index)
            .copied()
            .map(CompoundOrderHint::reference)
            .ok_or_else(|| {
                DecodeReferenceStateError::SlotOutOfRange {
                    slot: slot_index,
                    slot_count,
                }
                .into()
            })
    };
    let first_order_hint = order_hint_of(pair.0)?;
    let second_order_hint = order_hint_of(pair.1)?;
    Ok(compound_is_joint_context_from_order_hints(
        first_order_hint,
        second_order_hint,
        CompoundOrderHint::current(current_order_hint),
    ))
}

#[cfg(test)]
#[test]
fn compound_is_joint_context_keeps_reference_list_bounds_fail_closed() {
    assert!(matches!(
        compound_is_joint_context(&[0, 1], &[9, 11], (2, 0), 10),
        Err(DecodeError::ReferenceState {
            source: DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                index: 2,
                list_len: 2,
            }
        })
    ));
}

#[cfg(test)]
#[test]
fn compound_is_joint_context_keeps_reference_slot_bounds_fail_closed() {
    assert!(matches!(
        compound_is_joint_context(&[2], &[9, 11], (0, 0), 10),
        Err(DecodeError::ReferenceState {
            source: DecodeReferenceStateError::SlotOutOfRange {
                slot: 2,
                slot_count: 2,
            }
        })
    ));
}

fn compound_is_joint_context_from_order_hints(
    first_order_hint: CompoundOrderHint,
    second_order_hint: CompoundOrderHint,
    current_order_hint: CompoundOrderHint,
) -> usize {
    let first_side = first_order_hint.relative_dist(current_order_hint);
    let second_side = second_order_hint.relative_dist(current_order_hint);
    let first_dist = first_side.abs();
    let second_dist = second_side.abs();
    let same_side = (first_side < 0 && second_side < 0) || (first_side > 0 && second_side > 0);
    let one_restricted = first_order_hint.is_restricted() != second_order_hint.is_restricted();
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
    let use_ddt = enable_inter_ddt && !use_intrabc;
    let blocks = residual
        .blocks(residual_blocks)
        .ok_or(inter_internal!("inter_residual_block_range", offset))?;
    for (index, block) in blocks.iter().enumerate() {
        if block.cctx_pair_delta < 0 {
            continue;
        }
        let cctx_type = block.coeffs.cctx_type.unwrap_or(0);
        if block.plane == ReconPlaneId::U && cctx_type != 0 {
            let v_block = inter_residual_chroma_pair(blocks, index, block)
                .ok_or(DecodeHeaderStateError::MissingInterResidualCctxPair)?;
            reconstruct_inter_residual_chroma_cctx_pair(
                scratch,
                sink,
                [block, v_block],
                qindex,
                cctx_type,
                use_ddt,
                bit_depth,
            )
            .map_err(inter_residual_reconstruction_error)?;
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
        .map_err(inter_residual_reconstruction_error)?;
    }
    Ok(())
}

fn inter_residual_reconstruction_error(error: GeneralIntraResidualError) -> DecodeError {
    match error {
        GeneralIntraResidualError::Reconstruct { source } => source.into(),
        _ => DecodeHeaderStateError::InvalidInterResidualReconstruction.into(),
    }
}

#[cfg(test)]
#[test]
fn inter_residual_reconstruction_failures_keep_their_public_error_category() {
    assert!(matches!(
        inter_residual_reconstruction_error(GeneralIntraResidualError::UnexpectedBranch),
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::InvalidInterResidualReconstruction,
        }
    ));
    assert!(matches!(
        inter_residual_reconstruction_error(GeneralIntraResidualError::Reconstruct {
            source: splot_recon::ReconError::ArithmeticOverflow {
                context: "inter residual test",
            },
        }),
        DecodeError::Reconstruction {
            source: splot_recon::ReconError::ArithmeticOverflow {
                context: "inter residual test",
            },
        }
    ));
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
    FrameSegmentIdMap,
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
    pub(crate) ref_num_total_refs: Vec<u32>,
    pub(crate) saved_global_motion_order_hints:
        Vec<splot_core::headers::frame::SavedGlobalMotionOrderHints>,
    pub(crate) saved_global_motion_params: Vec<splot_core::headers::frame::SavedGlobalMotionParams>,
    pub(crate) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    pub(crate) lr_frame_filter_taps: Vec<SlotFrameFilterTaps>,
    pub(crate) ref_frame_cdfs: Vec<Option<FrameCdfHandle>>,
    pub(crate) ref_ccso_params: Vec<Option<Arc<splot_core::headers::frame::CcsoParams>>>,
    pub(crate) ref_ccso_unit_grids: Vec<Option<CcsoGridHandle>>,
    pub(crate) ref_segment_ids: Vec<Option<SegmentIdMapHandle>>,
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
            ref_num_total_refs: Vec::new(),
            saved_global_motion_order_hints: Vec::new(),
            saved_global_motion_params: Vec::new(),
            lr_frame_filter_class_counts: Vec::new(),
            lr_frame_filter_taps: Vec::new(),
            ref_frame_cdfs: Vec::new(),
            ref_ccso_params: Vec::new(),
            ref_ccso_unit_grids: Vec::new(),
            ref_segment_ids: Vec::new(),
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
            ref_num_total_refs: metadata.ref_num_total_refs,
            saved_global_motion_order_hints: metadata.saved_global_motion_order_hints,
            saved_global_motion_params: metadata.saved_global_motion_params,
            lr_frame_filter_class_counts: metadata.lr_frame_filter_class_counts,
            lr_frame_filter_taps: metadata.lr_frame_filter_taps,
            ref_frame_cdfs: metadata.ref_frame_cdfs,
            ref_ccso_params: metadata.ref_ccso_params,
            ref_ccso_unit_grids: metadata.ref_ccso_unit_grids,
            ref_segment_ids: metadata.ref_segment_ids,
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
            ref_segment_ids: std::mem::take(&mut self.ref_segment_ids),
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

    fn cdfs_for_slot(&self, slot: u32) -> Result<Arc<FrameCdfSubset>> {
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        self.ref_frame_cdfs
            .get(slot)
            .and_then(Option::as_ref)
            .and_then(FrameCdfHandle::product)
            .cloned()
            .ok_or(DecodeReferenceStateError::MissingCdfContext { slot }.into())
    }

    fn ccso_params_for_slot(
        &self,
        slot: u32,
    ) -> Result<Arc<splot_core::headers::frame::CcsoParams>> {
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        self.ref_ccso_params
            .get(slot)
            .and_then(Clone::clone)
            .ok_or(DecodeReferenceStateError::MissingCcsoParams { slot }.into())
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
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    if envelope.header.obu_type.is_sef() || envelope.header.obu_type.is_tip_frame() {
        parse_sef_or_tip_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            None,
            frame_index,
        )
    } else {
        parse_inter_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            None,
            frame_index,
        )
    }
}

pub(crate) fn parse_validated_inter_frame_core_with_mfh(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    let mut core = if envelope.header.obu_type.is_sef() || envelope.header.obu_type.is_tip_frame() {
        parse_sef_or_tip_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
            frame_index,
        )?
    } else {
        parse_inter_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
            frame_index,
        )?
    };
    if envelope.header.obu_type.is_sef() {
        validate_sef_frame_core(&core, reference, envelope.offset, frame_index)?;
    } else if envelope.header.obu_type.is_tip_frame() {
        validate_tip_output_frame_parse(&core, envelope.offset, frame_index)?;
        infer_tip_output_quantization(
            &mut core,
            sequence,
            reference,
            envelope.offset,
            frame_index,
        )?;
        validate_tip_output_frame_core(&core)?;
    } else {
        validate_and_resolve_inter_frame_core(
            &mut core,
            sequence,
            reference,
            envelope.offset,
            frame_index,
        )?;
    }
    Ok(core)
}

fn validate_and_resolve_inter_frame_core(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    validate_ras_reference_ids(core, reference, offset, frame_index)?;
    validate_inter_frame_parse(core, offset, frame_index)?;
    resolve_ccso_reference_reuse(core, reference, offset, frame_index)?;
    validate_inter_frame_core(core, sequence, offset)?;
    if core.inter.as_ref().is_some_and(|inter| {
        matches!(
            inter.tip_frame_mode,
            Some(TipFrameMode::AsRef | TipFrameMode::AsOutput)
        )
    }) {
        required_tip_reference_pair(core, reference, offset, frame_index)?;
    }
    Ok(())
}

fn parse_sef_or_tip_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
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
    let core = parse_frame_header_core(&mut reader, &input).map_err(|error| {
        malformed_frame_header(envelope.offset, frame_index, SPEC_HEADER, error.to_string())
    })?;
    if envelope.header.obu_type.is_sef()
        && core.status == FrameHeaderParseStatus::StoppedInsideShowExistingFrame
    {
        return Err(malformed_frame_header(
            envelope.offset,
            frame_index,
            SPEC_HEADER,
            "show-existing frame header ends inside film_grain_config()".to_owned(),
        ));
    }
    Ok(core)
}

fn malformed_frame_header(
    offset: ByteOffset,
    frame_index: Option<usize>,
    spec_section: &'static str,
    message: String,
) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::frame_header_conformance(
            offset,
            frame_index,
            spec_section,
            message,
        ),
    }
}

fn validate_sef_frame_core(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    if let Some(trailing_bits) = core.sef_trailing_bits
        && let Some(message) = trailing_bits.violation_message()
    {
        let spec_section = if trailing_bits == SefTrailingBits::Empty {
            "6.2.1"
        } else {
            "6.2.3"
        };
        return Err(malformed_frame_header(
            offset,
            frame_index,
            spec_section,
            format!("show-existing-frame trailing_bits() are malformed: {message}"),
        ));
    }
    if let Some(slot) = core.frame_to_show_map_idx
        && usize::try_from(slot).map_or(true, |slot| slot >= reference.ref_valid.len())
    {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            "6.17.2",
            format!(
                "show-existing-frame reference slot {slot} is outside the active {}-slot buffer",
                reference.ref_valid.len()
            ),
        ));
    }
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
        return Err(DecodeHeaderStateError::IncompleteShowExistingFrame.into());
    }
    Ok(())
}

fn required_tip_reference_pair(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<find_mv_stack::TipReferencePair> {
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let hints = find_mv_stack::reference_order_hints(
        &inter.ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
    );
    let current_order_hint = core
        .display_order_hint()
        .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?;
    find_mv_stack::tip_reference_pair_from_hints(current_order_hint, &hints).ok_or_else(|| {
        DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                frame_index,
                SPEC_TIP_TEMPORAL_SCALE,
                "TIP has no usable past/future reference pair".to_owned(),
            ),
        }
    })
}

fn infer_tip_output_quantization(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    let pair = required_tip_reference_pair(core, reference, offset, frame_index)?;
    if core.quantization_params.is_some()
        || sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_tip_explicit_qp)
    {
        return Ok(());
    }
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let past = block_reference_slot(&inter.ref_frame_idx, pair.past_ref)? as usize;
    let future = block_reference_slot(&inter.ref_frame_idx, pair.future_ref)? as usize;
    let quantizer = |slot| -> Result<(u32, i32, i32)> {
        let Some((&base_q_idx, &delta_q_u_ac, &delta_q_v_ac)) = reference
            .ref_base_q_idx
            .get(slot)
            .zip(reference.ref_delta_q_u_ac.get(slot))
            .zip(reference.ref_delta_q_v_ac.get(slot))
            .map(|((base_q_idx, delta_q_u_ac), delta_q_v_ac)| {
                (base_q_idx, delta_q_u_ac, delta_q_v_ac)
            })
        else {
            return Err(DecodeReferenceStateError::MissingQuantizerMetadata { slot }.into());
        };
        Ok((base_q_idx, delta_q_u_ac, delta_q_v_ac))
    };
    let (past_q, past_u, past_v) = quantizer(past)?;
    let (future_q, future_u, future_v) = quantizer(future)?;
    let average_delta = |a: i32, b: i32| a / 2 + b / 2 + ((a % 2 + b % 2 + 1) >> 1);
    core.quantization_params = Some(QuantizationParams::inferred_tip(
        (past_q + future_q + 1) >> 1,
        average_delta(past_u, future_u),
        average_delta(past_v, future_v),
    ));
    Ok(())
}

fn validate_tip_output_frame_core(core: &FrameHeaderCore) -> Result<()> {
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
        return Err(DecodeHeaderStateError::IncompleteTipOutput.into());
    }
    Ok(())
}

fn validate_tip_output_frame_parse(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    validate_frame_header_parse_status(
        core,
        offset,
        frame_index,
        "TIP-output OBU payload ends inside mandatory frame_header_info() syntax",
        "TIP-output frame header requires unsupported parser coverage",
    )?;
    if core.obu_type.is_tip_frame()
        && core.inter.as_ref().and_then(|inter| inter.tip_frame_mode)
            != Some(TipFrameMode::AsOutput)
    {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                frame_index,
                SPEC_FRAME_HEADER_INFO_SEMANTICS,
                "TIP-frame OBU did not compute TipFrameMode == TIP_FRAME_AS_OUTPUT".to_owned(),
            ),
        });
    }
    Ok(())
}

fn resolve_ccso_reference_reuse(
    core: &mut FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
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
            frame_index,
        )?
        else {
            continue;
        };
        let ref_ccso = reference.ccso_params_for_slot(slot)?;
        let Some(ref_plane) = ref_ccso
            .planes
            .get(plane_index)
            .filter(|plane| plane.ccso_planes)
        else {
            return Err(malformed_frame_header(
                offset,
                frame_index,
                "6.17.7.8",
                format!(
                    "CCSO reference slot {slot} has no saved enabled plane {plane_index}; \
                     SavedCcsoPlanes must equal 1 when ccso_ref_idx is present"
                ),
            ));
        };
        if !reuse_ccso {
            continue;
        }
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
    frame_index: Option<usize>,
) -> Result<Option<u32>> {
    if !reuse_ccso && !sb_reuse_ccso {
        return Ok(None);
    }
    let Some(slot) = ref_frame_idx.get(ref_index as usize).copied() else {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            "6.17.7.8",
            format!(
                "ccso_ref_idx {ref_index} must be less than NumTotalRefs {}",
                ref_frame_idx.len()
            ),
        ));
    };
    Ok(Some(slot))
}

fn parse_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<impl ReconSample>,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if envelope.header.obu_type != ObuType::BridgeFrame {
        reader
            .read_bit()
            .map_err(|_| DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    frame_index,
                    SPEC_TILE_GROUP,
                    "tile group payload ends before is_first_tile_group".to_owned(),
                ),
            })?;
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: reference.header_view(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|error| DecodeError::MalformedSource {
        issue: DecodeSourceIssue::frame_header_conformance(
            envelope.offset,
            frame_index,
            SPEC_HEADER,
            error.to_string(),
        ),
    })
}

fn validate_inter_frame_parse(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    validate_frame_header_parse_status(
        core,
        offset,
        frame_index,
        "inter-frame OBU payload ends inside mandatory frame_header_info() syntax",
        "inter-frame header requires unsupported parser coverage",
    )?;
    if core.status == FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(unsupported_at(
            "unsupported_tile_boundary",
            offset,
            "decode runtime does not support intra-only frames carried by tile-group OBUs",
            SPEC_HEADER,
        ));
    }
    Ok(())
}

fn validate_frame_header_parse_status(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_index: Option<usize>,
    truncated_message: &'static str,
    unsupported_message: &'static str,
) -> Result<()> {
    if core.status.is_truncated_in_modeled_region() {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                frame_index,
                "6.2.1",
                truncated_message.to_owned(),
            ),
        });
    }
    match core.status {
        FrameHeaderParseStatus::UnsupportedUntilFeature { feature_id } => Err(unsupported_at(
            feature_id,
            offset,
            unsupported_message,
            SPEC_HEADER,
        )),
        _ => Ok(()),
    }
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
        return Err(DecodeHeaderStateError::IncompleteInterFrame.into());
    }
    if core.order_hint.is_none() {
        return Err(DecodeHeaderStateError::MissingDisplayOrderHint.into());
    }
    let frame_size = core
        .frame_size
        .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
    let width = frame_size.width;
    let height = frame_size.height;
    if width == 0 || height == 0 {
        return Err(DecodeHeaderStateError::ZeroFrameSize.into());
    }
    let incomplete_tools = core.quantization_params.is_none()
        || core.segmentation_params.is_none()
        || core.setup_qm_params.is_none()
        || core.delta_q_params.is_none()
        || core.lossless_info.is_none()
        || sequence.inter.is_none()
        || core.deblocking_filter_params.is_none()
        || core.gdf_params.is_none()
        || core.cdef_params.is_none()
        || core.lr_params.is_none()
        || core.ccso_params.is_none();
    if incomplete_tools {
        return Err(DecodeHeaderStateError::IncompleteInterFrameTools.into());
    }
    Ok(())
}

fn validate_ras_reference_ids(
    core: &FrameHeaderCore,
    reference: &InterReferenceState<impl ReconSample>,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    if core.obu_type != ObuType::RasFrame {
        return Ok(());
    }
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let count = usize::try_from(inter.num_total_refs.unwrap_or(0)).unwrap_or(usize::MAX);
    for &slot in inter.ref_frame_idx.iter().take(count) {
        let slot = slot as usize;
        let Some(id) = reference.ref_long_term_id.get(slot).copied() else {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    offset,
                    frame_index,
                    SPEC_FRAME_HEADER_INFO_SEMANTICS,
                    format!(
                        "RAS reference slot {slot} is outside the active reference map of {} slots",
                        reference.ref_long_term_id.len()
                    ),
                ),
            });
        };
        if id.is_none_or(|id| !core.ref_long_term_ids.contains(&id)) {
            let description = id.map_or_else(
                || "no long-term ID".to_owned(),
                |id| format!("RefLongTermId {id}"),
            );
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    offset,
                    frame_index,
                    SPEC_FRAME_HEADER_INFO_SEMANTICS,
                    format!(
                        "RAS reference slot {slot} has {description}, which is absent from the \
                         frame's listed long-term IDs {:?}",
                        core.ref_long_term_ids
                    ),
                ),
            });
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
        return Err(inter_internal!("bridge_incomplete_state", offset));
    }
    Ok(())
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

pub(crate) use block::{
    InterBlockFacts, InterDecodeScratch, InterFilterInputs, InterFrameParse,
    ScheduledFrameProgress, decode_inter_blocks, parse_inter_frame_blocks,
};
use cross_frame::{ResolvedCdfLoad, resolve_cdf_load};
pub(crate) use find_mv_stack::{MotionFieldLayout, TemporalMotionField, TemporalMvScratch};
pub(crate) use frame_products::{CcsoGridHandle, FrameCdfHandle, SegmentIdMapHandle};
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
