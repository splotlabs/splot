// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

pub(crate) use block::{ParseProgress, ReconRowBuffers, TileParseState};
use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use std::sync::Arc;

use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseStatus, FrameType,
    QuantizationParams, RESTRICTED_ORDER_HINT, RefIdxBuf, SefTrailingBits, SlotFrameFilterTaps,
    TipFrameMode, get_relative_dist, parse_frame_header_core,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::hls::{MfhId, MultiFrameHeaderRecord};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, CurrentFrameSurface, DecodedFrame, DecodedFrameInfo,
    InterpolationFilter as ReconInterpolationFilter, IntraRectBlockSize, PlaneId as ReconPlaneId,
    QuantizerDeltas, ReconSample, ReferenceFrameStore, ReferenceSlot,
};

use crate::bitstream::byte_stream::SourceBytes;
use crate::bitstream::tile_payload::{
    CoeffBlock, FrameCdfSubset, FrameQuantizerDeltasScope, FrameSegmentIdMap,
    GeneralIntraResidualError, ResidualScratch, add_cctx_pair_residual_in_place_with,
    reconstruct_inter_coeff_block_residual_rect_into_with, with_residual_scratch,
};
use crate::error::{DecodeError, DecodeHeaderStateError, DecodeReferenceStateError};
use crate::pipeline::frame_engine::finish::{FilterSinkSetup, FrameWalk, WalkStage};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{derive_visible_luma_rect, ensure_runtime_limits};
use crate::reference::buffer::RefSlots;
use crate::{DecodeIvfFrameContext, DecodeOptions, DecodePlannedObu, DecodeSourceIssue, Result};

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

const _: () = assert!(ReferenceSlot::MAX_SLOTS <= u16::BITS as usize);
const SPEC_TIP_TEMPORAL_SCALE: &str = "7.10.1";
pub(crate) const SINGLE_MODE_NEARMV: u8 = 0;
pub(crate) const SINGLE_MODE_GLOBALMV: u8 = 1;
pub(crate) const SINGLE_MODE_NEWMV: u8 = 2;
/// A motion vector in `1/8` luma samples (§ 7.12).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct MotionVector {
    /// Vertical component.
    pub row: i32,
    /// Horizontal component.
    pub col: i32,
}

impl MotionVector {
    const ZERO: Self = Self { row: 0, col: 0 };
}
fn completed_walk<T: ReconSample>(output: InterDecodeOutput<T>) -> FrameWalk<T> {
    let (frame, core, frame_cdfs, ccso_grid, motion_field, segment_ids) = output;
    FrameWalk {
        stage: WalkStage::complete(frame),
        core,
        frame_cdfs,
        ccso_grid,
        segment_ids,
        motion_field,
    }
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn walk_inter_frame<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    scratch: &mut InterDecodeScratch<T>,
    candidate: &DecodePlannedObu,
    bytes: SourceBytes<'_>,
    frame_envelope: ObuEnvelope<'_>,
    core: Arc<FrameHeaderCore>,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
    products: &mut FrameProductWriters,
) -> Result<FrameWalk<T>> {
    if frame_envelope.header.obu_type == ObuType::BridgeFrame || bru_inactive_frame(&core) {
        reference
            .pixel_reference_gate(named_pixel_reference_slots(&core))
            .wait()?;
        return decode_bridge_frame(
            backend,
            candidate,
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
            products,
        )
        .map(completed_walk);
    }
    let frame_is_intra = core
        .frame_is_intra
        .ok_or(DecodeHeaderStateError::IncompleteInterFrame)?;
    let geometry =
        frame_walk::FrameDecodeGeometry::new(&core, sequence, bit_depth, frame_is_intra)?;
    if frame_envelope.header.obu_type.is_tip_frame() {
        reference
            .pixel_reference_gate(named_pixel_reference_slots(&core))
            .wait()?;
        let planes = scratch.pooled_planes();
        return decode_tip_output_frame(
            backend,
            scratch,
            planes,
            candidate,
            frame_envelope.offset,
            core,
            sequence,
            options,
            reference,
            geometry,
            products,
        )
        .map(completed_walk);
    }
    if let Some(inter) = core.inter.as_ref() {
        for dependency in reference
            .motion_dependencies(&inter.ref_frame_idx)
            .iter()
            .flatten()
        {
            dependency.wait_field();
        }
    }
    let mut payload_scratch = core::mem::take(&mut scratch.payload);
    let frame_walk::InterWalkPrologue {
        tile_plan,
        setup,
        facts,
        ref_frame_idx,
        quantizer_deltas,
    } = frame_walk::derive_inter_walk_prologue(
        candidate,
        bytes,
        &core,
        sequence,
        options,
        reference,
        bit_depth,
        geometry,
        Some(&mut scratch.initial_cdfs),
        &mut payload_scratch,
    )?;
    scratch.payload = payload_scratch;
    let mut recycled = scratch.pooled_planes();
    let workspace = splot_recon::CurrentFrameWorkspace::new_recycled_from(
        geometry.info(),
        scratch.kernels(),
        &mut recycled,
    )?; // the last frame's buffers keep its samples: restore the fill if § 7.11/§ 7.13 ever leave a coded sample unwritten
    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let InterBlockDecodeOutput {
        workspace,
        frame_cdfs,
        filter_inputs,
        segment_ids,
    } = decode_inter_blocks(
        backend,
        scratch,
        tile_plan,
        sequence,
        &core,
        options,
        facts,
        ref_frame_idx.as_slice(),
        reference,
        workspace,
        products,
    )?;
    Ok(setup.frame_walk(
        workspace,
        filter_inputs,
        core,
        frame_cdfs,
        segment_ids,
        true,
    ))
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn decode_tip_output_frame<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    scratch: &mut InterDecodeScratch<T>,
    planes: splot_recon::FramePlaneSamples<T>,
    candidate: &DecodePlannedObu,
    offset: ByteOffset,
    core: Arc<FrameHeaderCore>,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    geometry: FrameDecodeGeometry,
    products: &mut FrameProductWriters,
) -> Result<InterDecodeOutput<T>> {
    let (frame_cdfs, segment_ids) = tip_output_products(
        &core, candidate, offset, sequence, options, reference, &geometry, products,
    )?;
    let (frame, motion_field) = reconstruct_tip_output(
        backend, scratch, planes, &core, sequence, reference, geometry, offset,
    )?;
    Ok((frame, core, frame_cdfs, None, motion_field, segment_ids))
}

/// The entropy products of a TIP output frame, which parses no blocks: the
/// initial CDFs pass through unchanged and the segment-id map is empty. They
/// do not depend on the frame's samples, so a caller may publish them first.
#[expect(clippy::too_many_arguments)]
pub(crate) fn tip_output_products(
    core: &FrameHeaderCore,
    candidate: &DecodePlannedObu,
    offset: ByteOffset,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &ReferenceMetadata,
    geometry: &FrameDecodeGeometry,
    products: &mut FrameProductWriters,
) -> Result<(Arc<FrameCdfSubset>, Arc<FrameSegmentIdMap>)> {
    let frame_size = geometry.frame_size();
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        geometry.info().bit_depth(),
        sequence.general.chroma_format_idc,
    )?;
    let frame_cdfs =
        resolve_initial_frame_cdfs(core, sequence, reference, Some(candidate), offset)?;
    products.frame_cdfs()?.reset_output_from(&frame_cdfs);
    let frame_cdfs = products.cdf_output()?;
    let (mi_rows, mi_cols) = geometry.mi_dimensions();
    products.segment_ids(mi_rows, mi_cols, false)?;
    let segment_ids = products.finish_segment_ids()?;
    Ok((frame_cdfs, segment_ids))
}

/// Predicts a TIP output frame's samples and its stored motion field.
#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_tip_output<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    scratch: &mut InterDecodeScratch<T>,
    planes: splot_recon::FramePlaneSamples<T>,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<T>,
    geometry: FrameDecodeGeometry,
    offset: ByteOffset,
) -> Result<(DecodedFrame<T>, TemporalMotionField)> {
    block::tip::reconstruct_output(
        backend, scratch, planes, sequence, core, reference, geometry, offset,
    )
}

#[expect(clippy::too_many_arguments)]
fn decode_bridge_frame<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    core: Arc<FrameHeaderCore>,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
    products: &mut FrameProductWriters,
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
    let ref_slot = core
        .bridge_frame_ref_idx
        .or_else(|| bru_ref_slot(&core))
        .ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
    let source = hold_reference_slot(reference, ref_slot)?;
    let ref_slot_index = usize::try_from(ref_slot).unwrap_or(usize::MAX);
    let ref_order_hint_count = reference.ref_order_hint.len();
    let reference_order_hint = reference
        .ref_order_hint
        .get(ref_slot_index)
        .copied()
        .ok_or(DecodeReferenceStateError::SlotOutOfRange {
            slot: ref_slot_index,
            slot_count: ref_order_hint_count,
        })?;
    let current_order_hint = core
        .display_order_hint()
        .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?;
    let bru_ref_hints = bru_inactive_frame(&core)
        .then(|| {
            core.inter.as_ref().map(|inter| {
                find_mv_stack::reference_order_hints(
                    &inter.ref_frame_idx,
                    &reference.ref_valid,
                    &reference.ref_order_hint,
                )
            })
        })
        .flatten();
    let single_hint = [Some(reference_order_hint)];
    let ref_order_hints = bru_ref_hints.as_deref().unwrap_or(&single_hint);
    let motion_field = bridge::motion_field(
        frame_size,
        current_order_hint,
        reference_order_hint,
        ref_order_hints,
    )
    .ok_or(inter_allocation!("bridge temporal motion field"))?;
    let frame_cdfs =
        resolve_initial_frame_cdfs(&core, sequence, reference, Some(candidate), offset)?;
    let visible = derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    let frame = bridge::reconstruct(backend, &source, frame_size, visible, 0, offset)?;
    products.frame_cdfs()?.reset_output_from(&frame_cdfs);
    let frame_cdfs = products.cdf_output()?;
    let (mi_rows, mi_cols) = segment_id_map_dimensions(&core)?;
    products.segment_ids(mi_rows, mi_cols, false)?;
    let segment_ids = products.finish_segment_ids()?;
    Ok((frame, core, frame_cdfs, None, motion_field, segment_ids))
}

#[cfg(test)]
fn empty_segment_id_map(core: &FrameHeaderCore) -> Result<FrameSegmentIdMap> {
    let (mi_rows, mi_cols) = segment_id_map_dimensions(core)?;
    FrameSegmentIdMap::new(mi_rows, mi_cols)
        .map_err(|_| DecodeHeaderStateError::MissingSegmentIdMap.into())
}

fn segment_id_map_dimensions(core: &FrameHeaderCore) -> Result<(usize, usize)> {
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
    Ok((mi_rows, mi_cols))
}

fn frame_cdf_load(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
) -> ResolvedCdfLoad {
    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint =
        i32::try_from(core.display_order_hint().unwrap_or(0)).unwrap_or(i32::MAX);
    let Some(inter_ctrl) = core.inter.as_ref() else {
        return ResolvedCdfLoad::Default;
    };
    let (enable_avg_cdf, avg_cdf_type) = sequence
        .transform_quant_entropy
        .as_ref()
        .map_or((false, 1u8), |tq| (tq.enable_avg_cdf, tq.avg_cdf_type));
    resolve_cdf_load(
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
        cdf_blending_enabled(
            enable_avg_cdf,
            inter_ctrl.tip_frame_mode,
            inter_ctrl.bru_inactive == Some(true),
        ),
        avg_cdf_type,
    )
}

/// The CDFs frame `core` starts from (§ 5.18.2 `set_primary_ref_frame_and_ctx`):
/// the defaults, or the saved CDFs of the primary reference blended with the
/// secondary one. The encoder facade calls it with no `candidate`.
pub(crate) fn resolve_initial_frame_cdfs(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    candidate: Option<&DecodePlannedObu>,
    offset: ByteOffset,
) -> Result<Arc<FrameCdfSubset>> {
    match frame_cdf_load(core, sequence, reference) {
        ResolvedCdfLoad::Default => Ok(Arc::new(FrameCdfSubset::default_for_base_q(
            core.quantization_params.map_or(0, |q| q.base_q_idx),
        ))),
        ResolvedCdfLoad::OutOfRangePrimary {
            index,
            reference_count,
        } => Err(primary_out_of_range(
            candidate,
            offset,
            index,
            reference_count,
        )),
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

fn primary_out_of_range(
    candidate: Option<&DecodePlannedObu>,
    offset: ByteOffset,
    index: u8,
    reference_count: usize,
) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::frame_header_conformance(
            offset,
            candidate
                .and_then(DecodePlannedObu::ivf_frame)
                .map(DecodeIvfFrameContext::frame_index),
            SPEC_HEADER_SEMANTICS,
            format!(
                "primary reference index {index} is outside the active \
                 {reference_count}-entry map"
            ),
        ),
    }
}

fn resolve_initial_frame_cdfs_reusing(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    candidate: &DecodePlannedObu,
    offset: ByteOffset,
    storage: &mut Option<Arc<FrameCdfSubset>>,
) -> Result<Arc<FrameCdfSubset>> {
    let cdf_load = frame_cdf_load(core, sequence, reference);
    let primary = match cdf_load {
        ResolvedCdfLoad::OutOfRangePrimary {
            index,
            reference_count,
        } => {
            return Err(primary_out_of_range(
                Some(candidate),
                offset,
                index,
                reference_count,
            ));
        }
        ResolvedCdfLoad::Default => None,
        ResolvedCdfLoad::LoadSlot {
            primary,
            blend: None,
        } => return reference.cdfs_for_slot(primary),
        ResolvedCdfLoad::LoadSlot {
            primary,
            blend: Some(_),
        } => Some(reference.cdfs_for_slot(primary)?),
    };
    if storage.is_none() {
        *storage = Some(Arc::new(FrameCdfSubset::from_defaults()));
    }
    let output = storage
        .as_mut()
        .and_then(Arc::get_mut)
        .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
    if let Some(primary) = primary.as_deref() {
        output.reset_from(primary);
    } else {
        output.reset_to_defaults(core.quantization_params.map_or(0, |q| q.base_q_idx));
    }
    if let ResolvedCdfLoad::LoadSlot {
        blend: Some(blend), ..
    } = cdf_load
    {
        output.blend_from_saved(reference.cdfs_for_slot(blend)?.as_ref());
    }
    storage
        .as_ref()
        .map(Arc::clone)
        .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
}

/// Exact pending entropy products one frame's tile parse may consume.
#[derive(Default)]
pub(crate) struct EntropyDependencies {
    /// The primary and optional blend CDF slots.
    cdfs: [Option<FrameCdfHandle>; 2],
    ccso_grids: [Option<CcsoGridHandle>; 3],
    segment_ids: Option<SegmentIdMapHandle>,
}

impl EntropyDependencies {
    /// Whether every product this parse reads has been published or failed.
    /// The reused CCSO grids are not among them: the parse reads no flag of
    /// a reused plane (§ 5.20.10.2).
    pub(crate) fn is_settled(&self) -> bool {
        self.cdfs.iter().flatten().all(FrameCdfHandle::is_settled)
            && self.segment_ids.iter().all(SegmentIdMapHandle::is_settled)
    }

    /// Whether the grids the frame's CCSO planes reuse have been published
    /// or failed.
    pub(crate) fn ccso_settled(&self) -> bool {
        self.ccso_grids
            .iter()
            .flatten()
            .all(CcsoGridHandle::is_settled)
    }

    /// The grid each CCSO plane reuses, once [`Self::ccso_settled`].
    pub(crate) fn reused_ccso(&self) -> [Option<&crate::filters::ccso::CcsoUnitGrid>; 3] {
        reused_ccso_sources(&self.ccso_grids)
    }
}

/// The published grid behind each handle of [`reused_ccso_grids`].
pub(crate) fn reused_ccso_sources(
    grids: &[Option<CcsoGridHandle>; 3],
) -> [Option<&crate::filters::ccso::CcsoUnitGrid>; 3] {
    grids.each_ref().map(|handle| {
        handle
            .as_ref()
            .and_then(CcsoGridHandle::product)
            .and_then(Option::as_deref)
    })
}

/// Resolves entropy-product identities before the current frame refreshes slots.
pub(crate) fn entropy_dependencies(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
) -> EntropyDependencies {
    let mut cdfs = [None, None];
    if let ResolvedCdfLoad::LoadSlot { primary, blend } = frame_cdf_load(core, sequence, reference)
    {
        for (held, slot) in cdfs.iter_mut().zip([Some(primary), blend]) {
            *held = slot
                .and_then(|slot| reference.ref_frame_cdfs.get(slot as usize))
                .and_then(Option::as_ref)
                .cloned();
        }
    }

    let ccso_grids = reused_ccso_grids(core, sequence, reference);
    let needs_previous_segment_ids = core.segmentation_params.as_ref().is_some_and(|seg| {
        seg.segmentation_enabled
            && (!seg.segmentation_update_map || seg.segmentation_temporal_update)
    });
    let segment_ids = needs_previous_segment_ids
        .then(|| previous_segment_slot(core, reference))
        .flatten()
        .and_then(|slot| reference.ref_segment_ids.get(slot))
        .and_then(Option::as_ref)
        .cloned();
    EntropyDependencies {
        cdfs,
        ccso_grids,
        segment_ids,
    }
}

/// The saved grid handle each CCSO plane's `sb_reuse_ccso` copies from.
pub(crate) fn reused_ccso_grids(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
) -> [Option<CcsoGridHandle>; 3] {
    let mut ccso_grids = [None, None, None];
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
        for (held, plane) in ccso_grids.iter_mut().zip(&ccso.planes) {
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
                *held = Some(handle.clone());
            }
        }
    }
    ccso_grids
}

fn previous_segment_slot(core: &FrameHeaderCore, reference: &ReferenceMetadata) -> Option<usize> {
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

/// The segment-ID product of the frame § 7.21 `load_previous_segment_ids`
/// reads, which the entropy pass holds in place of the typed references.
fn previous_segment_handle<'a>(
    core: &FrameHeaderCore,
    reference: &'a ReferenceMetadata,
) -> Option<&'a SegmentIdMapHandle> {
    if !core
        .segmentation_params
        .as_ref()
        .is_some_and(|seg| seg.segmentation_enabled)
    {
        return None;
    }
    let slot = previous_segment_slot(core, reference)?;
    reference.ref_segment_ids.get(slot)?.as_ref()
}

fn previous_segment_ids(
    previous: Option<&SegmentIdMapHandle>,
    mi_rows: usize,
    mi_cols: usize,
) -> Option<&Arc<FrameSegmentIdMap>> {
    previous?
        .product()
        .filter(|map| map.dimensions() == (mi_rows, mi_cols))
}

const fn cdf_blending_enabled(
    enable_avg_cdf: bool,
    tip_frame_mode: Option<TipFrameMode>,
    bru_inactive: bool,
) -> bool {
    enable_avg_cdf && !matches!(tip_frame_mode, Some(TipFrameMode::AsOutput)) && !bru_inactive
}

/// Reference lists a frame header can name: `REFS_PER_FRAME` (AV2 § 3) plus
/// the bridge slot.
const ROW_LISTS: usize = 8;

/// Each reference list's samples with its per-frame scalings, borrowed once
/// for a row of blocks, or `None` where the list's slot lent no samples then.
///
/// [`hold_row_references`] runs when the row's reconstruction starts, after
/// its reference admission: the row gate has seen every list the row reads
/// publish the rows its blocks reach, or settle. A published row is final and
/// the watermark only grows, so the borrowed prefix covers every block of the
/// row however far the reference publishes or freezes meanwhile; each read
/// still checks it.
pub(in crate::prediction::inter) type RowReferences<'a, T> =
    [Option<reference::ReferenceSamples<'a, T>>; ROW_LISTS];

/// A block's reference lists: the slots they name, their per-frame facts and
/// the row borrows of their samples.
pub(in crate::prediction::inter) type BlockLists<'a, T> = (
    &'a [u32],
    &'a block::FrameRefFacts,
    &'a RowReferences<'a, T>,
);

/// Borrows the slot of each list of `ref_frame_idx` for one row of blocks.
pub(in crate::prediction::inter) fn hold_row_references<'a, T: ReconSample>(
    reference: &'a InterReferenceState<T>,
    (ref_frame_idx, facts): (&[u32], &'a block::FrameRefFacts),
) -> RowReferences<'a, T> {
    core::array::from_fn(|list| {
        let scalings = facts.scalings(i8::try_from(list).ok()?)?;
        let held = reference.slot(*ref_frame_idx.get(list)?)?.hold_samples()?;
        Some(held.samples(scalings))
    })
}

/// The row borrow of list `ref_frame`, when it has one.
#[inline]
fn row_reference<'a, T: ReconSample>(
    row: &RowReferences<'a, T>,
    ref_frame: i8,
) -> Option<reference::ReferenceSamples<'a, T>> {
    *row.get(usize::try_from(ref_frame).ok()?)?
}

/// The reference samples one inter block reads.
pub(in crate::prediction::inter) struct HeldInterBlockReferences<'a, T: ReconSample> {
    reference0: reference::ReferenceSamples<'a, T>,
    /// The second list's samples; `reference0` for a single-reference block.
    reference1: reference::ReferenceSamples<'a, T>,
    compound: bool,
}

/// Borrows the reference frames one placed inter block names, from the row
/// borrows of `row` when both lists have one.
///
/// # Errors
///
/// Returns a typed reference-state diagnostic when a named slot has no readable
/// frame samples or `facts` no scalings for it.
#[inline]
pub(in crate::prediction::inter) fn hold_inter_block_references<'a, T: ReconSample>(
    (ref_frame_idx, facts, row): BlockLists<'a, T>,
    reference: &'a InterReferenceState<T>,
    placed: &PlacedInterBlock,
) -> Result<HeldInterBlockReferences<'a, T>> {
    let (ref_frame0, ref_frame1) = (placed.block.ref_frame0, placed.block.ref_frame1);
    if let Some(reference0) = row_reference(row, ref_frame0) {
        let reference1 = match ref_frame1 {
            None => Some(reference0),
            Some(ref_frame1) => row_reference(row, ref_frame1),
        };
        if let Some(reference1) = reference1 {
            return Ok(HeldInterBlockReferences {
                reference0,
                reference1,
                compound: ref_frame1.is_some(),
            });
        }
    }
    let slot0 = block_reference_slot(ref_frame_idx, ref_frame0)?;
    let slot1 = ref_frame1
        .map(|ref_frame1| block_reference_slot(ref_frame_idx, ref_frame1))
        .transpose()?;
    let (held0, held1) = hold_reference_pair(reference, slot0, slot1)?;
    let scalings = |ref_frame: i8, slot: u32| {
        facts
            .scalings(ref_frame)
            .ok_or(DecodeReferenceStateError::ReferenceSamplesUnavailable {
                slot: slot as usize,
            })
    };
    let scaling0 = scalings(ref_frame0, slot0)?;
    let scaling1 = match (ref_frame1, slot1) {
        (Some(ref_frame1), Some(slot1)) => scalings(ref_frame1, slot1)?,
        _ => scaling0,
    };
    Ok(HeldInterBlockReferences {
        reference0: held0.samples(scaling0),
        reference1: held1.unwrap_or(held0).samples(scaling1),
        compound: slot1.is_some(),
    })
}

impl<'a, T: ReconSample> HeldInterBlockReferences<'a, T> {
    pub(in crate::prediction::inter) const fn reference0_samples(
        &self,
    ) -> reference::ReferenceSamples<'a, T> {
        self.reference0
    }

    /// Predicts the held block, which does not refine its motion, into
    /// `sink`: a single-reference translation straight from its motion vector.
    ///
    /// # Errors
    ///
    /// As [`mc::motion_compensate_single_block_into`] and
    /// [`mc::predict_inter_block`].
    pub(in crate::prediction::inter) fn predict_into(
        &self,
        backend: splot_core::simd::SimdBackend,
        sink: &mut CurrentFrameSurface<'_, '_, T>,
        placed: &PlacedInterBlock,
        rect: mc::McBlockRect,
        offset: ByteOffset,
    ) -> Result<()> {
        if !self.compound && placed.block.warp_params[0].is_none() {
            return mc::motion_compensate_single_block_into(
                backend,
                sink,
                self.reference0,
                rect,
                placed.block.mv,
                placed.block.interp,
                placed.predict_chroma,
                offset,
            );
        }
        mc::predict_inter_block(backend, sink, &self.block_params(placed, rect), offset)
    }

    /// Builds the § 7.13.3 prediction parameters for the held block.
    pub(in crate::prediction::inter) fn block_params(
        &self,
        placed: &PlacedInterBlock,
        rect: mc::McBlockRect,
    ) -> mc::InterBlockParams<'a, T> {
        let ref_frame0 = self.reference0;
        if self.compound {
            mc::InterBlockParams::compound_average(
                ref_frame0,
                self.reference1,
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
        }
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

/// Borrows two named references in list order, once when both name one slot.
pub(in crate::prediction::inter) fn hold_reference_pair<T: ReconSample>(
    reference: &InterReferenceState<T>,
    first: u32,
    second: Option<u32>,
) -> Result<(
    reference::HeldFrameSamples<'_, T>,
    Option<reference::HeldFrameSamples<'_, T>>,
)> {
    let first_held = hold_reference_slot(reference, first)?;
    let second = second
        .filter(|second| *second != first)
        .map(|second| hold_reference_slot(reference, second))
        .transpose()?;
    Ok((first_held, second))
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
        if value == RESTRICTED_ORDER_HINT {
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
#[expect(clippy::too_many_arguments)]
pub(in crate::prediction::inter) fn add_inter_residual_to_workspace<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    sink: &mut CurrentFrameSurface<'_, '_, T>,
    residual: &InterResidual,
    residual_blocks: &[InterResidualBlock],
    residual_coeffs: &[i32],
    qindex: u32,
    luma_use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<()> {
    let blocks = residual
        .blocks(residual_blocks)
        .ok_or(DecodeHeaderStateError::InvalidInterResidualReconstruction)?;
    if blocks.is_empty() {
        return Ok(());
    }
    let sample_depth = sink.info().bit_depth();
    with_residual_scratch(|scratch| {
        for (index, block) in blocks.iter().enumerate() {
            if block.cctx_pair_delta < 0 {
                continue;
            }
            if block.plane == ReconPlaneId::U
                && let Some(cctx_type) = block.coeffs.cctx_type().filter(|&kind| kind != 0)
            {
                let v_block = inter_residual_chroma_pair(blocks, index, block)
                    .ok_or(DecodeHeaderStateError::MissingInterResidualCctxPair)?;
                reconstruct_inter_residual_chroma_cctx_pair(
                    backend,
                    sink,
                    scratch,
                    [block, v_block],
                    residual_coeffs,
                    qindex,
                    cctx_type,
                    use_ddt,
                    bit_depth,
                )
                .map_err(inter_residual_reconstruction_error)?;
                continue;
            }
            let (log2_width, log2_height) = inter_residual_log2(block)
                .map_err(|_| DecodeHeaderStateError::InvalidInterResidualReconstruction)?;
            if block.coeffs.eob == 0 {
                #[cfg(feature = "block-evidence")]
                crate::block_evidence::stages::skip_inter_residual(
                    block,
                    (log2_width, log2_height),
                );
                continue;
            }
            let coeffs = CoeffBlock::new(&block.coeffs, residual_coeffs)
                .map_err(|_| DecodeHeaderStateError::InvalidInterResidualReconstruction)?;
            let block_size = IntraRectBlockSize::new(log2_width as u8, log2_height as u8)?;
            reconstruct_inter_coeff_block_residual_rect_into_with(
                backend,
                sink,
                scratch,
                coeffs,
                block.plane,
                usize::from(block.x),
                usize::from(block.y),
                block_size,
                qindex,
                block.plane == ReconPlaneId::Y && luma_use_tcq,
                use_ddt,
                sample_depth,
            )
            .map_err(inter_residual_reconstruction_error)?;
        }
        Ok(())
    })
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
        inter_residual_reconstruction_error(
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "inter residual test",
            },
        ),
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
    v.plane == ReconPlaneId::V && u.x == v.x && u.y == v.y && u.tx_size == v.tx_size
}

#[expect(clippy::too_many_arguments)]
#[inline(never)]
fn reconstruct_inter_residual_chroma_cctx_pair<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    sink: &mut CurrentFrameSurface<'_, '_, T>,
    scratch: &mut ResidualScratch,
    [u, v]: [&InterResidualBlock; 2],
    residual_coeffs: &[i32],
    qindex: u32,
    cctx_type: usize,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (log2_width, log2_height) = inter_residual_log2(u).map_err(|table| {
        GeneralIntraResidualError::TransformPartitionGeometry {
            table,
            index: usize::from(u.tx_size),
        }
    })?;
    let size = splot_recon::IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let coeffs = crate::bitstream::tile_payload::CoeffBlock::new;
    add_cctx_pair_residual_in_place_with(
        backend,
        sink,
        scratch,
        [
            coeffs(&u.coeffs, residual_coeffs)?,
            coeffs(&v.coeffs, residual_coeffs)?,
        ],
        (usize::from(u.x), usize::from(u.y), size),
        qindex,
        cctx_type,
        use_ddt,
        bit_depth,
    )
}

fn inter_residual_log2(
    block: &InterResidualBlock,
) -> core::result::Result<(u32, u32), &'static str> {
    let shape = crate::tile::block_context::TxShape::from_tx_size(usize::from(block.tx_size))
        .ok_or("Tx_Width_Log2")?;
    Ok((shape.width_log2(), shape.height_log2()))
}
#[derive(Clone, Debug)]
pub(crate) struct InterBlock {
    pub(crate) ref_frame0: i8,
    pub(crate) ref_frame1: Option<i8>,
    pub(crate) mv: MotionVector,
    pub(crate) mv1: MotionVector,
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
    Arc<FrameHeaderCore>,
    Arc<FrameCdfSubset>,
    Option<Arc<crate::filters::ccso::CcsoUnitGrid>>,
    TemporalMotionField,
    Arc<FrameSegmentIdMap>,
);

#[derive(Clone, Debug)]
pub(crate) struct PlacedInterBlock<B = InterBlock> {
    pub(crate) luma: [u32; 4],
    pub(crate) chroma_luma: [u32; 4],
    pub(crate) predict_chroma: bool,
    pub(crate) sub8x8_chroma: bool,
    pub(crate) interintra_chroma: bool,
    pub(crate) interintra_edges: [crate::pipeline::reconstruct::IntraEdgeAvailability; 2],
    pub(crate) block: B,
}

impl<B> PlacedInterBlock<B> {
    pub(in crate::prediction::inter) fn with_block<U>(&self, block: U) -> PlacedInterBlock<U> {
        PlacedInterBlock {
            luma: self.luma,
            chroma_luma: self.chroma_luma,
            predict_chroma: self.predict_chroma,
            sub8x8_chroma: self.sub8x8_chroma,
            interintra_chroma: self.interintra_chroma,
            interintra_edges: self.interintra_edges,
            block,
        }
    }

    pub(crate) const fn luma_x(&self) -> usize {
        self.luma[0] as usize
    }

    pub(crate) const fn luma_y(&self) -> usize {
        self.luma[1] as usize
    }

    pub(crate) const fn luma_w(&self) -> usize {
        self.luma[2] as usize
    }

    pub(crate) const fn luma_h(&self) -> usize {
        self.luma[3] as usize
    }

    pub(crate) const fn chroma_luma_x(&self) -> usize {
        self.chroma_luma[0] as usize
    }

    pub(crate) const fn chroma_luma_y(&self) -> usize {
        self.chroma_luma[1] as usize
    }

    pub(crate) const fn chroma_luma_w(&self) -> usize {
        self.chroma_luma[2] as usize
    }

    pub(crate) const fn chroma_luma_h(&self) -> usize {
        self.chroma_luma[3] as usize
    }

    pub(in crate::prediction::inter) const fn motion_compensation_rect(&self) -> mc::McBlockRect {
        mc::McBlockRect::from_luma_rect(self.luma_x(), self.luma_y(), self.luma_w(), self.luma_h())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct InterResidual {
    pub(crate) block_range: core::ops::Range<u32>,
}

impl InterResidual {
    /// The arena blocks `range`; a row holds far fewer than `u32::MAX`.
    pub(crate) fn new(range: core::ops::Range<usize>) -> Self {
        Self {
            block_range: range.start as u32..range.end as u32,
        }
    }

    pub(crate) fn blocks<'a>(
        &self,
        arena: &'a [InterResidualBlock],
    ) -> Option<&'a [InterResidualBlock]> {
        arena.get(self.block_range.start as usize..self.block_range.end as usize)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InterResidualBlock {
    pub(crate) plane: ReconPlaneId,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) tx_size: u8,
    pub(crate) cctx_pair_delta: i16,
    pub(crate) coeffs: crate::bitstream::tile_payload::LumaCoeffBlock,
}
/// The reference frames one frame's decode reads: their samples in `store`,
/// and everything else in [`ReferenceMetadata`], which it dereferences to.
pub(crate) struct InterReferenceState<T: ReconSample> {
    store: ReferenceFrameStore<RefFrameSlot<T>>,
    pub(crate) metadata: ReferenceMetadata,
}

impl<T: ReconSample> core::ops::Deref for InterReferenceState<T> {
    type Target = ReferenceMetadata;

    fn deref(&self) -> &ReferenceMetadata {
        &self.metadata
    }
}

impl<T: ReconSample> core::ops::DerefMut for InterReferenceState<T> {
    fn deref_mut(&mut self) -> &mut ReferenceMetadata {
        &mut self.metadata
    }
}

/// What a frame's decode reads of its reference frames apart from their
/// samples. It names no sample type, so its readers compile once for every
/// bit depth.
#[derive(Default)]
pub(crate) struct ReferenceMetadata {
    pub(crate) ref_valid: RefSlots<bool>,
    pub(crate) ref_order_hint: RefSlots<u32>,
    pub(crate) ref_order_hint_lsbs: RefSlots<u32>,
    pub(crate) ref_implicit_output_frame: RefSlots<bool>,
    pub(crate) ref_immediate_output_frame: RefSlots<bool>,
    pub(crate) ref_frame_width: RefSlots<u32>,
    pub(crate) ref_frame_height: RefSlots<u32>,
    pub(crate) ref_base_q_idx: RefSlots<u32>,
    pub(crate) ref_counter: RefSlots<u32>,
    pub(crate) ref_chroma_ac_deltas: RefSlots<[i32; 2]>,
    pub(crate) ref_is_inter: RefSlots<bool>,
    /// The § 7.9 `RefFrameType` is `KEY_FRAME` or `INTRA_ONLY_FRAME`.
    pub(crate) ref_is_intra_frame: RefSlots<bool>,
    pub(crate) ref_long_term_id: RefSlots<Option<u32>>,
    pub(crate) ref_tlayer_id: RefSlots<u8>,
    pub(crate) ref_mlayer_id: RefSlots<u8>,
    pub(crate) ref_num_total_refs: RefSlots<u32>,
    pub(crate) saved_global_motion_order_hints:
        RefSlots<splot_core::headers::frame::SavedGlobalMotionOrderHints>,
    pub(crate) saved_global_motion_params:
        RefSlots<splot_core::headers::frame::SavedGlobalMotionParams>,
    pub(crate) lr_frame_filter_class_counts: RefSlots<[u8; 3]>,
    pub(crate) lr_frame_filter_taps: Option<Arc<[SlotFrameFilterTaps]>>,
    pub(crate) allowed_frames: Option<u32>,
    pub(crate) ref_frame_cdfs: RefSlots<Option<FrameCdfHandle>>,
    pub(crate) ref_ccso_headers: RefSlots<Option<Arc<FrameHeaderCore>>>,
    pub(crate) ref_ccso_unit_grids: RefSlots<Option<CcsoGridHandle>>,
    pub(crate) ref_segment_ids: RefSlots<Option<SegmentIdMapHandle>>,
    pub(crate) ref_motion_fields: RefSlots<Option<MotionFieldHandle>>,
    /// The geometry of each stored frame, which [`InterReferenceState::put`]
    /// records.
    ref_frame_info: [Option<DecodedFrameInfo>; ReferenceSlot::MAX_SLOTS],
}

impl<T: ReconSample> InterReferenceState<T> {
    pub(crate) fn retire_handles(&mut self) {
        self.store.clear();
        self.metadata.retire_handles();
    }

    /// Builds a reference-free state over a minimal empty store.
    ///
    /// # Errors
    /// Returns [`splot_recon::ReconError`] when the minimal store capacity is
    /// rejected.
    pub(crate) fn empty() -> splot_recon::Result<Self> {
        Self::with_slot_count(1)
    }

    pub(crate) fn with_slot_count(num: usize) -> splot_recon::Result<Self> {
        Ok(Self {
            store: ReferenceFrameStore::with_capacity(num)?,
            metadata: ReferenceMetadata::default(),
        })
    }

    /// Stores `frame` in `slot` and records its geometry in the metadata.
    ///
    /// # Errors
    /// Returns [`splot_recon::ReconError`] when `slot` is outside the store.
    pub(crate) fn put(
        &mut self,
        slot: ReferenceSlot,
        frame: RefFrameSlot<T>,
    ) -> splot_recon::Result<()> {
        let info = frame.info();
        self.store.put(slot, frame)?;
        if let Some(stored) = self.metadata.ref_frame_info.get_mut(slot.index()) {
            *stored = Some(info);
        }
        Ok(())
    }

    /// The frame stored in `slot`.
    pub(crate) fn frame(&self, slot: ReferenceSlot) -> Option<&RefFrameSlot<T>> {
        self.store.get(slot).ok().flatten()
    }
}

impl ReferenceMetadata {
    /// The geometry of the frame stored in `slot`.
    pub(crate) fn frame_info(&self, slot: u32) -> Option<DecodedFrameInfo> {
        *self.ref_frame_info.get(slot as usize)?
    }

    fn retire_handles(&mut self) {
        self.ref_frame_info = Default::default();
        self.lr_frame_filter_taps = None;
        self.ref_frame_cdfs = RefSlots::default();
        self.ref_ccso_headers = RefSlots::default();
        self.ref_ccso_unit_grids = RefSlots::default();
        self.ref_segment_ids = RefSlots::default();
        self.ref_motion_fields = RefSlots::default();
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
    ) -> Result<RefSlots<Option<Arc<TemporalMotionField>>>> {
        let mut resolved = RefSlots::default();
        for (index, slot) in self.ref_motion_fields.iter().enumerate() {
            let selected = ref_frame_idx
                .iter()
                .any(|&selected| selected as usize == index);
            let field = match slot {
                Some(handle) if selected => {
                    Some(Arc::clone(handle.field().ok_or(
                        DecodeReferenceStateError::MissingMotionFieldPublication,
                    )?))
                }
                None if selected => {
                    return Err(DecodeReferenceStateError::MissingMotionFieldPublication.into());
                }
                None | Some(_) => None,
            };
            resolved
                .push(field)
                .ok_or(DecodeReferenceStateError::MissingMotionFieldPublication)?;
        }
        if ref_frame_idx
            .iter()
            .any(|&selected| selected as usize >= self.ref_motion_fields.len())
        {
            return Err(DecodeReferenceStateError::MissingMotionFieldPublication.into());
        }
        Ok(resolved)
    }
}

/// The settled state of the reference frames one frame's prediction reads.
///
/// A frame's entropy walk parses without reference samples; only reconstruction
/// reads them. A caller polls [`Self::is_ready`] before enqueuing work that
/// reads samples, and blocks on [`Self::wait`] when it is about to read them on
/// the driver thread itself.
pub(crate) struct PixelReferenceGate<'a, T: ReconSample> {
    slots: RefSlots<Option<&'a RefFrameSlot<T>>>,
}

/// The reference slots a parsed frame header names for pixel prediction.
/// Whether `core` is a `bru_inactive == 1` frame, whose § 7.2 wrapup copies
/// `FrameStore[ ref_frame_idx[ bru_ref ] ]` and records a zero motion field on
/// `bru_ref` instead of decoding tiles.
pub(crate) fn bru_inactive_frame(core: &FrameHeaderCore) -> bool {
    core.inter
        .as_ref()
        .is_some_and(|inter| inter.bru_inactive == Some(true))
}

/// The reference slot `ref_frame_idx[ bru_ref ]` of a `use_bru == 1` frame.
fn bru_ref_slot(core: &FrameHeaderCore) -> Option<u32> {
    let inter = core.inter.as_ref()?;
    let bru_ref = usize::try_from(inter.bru_ref?).ok()?;
    inter.ref_frame_idx.get(bru_ref).copied()
}

pub(crate) fn named_pixel_reference_slots(core: &FrameHeaderCore) -> impl Iterator<Item = u32> {
    core.inter
        .as_ref()
        .map_or(&[][..], |inter| &inter.ref_frame_idx[..])
        .iter()
        .copied()
        .chain(core.bridge_frame_ref_idx)
}

impl<T: ReconSample> PixelReferenceGate<'_, T> {
    /// Whether every named reference frame has settled.
    pub(crate) fn is_ready(&self) -> bool {
        self.slots.iter().flatten().all(|slot| slot.is_settled())
    }

    /// Blocks the calling driver thread until every named reference frame has
    /// settled.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when a referenced frame's filter phase
    /// failed; the driver replaces it with that frame's own recorded failure.
    pub(crate) fn wait(&self) -> Result<()> {
        let mut first_error = None;
        for slot in self.slots.iter().flatten() {
            if let Err(error) = slot.wait_settled()
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

fn distinct_reference_slots(
    named: impl IntoIterator<Item = u32>,
) -> impl Iterator<Item = ReferenceSlot> {
    let mut seen = 0u16;
    named.into_iter().filter_map(move |slot| {
        let bit = 1u16.checked_shl(slot)?;
        if seen & bit != 0 {
            return None;
        }
        seen |= bit;
        ReferenceSlot::new(slot as usize).ok()
    })
}

impl ReferenceMetadata {
    /// Shares each distinct motion-field product named by a reference map.
    pub(crate) fn motion_dependencies(
        &self,
        ref_frame_idx: &[u32],
    ) -> RefSlots<Option<MotionFieldHandle>> {
        let mut dependencies = RefSlots::default();
        dependencies.extend_within(
            distinct_reference_slots(ref_frame_idx.iter().copied())
                .filter_map(|slot| self.ref_motion_fields.get(slot.index())?.as_ref())
                .cloned()
                .map(Some),
        );
        dependencies
    }
}

impl<T: ReconSample> InterReferenceState<T> {
    /// Gates on the stored frames the named reference slots resolve to.
    ///
    /// Slots without a stored frame are left out: reading one is already a
    /// fail-closed diagnostic at the point of use.
    pub(crate) fn pixel_reference_gate(
        &self,
        named: impl IntoIterator<Item = u32>,
    ) -> PixelReferenceGate<'_, T> {
        let mut slots = RefSlots::default();
        slots.extend_within(
            distinct_reference_slots(named)
                .filter_map(|slot| self.frame(slot))
                .map(Some),
        );
        PixelReferenceGate { slots }
    }

    fn slot(&self, slot: u32) -> Option<&RefFrameSlot<T>> {
        self.frame(ReferenceSlot::new(slot as usize).ok()?)
    }
}

impl ReferenceMetadata {
    fn cdfs_for_slot(&self, slot: u32) -> Result<Arc<FrameCdfSubset>> {
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        self.ref_frame_cdfs
            .get(slot)
            .and_then(Option::as_ref)
            .and_then(FrameCdfHandle::product)
            .cloned()
            .ok_or(DecodeReferenceStateError::MissingCdfContext { slot }.into())
    }

    fn ccso_params_for_slot(&self, slot: u32) -> Result<&splot_core::headers::frame::CcsoParams> {
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        self.ref_ccso_headers
            .get(slot)
            .and_then(Option::as_deref)
            .and_then(|header| header.ccso_params.as_ref())
            .ok_or(DecodeReferenceStateError::MissingCcsoParams { slot }.into())
    }
}

fn parse_inter_or_sef_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    if envelope.header.obu_type.is_sef() || envelope.header.obu_type.is_tip_frame() {
        parse_sef_or_tip_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
            frame_index,
        )
    } else {
        parse_inter_frame_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            mfh_record,
            frame_index,
        )
    }
}

pub(crate) fn parse_inter_frame_activation(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    first_picture_in_tu: bool,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    validate_obu_layer_ids(envelope, sequence, frame_index)?;
    parse_inter_or_sef_core(
        envelope,
        sequence,
        reference,
        first_picture_in_tu,
        None,
        frame_index,
    )
}

pub(crate) fn parse_validated_inter_frame_core<'a>(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    first_picture_in_tu: bool,
    resolve_mfh: impl FnOnce(MfhId) -> Result<&'a MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    let mut core = parse_inter_frame_activation(
        envelope,
        sequence,
        reference,
        first_picture_in_tu,
        frame_index,
    )?;
    if !core.cur_mfh_id.is_zero() {
        let record = resolve_mfh(core.cur_mfh_id)?;
        core = parse_inter_or_sef_core(
            envelope,
            sequence,
            reference,
            first_picture_in_tu,
            Some(record),
            frame_index,
        )?;
    }
    if envelope.header.obu_type.is_sef() {
        validate_sef_frame_core(&core, reference, envelope.offset, frame_index)?;
        validate_sef_layer_dependency(
            envelope.header,
            &core,
            sequence,
            reference,
            envelope.offset,
            frame_index,
        )?;
    } else {
        validate_inter_layer_dependencies(
            envelope.header,
            &core,
            sequence,
            reference,
            envelope.offset,
            frame_index,
        )?;
        if envelope.header.obu_type.is_tip_frame() {
            resolve_tip_output_core(&mut core, sequence, reference, envelope.offset, frame_index)?;
        } else {
            validate_and_resolve_inter_frame_core(
                &mut core,
                sequence,
                reference,
                envelope.offset,
                frame_index,
            )?;
        }
    }
    Ok(core)
}

fn validate_and_resolve_inter_frame_core(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    validate_inter_frame_parse(core, offset, frame_index)?;
    if core.status == FrameHeaderParseStatus::IntraHeaderComplete {
        return Ok(());
    }
    validate_ras_reference_ids(core, reference, offset, frame_index)?;
    resolve_ccso_reference_reuse(core, reference, offset, frame_index)?;
    validate_inter_frame_core(core, sequence)?;
    if bru_inactive_frame(core) {
        validate_bru_inactive_reference(core, reference, offset, frame_index)?;
    }
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
    reference: &ReferenceMetadata,
    first_picture_in_tu: bool,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        obu_mlayer_id: envelope.header.embedded_layer_id,
        obu_tlayer_id: envelope.header.temporal_layer_id,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: reference.header_view(),
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

pub(super) fn malformed_frame_header(
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
    reference: &ReferenceMetadata,
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

/// Whether frame `core` over `reference` has the past/future reference pair
/// TIP needs (§ 7.12.2) with an inter frame on at least one side (§ 7.9):
/// the rule the decoder checks.
pub(crate) fn tip_pair_available(core: &FrameHeaderCore, reference: &ReferenceMetadata) -> bool {
    required_tip_reference_pair(core, reference, ByteOffset::new(0), None).is_ok()
}

fn required_tip_reference_pair(
    core: &FrameHeaderCore,
    reference: &ReferenceMetadata,
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
    let pair = find_mv_stack::tip_reference_pair_from_hints(current_order_hint, &hints)
        .ok_or_else(|| DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                frame_index,
                SPEC_TIP_TEMPORAL_SCALE,
                "TIP has no usable past/future reference pair".to_owned(),
            ),
        })?;
    let intra = |ref_frame| {
        block_reference_slot(&inter.ref_frame_idx, ref_frame)
            .map(|slot| reference.ref_is_intra_frame.get(slot as usize) == Some(&true))
    };
    if intra(pair.past_ref)? && intra(pair.future_ref)? {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                offset,
                frame_index,
                SPEC_TIP_TEMPORAL_SCALE,
                "TIP has no inter frame as ClosestPast or ClosestFuture".to_owned(),
            ),
        });
    }
    Ok(pair)
}

/// Validates the parsed TIP output frame `core` and infers its quantizer.
pub(crate) fn resolve_tip_output_core(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    validate_tip_output_frame_parse(core, offset, frame_index)?;
    infer_tip_output_quantization(core, sequence, reference, offset, frame_index)?;
    validate_tip_output_frame_core(core)
}

fn infer_tip_output_quantization(
    core: &mut FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
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
        let Some((&base_q_idx, chroma_ac_deltas)) = reference
            .ref_base_q_idx
            .get(slot)
            .zip(reference.ref_chroma_ac_deltas.get(slot))
        else {
            return Err(DecodeReferenceStateError::MissingQuantizerMetadata { slot }.into());
        };
        let [delta_q_u_ac, delta_q_v_ac] = *chroma_ac_deltas;
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

pub(crate) fn resolve_ccso_reference_reuse(
    core: &mut FrameHeaderCore,
    reference: &ReferenceMetadata,
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
        plane.ccso_offset_idx = ref_plane.ccso_offset_idx;
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
    reference: &ReferenceMetadata,
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
        obu_mlayer_id: envelope.header.embedded_layer_id,
        obu_tlayer_id: envelope.header.temporal_layer_id,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: reference.header_view(),
    };
    let core = parse_frame_header_core(&mut reader, &input).map_err(|error| {
        malformed_frame_header(envelope.offset, frame_index, SPEC_HEADER, error.to_string())
    })?;
    if bru_inactive_frame(&core) && core.status == FrameHeaderParseStatus::InterHeaderComplete {
        let remaining_bits = reader.remaining_bits();
        splot_core::obu::parse_trailing_bits(&mut reader, remaining_bits).map_err(|error| {
            malformed_frame_header(
                envelope.offset,
                frame_index,
                SPEC_TILE_GROUP,
                format!("bru_inactive tile group trailing_bits() are malformed: {error}"),
            )
        })?;
    }
    Ok(core)
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
        let valid_intra_only = matches!(
            core.obu_type,
            ObuType::LeadingTileGroup | ObuType::RegularTileGroup
        ) && core.frame_type == Some(FrameType::IntraOnly)
            && core.frame_is_intra == Some(true);
        if !valid_intra_only {
            return Err(DecodeHeaderStateError::InvalidIntraOnlyTileGroupState.into());
        }
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

fn validate_inter_frame_core(core: &FrameHeaderCore, sequence: &SequenceHeader) -> Result<()> {
    if core.obu_type == ObuType::BridgeFrame {
        return validate_bridge_frame_core(core);
    }
    if bru_inactive_frame(core) {
        if core.status != FrameHeaderParseStatus::InterHeaderComplete
            || core.order_hint.is_none()
            || core.quantization_params.is_none()
            || core.bridge_film_grain.is_none()
            || core
                .frame_size
                .is_none_or(|size| size.width == 0 || size.height == 0)
            || bru_ref_slot(core).is_none()
        {
            return Err(DecodeHeaderStateError::IncompleteInterFrame.into());
        }
        return Ok(());
    }
    if core.status != FrameHeaderParseStatus::InterHeaderComplete {
        return Err(DecodeHeaderStateError::IncompleteInterFrame.into());
    }
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    if inter.mv_precision.is_none() {
        return Err(DecodeHeaderStateError::MissingInterMotionVectorPrecision.into());
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
        || inter.opfl_refine_type.is_none()
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

fn validate_bru_inactive_reference(
    core: &FrameHeaderCore,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    let slot = bru_ref_slot(core).ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
    let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
    if core.immediate_output_frame != Some(true) {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            SPEC_FRAME_HEADER_INFO_SEMANTICS,
            "use_bru == 1 frame must set immediate_output_frame == 1".to_owned(),
        ));
    }
    let refresh_mask = 1u32.checked_shl(slot).unwrap_or(0);
    if core
        .refresh_frame_flags
        .is_none_or(|flags| flags & refresh_mask == 0)
    {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            SPEC_FRAME_HEADER_INFO_SEMANTICS,
            format!("use_bru == 1 frame must refresh its bru_ref reference slot {slot}"),
        ));
    }
    if reference.ref_valid.get(slot_index) == Some(&true)
        && let (Some(frame_size), Some(&ref_w), Some(&ref_h)) = (
            core.frame_size,
            reference.ref_frame_width.get(slot_index),
            reference.ref_frame_height.get(slot_index),
        )
        && (ref_w != frame_size.width || ref_h != frame_size.height)
    {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            SPEC_FRAME_HEADER_INFO_SEMANTICS,
            format!(
                "bru_inactive reference slot {slot} size {ref_w}x{ref_h} does not match frame size {}x{}",
                frame_size.width, frame_size.height
            ),
        ));
    }
    Ok(())
}

fn validate_ras_reference_ids(
    core: &FrameHeaderCore,
    reference: &ReferenceMetadata,
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

fn validate_bridge_frame_core(core: &FrameHeaderCore) -> Result<()> {
    if core.status != FrameHeaderParseStatus::InterHeaderComplete
        || core.frame_is_intra.is_none()
        || core.show_existing_frame != Some(false)
    {
        return Err(DecodeHeaderStateError::IncompleteInterFrame.into());
    }
    if core.order_hint.is_none() {
        return Err(DecodeHeaderStateError::MissingDisplayOrderHint.into());
    }
    let frame_size = core
        .frame_size
        .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
    if frame_size.width == 0 || frame_size.height == 0 {
        return Err(DecodeHeaderStateError::ZeroFrameSize.into());
    }
    if core.tile_info.is_none()
        || core.quantization_params.is_none()
        || core.bridge_film_grain.is_none()
    {
        return Err(DecodeHeaderStateError::IncompleteInterFrameTools.into());
    }
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let bridge_frame_ref_idx = core
        .bridge_frame_ref_idx
        .ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
    let reference_map_complete = if core.frame_is_intra == Some(true) {
        inter.num_total_refs == Some(0)
    } else {
        inter.num_total_refs == Some(1)
            && inter.ref_frame_idx.first() == Some(&bridge_frame_ref_idx)
    };
    if !reference_map_complete {
        return Err(DecodeHeaderStateError::InvalidInterReferenceMap.into());
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
pub(crate) mod cross_frame;
mod find_mv_stack;
mod frame_products;
mod frame_walk;
mod layer_dependencies;
pub(crate) mod mc;
mod mode_symbols;
mod motion_field;
pub(crate) mod mv_scaling;
pub(crate) mod read_mv;
pub(crate) mod reference;
mod single_ref;

pub use block::encoder_leaf::{Bawp, CompoundType};
pub(crate) use block::encoder_leaf::{
    EncoderCompoundMotion, EncoderInterCheckpoint, EncoderInterResidual, EncoderInterTile,
    EncoderMotion, EncoderSingleMotion, EncoderWarpMotion, EncoderWarpSource,
    JMVD_AMVD_SCALE_MODES, JMVD_SCALE_MODES, MAX_CWP_NUM, MvdCoding, chroma_inter_tx_type,
    encoder_motion_field, encoder_temporal_context,
};
pub(crate) use block::inter_residual_units;
pub(crate) use block::{
    InterBlockDecodeOutput, InterBlockFacts, InterDecodeScratch, InterFilterInputs,
    InterFrameParse, InterFrameParser, InterReconScratchPool, PendingFilterAttach,
    RowReferenceBounds, ScheduledTileRecon, ScheduledTileWorkspace, TipOutputStages, TipSpares,
    bounds_satisfied, deblock_tip_output, decode_inter_blocks, invalid_inter_tile_scheduling_state,
};
pub use compound::CompoundMode;
use cross_frame::{ResolvedCdfLoad, resolve_cdf_load};
pub(crate) use find_mv_stack::{
    FixedStack, MotionFieldLayout, TemporalMotionBlock, TemporalMotionField, TemporalMvContext,
};
pub(crate) use find_mv_stack::{GridEdge, TIP_REF_FRAME};
#[cfg(debug_assertions)]
pub(crate) use find_mv_stack::{RefMvBank, WarpParamBank};
pub(crate) use frame_products::{
    CcsoGridHandle, FrameCdfHandle, FrameProductWriters, FrameProducts, SegmentIdMapHandle,
};
pub(crate) use frame_walk::{
    DeferredInterWalk, FrameDecodeGeometry, InterFrameStart, InterWalkEarly, TileParseJobs,
    splittable_inter_frame,
};
use layer_dependencies::{
    validate_inter_layer_dependencies, validate_obu_layer_ids, validate_sef_layer_dependency,
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
