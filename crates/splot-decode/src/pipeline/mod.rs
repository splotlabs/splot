// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode pipeline orchestration for the supported decode runtime.
//!
//! The driver walks frames strictly in decode order and runs every sequential
//! state machine (output effects, film-grain slots, reference buffer, output
//! scheduler) at the same program point relative to each frame's walk. Only the
//! § 7.2 filter phase moves: it is handed to the worker pool through the frame
//! admission scheduler, and the driver is the only thread that blocks on a
//! frame's samples. The resolved frame delay bounds capacity, not the algorithm.

use core::num::NonZeroUsize;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{FrameHeaderCore, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader};
use splot_core::headers::tile_group::{
    FrameHeaderCopyOutcome, RecordedFrameHeaderBits, parse_frame_header_copy,
};
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::types::ObuType;
use splot_recon::BitDepth;
#[cfg(test)]
use splot_recon::DecodedFrame;

use crate::DecodeIvfFrameContext as IvfFrameContext;
use crate::bitstream::byte_stream::FlatParsedBitstream;
#[cfg(test)]
use crate::bitstream::byte_stream::parse_bounded_bitstream;
use crate::bitstream::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    GeneralIntraBlockModeError, GeneralIntraResidualError, TileGroupPositionFacts,
};
use crate::error::{DecodeError, Result};
use crate::prediction::inter;
use crate::reference::buffer as reference_buffer;
use crate::support::pipeline_limits::checked_add;
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

mod frame_lifecycle;
pub(crate) mod frame_pipeline;
pub(crate) mod frame_progress;
pub(crate) mod inflight;
pub(crate) mod output_effects;
mod output_schedule;
mod runtime_support;
mod stream_schedule;

use frame_lifecycle::*;
pub(crate) use frame_lifecycle::{
    ActiveFilmGrain, PipelineDecodedFrame, PipelineFrame, PipelineFrameRate, deblock_quant_deltas,
    derive_visible_luma_rect, effective_allow_screen_content_tools, frame_ref_update_from_core,
    is_key_or_switch, parse_frame_core, parse_frame_core_with_reference, parse_sequence,
};
use output_effects::{FrameOutputEffects, OutputEffectState};
use output_schedule::*;
use runtime_support::decode_tile_boundary_error;
pub(crate) use runtime_support::{
    ensure_runtime_limits, malformed_tile_payload, unsupported, unsupported_at,
    unsupported_feature_at, unsupported_with_spec,
};
pub(crate) use stream_schedule::following_inter_envelope;
#[cfg(test)]
pub(crate) use stream_schedule::require_minimal_obu_order;
use stream_schedule::*;

pub(crate) const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
pub(crate) const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";

#[cfg(test)]
pub(crate) fn decode_frame_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<PipelineFrame> {
    let mut frames = decode_frames_from_plan(bytes, options, plan)?;
    if frames.is_empty() {
        return Err(unsupported(
            "missing_decoded_frame",
            None,
            "decode runtime requires at least one decoded frame",
        ));
    }
    Ok(frames.swap_remove(0))
}
#[cfg(test)]
pub(crate) fn decode_frames_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<PipelineFrame>> {
    let mut parsed = parse_bounded_bitstream(bytes, options.limits())?;
    parsed.discard_runtime_noops();
    decode_frames_from_plan_impl(
        &parsed,
        bytes,
        options,
        plan,
        NonZeroUsize::MIN,
        |_| Ok(()),
        true,
        |_| Ok(()),
    )
}

pub(crate) fn emit_frames_from_prepared(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<()> {
    decode_frames_from_plan_impl(
        parsed,
        bytes,
        options,
        plan,
        frame_delay,
        preflight,
        false,
        emit,
    )
    .map(drop)
}

/// Retires the settled frames nothing owns any more, keeping their sample
/// buffers for the frames that take their reference slots and subtracting their
/// bytes from the live-frame accounting.
///
/// A frame with any remaining owner or shared sample handle is skipped: its
/// planes stay alive whatever the driver releases, and subtracting the bytes
/// would let the live-frame peak run above
/// [`crate::DecodeLimitName::MaxReferenceStoreBytes`]. The driver rescans every
/// frame on each call, so the skipped frame is reclaimed on the first pass after
/// its last owner releases it.
fn reclaim_unowned_frames(
    frames: &mut [Option<PipelineFrame>],
    reference: &reference_buffer::RuntimeReferenceBuffer,
    scheduler: &OutputScheduler,
    emission: &output_schedule::EmissionQueue,
    ring: &mut inflight::InflightRing,
    retained_frame_bytes: &mut u64,
) -> Result<()> {
    for frame_index in 0..frames.len() {
        let Some(frame) = frames[frame_index].as_ref() else {
            continue;
        };
        if reference.retains(frame_index)
            || scheduler.retains(frame_index)
            || emission.holds(frame_index)
            || ring.holds(frame_index)
            || !frame.frame.is_settled()
            || frame.frame.handle_count() > 1
            || !frame.frame.is_sole_handle()
        {
            continue;
        }
        let Some(frame) = frames.get_mut(frame_index).and_then(Option::take) else {
            continue;
        };
        let frame_bytes = retained_decoded_frame_bytes(&frame)?;
        *retained_frame_bytes = retained_frame_bytes
            .checked_sub(frame_bytes)
            .ok_or_else(|| {
                unsupported(
                    "retained_frame_byte_accounting_underflow",
                    None,
                    "decode pipeline live-frame byte accounting underflowed",
                )
            })?;
        ring.keep_frame_planes(frame.frame);
    }
    Ok(())
}

fn parse_key_core_with_effects(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    effects: &OutputEffectState,
) -> Result<FrameHeaderCore> {
    let activation = parse_frame_core(envelope, sequence)?;
    if activation.cur_mfh_id.is_zero() {
        return Ok(activation);
    }
    let record = effects.resolve_mfh_record(envelope, sequence, activation.cur_mfh_id)?;
    parse_frame_core_with_mfh(envelope, sequence, Some(record))
}

fn parse_olk_core_with_effects(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &inter::InterReferenceState<impl splot_recon::ReconSample>,
    effects: &OutputEffectState,
    first_picture_in_tu: bool,
) -> Result<FrameHeaderCore> {
    let activation = parse_frame_core_with_reference(
        envelope,
        sequence,
        None,
        first_picture_in_tu,
        &reference.header_view(),
    )?;
    if activation.cur_mfh_id.is_zero() {
        return Ok(activation);
    }
    let record = effects.resolve_mfh_record(envelope, sequence, activation.cur_mfh_id)?;
    parse_frame_core_with_reference(
        envelope,
        sequence,
        Some(record),
        first_picture_in_tu,
        &reference.header_view(),
    )
}

#[derive(Default)]
struct InBandLongTermPrelude {
    frames: Vec<InBandLongTermFrame>,
}

struct InBandLongTermFrame {
    obu_type: ObuType,
    long_term_id: u32,
    hidden: bool,
    frame_index: usize,
}

impl InBandLongTermPrelude {
    fn begin_frame(&mut self, first_picture_in_tu: bool) {
        if first_picture_in_tu {
            self.frames.clear();
        }
    }

    fn validate_required(
        &self,
        core: &FrameHeaderCore,
        reference: &reference_buffer::RuntimeReferenceBuffer,
        offset: ByteOffset,
    ) -> Result<()> {
        self.validate_required_with(&core.ref_long_term_ids, offset, |id, frame_index| {
            reference.retains_hidden_long_term_reference(id, frame_index)
        })
    }

    fn validate_required_with(
        &self,
        required_ids: &[u32],
        offset: ByteOffset,
        mut is_retained: impl FnMut(u32, usize) -> bool,
    ) -> Result<()> {
        if required_ids.is_empty() {
            return Ok(());
        }
        for &id in required_ids {
            if !self.frames.iter().any(|frame| frame.long_term_id == id) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_missing",
                    offset,
                    "each ref_long_term_id must name a preceding in-band key frame in the random-access temporal unit",
                    "7.3.9.1",
                ));
            }
            if self
                .frames
                .iter()
                .any(|frame| frame.long_term_id == id && !frame.hidden)
            {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_visible",
                    offset,
                    "in-band long-term reference key frames must disable immediate and implicit output",
                    "7.3.9.1",
                ));
            }
            if !self.frames.iter().any(|frame| {
                frame.long_term_id == id && frame.hidden && is_retained(id, frame.frame_index)
            }) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_slot_unavailable",
                    offset,
                    "an in-band long-term reference must remain in the same valid reference slot used by sequential decoding",
                    "7.3.9.1",
                ));
            }
        }
        let mut saw_olk = false;
        for frame in self
            .frames
            .iter()
            .filter(|frame| required_ids.contains(&frame.long_term_id))
        {
            if frame.obu_type == ObuType::OpenLoopKey {
                saw_olk = true;
            } else if frame.obu_type == ObuType::ClosedLoopKey && saw_olk {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_order",
                    offset,
                    "in-band long-term CLK references must precede all in-band long-term OLK references",
                    "7.3.9.1",
                ));
            }
        }
        Ok(())
    }

    fn note_frame(&mut self, core: &FrameHeaderCore, frame_index: usize) {
        if !matches!(core.obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey) {
            return;
        }
        let Some(long_term_id) = core.long_term_id.and_then(|id| u32::try_from(id).ok()) else {
            return;
        };
        self.frames.push(InBandLongTermFrame {
            obu_type: core.obu_type,
            long_term_id,
            hidden: core.immediate_output_frame == Some(false)
                && core.implicit_output_frame == Some(false),
            frame_index,
        });
    }
}

fn parse_inter_core_with_effects(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &inter::InterReferenceState<impl splot_recon::ReconSample>,
    effects: &OutputEffectState,
    first_picture_in_tu: bool,
    frame_index: Option<usize>,
) -> Result<FrameHeaderCore> {
    let activation = inter::parse_inter_frame_activation(
        envelope,
        sequence,
        reference,
        first_picture_in_tu,
        frame_index,
    )?;
    let record = if activation.cur_mfh_id.is_zero() {
        None
    } else {
        Some(effects.resolve_mfh_record(envelope, sequence, activation.cur_mfh_id)?)
    };
    inter::parse_validated_inter_frame_core_with_mfh(
        envelope,
        sequence,
        reference,
        first_picture_in_tu,
        record,
        frame_index,
    )
}
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_key_frame(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    frame_rate: PipelineFrameRate,
    display_grain: Option<ActiveFilmGrain>,
) -> Result<PipelineFrame> {
    let core = parse_frame_core(frame_envelope, sequence)?;
    let admission = splot_parallel::AdmissionScheduler::new();
    let decoded = splot_parallel::ready_task_scope(|scope| {
        let buffers = crate::support::decode_buffers::DecodeBuffers::new();
        let mut scratch_eight = inter::InterDecodeScratch::default();
        let mut scratch_ten = inter::InterDecodeScratch::default();
        scratch_eight.set_decode_buffers(&buffers);
        scratch_ten.set_decode_buffers(&buffers);
        let mut ring = inflight::InflightRing::new(NonZeroUsize::MIN, buffers);
        let mut lane = frame_pipeline::ReconAdmissionLane::new(ring.capacity());
        let decoded = decode_key_frame_with_effects(
            &mut scratch_eight,
            &mut scratch_ten,
            scope,
            &admission,
            &mut lane,
            &mut ring,
            0,
            bytes,
            options,
            plan,
            candidate,
            frame_envelope,
            core,
            sequence,
            frame_rate,
            display_grain,
            None,
            FrameOutputEffects::empty(),
        );
        ring.harvest_all(&mut scratch_eight, &mut scratch_ten);
        match ring.take_failure() {
            Some(failure) => Err(failure),
            None => decoded,
        }
    })?;
    match decoded {
        Err(error) => Err(error),
        Ok(frame) => {
            admission.finish()?;
            Ok(frame)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_key_frame_with_effects<'job, 'scope>(
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope splot_parallel::AdmissionScheduler<
        'job,
        crate::pipeline::frame_pipeline::FrameTask,
    >,
    lane: &mut frame_pipeline::ReconAdmissionLane,
    ring: &mut inflight::InflightRing,
    frame_index: usize,
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    frame_rate: PipelineFrameRate,
    display_grain: Option<ActiveFilmGrain>,
    user_qm: Option<crate::bitstream::tile_payload::FrameUserQmLevels>,
    output_effects: FrameOutputEffects,
) -> Result<PipelineFrame>
where
    'job: 'scope,
{
    ring.reserve(scratch_eight, scratch_ten);
    let _user_qm_scope = crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
    let (frame, frame_cdfs, ccso_params, ccso_grid, segment_ids, motion_field) =
        match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let walk = frame_engine::walk_frame::<u8>(
                    scratch_eight,
                    plan,
                    candidate,
                    bytes,
                    frame_envelope,
                    core,
                    sequence,
                    options,
                    &frame_engine::FrameSetup::Intra,
                    BitDepth::Eight,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Eight,
                    scope,
                    scheduler,
                    lane,
                    ring,
                    frame_index,
                )?;
                (
                    frame,
                    walk.frame_cdfs,
                    ccso_params,
                    walk.ccso_grid,
                    walk.segment_ids,
                    walk.motion_field,
                )
            }
            BitDepthIdc::Ten => {
                let walk = frame_engine::walk_frame::<u16>(
                    scratch_ten,
                    plan,
                    candidate,
                    bytes,
                    frame_envelope,
                    core,
                    sequence,
                    options,
                    &frame_engine::FrameSetup::Intra,
                    BitDepth::Ten,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Ten,
                    scope,
                    scheduler,
                    lane,
                    ring,
                    frame_index,
                )?;
                (
                    frame,
                    walk.frame_cdfs,
                    ccso_params,
                    walk.ccso_grid,
                    walk.segment_ids,
                    walk.motion_field,
                )
            }
        };
    let frame_rate = output_effects.frame_rate(frame_rate);
    Ok(PipelineFrame {
        frame,
        display_grain,
        output_effects,
        frame_cdfs: inter::FrameCdfHandle::settled(frame_cdfs),
        motion_field: inter::MotionFieldHandle::settled(motion_field),
        ccso_params: ccso_params.map(Arc::new),
        ccso_grid: inter::CcsoGridHandle::settled(ccso_grid.map(Arc::new)),
        segment_ids: inter::SegmentIdMapHandle::settled(segment_ids),
        frame_rate_numerator: frame_rate.numerator,
        frame_rate_denominator: frame_rate.denominator,
    })
}

/// Runs the frame loop through the admission scheduler. The resolved frame
/// delay bounds in-flight storage at every worker count.
#[allow(clippy::too_many_arguments)]
fn decode_frames_from_plan_impl<'job>(
    parsed: &'job FlatParsedBitstream<'job>,
    bytes: &'job [u8],
    options: &'job DecodeOptions,
    plan: &'job DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<Vec<PipelineFrame>> {
    let pipeline_capacity = frame_delay
        .min(NonZeroUsize::new(splot_parallel::current_pool_width()).unwrap_or(NonZeroUsize::MIN));
    let admission: splot_parallel::AdmissionScheduler<
        'job,
        crate::pipeline::frame_pipeline::FrameTask,
    > = splot_parallel::AdmissionScheduler::new();
    let decoded = splot_parallel::ready_task_scope(|scope| {
        drive_frames(
            parsed,
            bytes,
            options,
            plan,
            pipeline_capacity,
            preflight,
            retain_decoded_frames,
            emit,
            scope,
            &admission,
        )
    })?;
    match decoded {
        Err(error) => Err(error),
        Ok(frames) => {
            admission.finish()?;
            Ok(frames)
        }
    }
}

/// Owns the decode scratch and the in-flight ring for one decode, and resolves
/// the run's outcome against the filter phases the ring collected.
///
/// A filter-phase failure outranks the frame loop's own error: serial decode
/// would have run that frame's filters before reaching the later error, so the
/// lowest-indexed collected failure is the one the caller sees.
#[allow(clippy::too_many_arguments)]
fn drive_frames<'job, 'scope>(
    parsed: &'job FlatParsedBitstream<'job>,
    bytes: &'job [u8],
    options: &'job DecodeOptions,
    plan: &'job DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    admission: &'scope splot_parallel::AdmissionScheduler<
        'job,
        crate::pipeline::frame_pipeline::FrameTask,
    >,
) -> Result<Vec<PipelineFrame>>
where
    'job: 'scope,
{
    let buffers = crate::support::decode_buffers::DecodeBuffers::new();
    let mut decode_scratch_eight = inter::InterDecodeScratch::default();
    let mut decode_scratch_ten = inter::InterDecodeScratch::default();
    decode_scratch_eight.set_decode_buffers(&buffers);
    decode_scratch_ten.set_decode_buffers(&buffers);
    let mut ring = inflight::InflightRing::new(frame_delay, buffers);
    let decoded = decode_frames_in_order(
        parsed,
        bytes,
        options,
        plan,
        preflight,
        retain_decoded_frames,
        emit,
        scope,
        admission,
        &mut ring,
        &mut decode_scratch_eight,
        &mut decode_scratch_ten,
    );
    ring.harvest_all(&mut decode_scratch_eight, &mut decode_scratch_ten);
    match ring.take_failure() {
        Some(failure) => Err(failure),
        None => decoded,
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_frames_in_order<'job, 'scope>(
    parsed: &'job FlatParsedBitstream<'job>,
    bytes: &'job [u8],
    options: &'job DecodeOptions,
    plan: &'job DecodeStreamPlan,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    mut emit: impl FnMut(&PipelineFrame) -> Result<()>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    admission: &'scope splot_parallel::AdmissionScheduler<
        'job,
        crate::pipeline::frame_pipeline::FrameTask,
    >,
    ring: &mut inflight::InflightRing,
    decode_scratch_eight: &mut inter::InterDecodeScratch<u8>,
    decode_scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<Vec<PipelineFrame>>
where
    'job: 'scope,
{
    let stream = require_runtime_stream(parsed)?;
    if matches!(stream, RuntimeStream::Ivf { ivf, .. } if ivf.frames.is_empty())
        && plan.obu_count() == 0
        && plan.frame_candidate_count() == 0
    {
        return Ok(Vec::new());
    }
    ensure_multiframe_plan_shape(plan)?;
    preflight(stream.ivf_header())?;
    let frame_rate = stream.frame_rate();

    let leading_obus = stream.leading_obus()?;
    let ([_, sequence_envelope, key_envelope], leading_frame_unit_len) =
        require_leading_frame_unit(leading_obus)?;

    let mut sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;
    let mut film_grain_slots = FilmGrainSlots::new();
    let leading_prefix = leading_prefix_obus(leading_obus)?;
    film_grain_slots.update_from_obus(leading_prefix)?;
    let mut output_effect_state = OutputEffectState::new();
    let mut recon_lane = frame_pipeline::ReconAdmissionLane::new(ring.capacity());
    output_effect_state.observe_prefix(leading_prefix, &sequence)?;

    let sequence_inter = sequence.inter.as_ref().ok_or_else(|| {
        unsupported(
            "missing_sequence_inter_config",
            None,
            "multi-frame decode requires the sequence inter config (NumRefFrames)",
        )
    })?;
    let num_ref_frames = usize::from(sequence_inter.num_ref_frames);
    let mut reference = reference_buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;
    let mut frames = Vec::new();
    let mut scheduler = OutputScheduler::new(num_ref_frames);
    let mut emission_queue = output_schedule::EmissionQueue::default();
    let mut in_band_long_term_prelude = InBandLongTermPrelude::default();
    in_band_long_term_prelude.begin_frame(true);
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "decode runtime requires one selected key frame candidate",
        )
    })?;
    let key_frame_index = key_candidate.ivf_frame().map(IvfFrameContext::frame_index);
    let key_core = match key_envelope.header.obu_type {
        ObuType::ClosedLoopKey => {
            parse_key_core_with_effects(key_envelope, &sequence, &output_effect_state)?
        }
        ObuType::OpenLoopKey => match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let (store, meta) = reference.build_store_eight(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                parse_olk_core_with_effects(
                    key_envelope,
                    &sequence,
                    &state,
                    &output_effect_state,
                    true,
                )?
            }
            BitDepthIdc::Ten => {
                let (store, meta) = reference.build_store_ten(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                parse_olk_core_with_effects(
                    key_envelope,
                    &sequence,
                    &state,
                    &output_effect_state,
                    true,
                )?
            }
        },
        ObuType::RasFrame => match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let (store, meta) = reference.build_store_eight(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                let activation = inter::parse_inter_frame_activation(
                    key_envelope,
                    &sequence,
                    &state,
                    true,
                    key_frame_index,
                )?;
                in_band_long_term_prelude.validate_required(
                    &activation,
                    &reference,
                    key_envelope.offset,
                )?;
                parse_inter_core_with_effects(
                    key_envelope,
                    &sequence,
                    &state,
                    &output_effect_state,
                    true,
                    key_frame_index,
                )?
            }
            BitDepthIdc::Ten => {
                let (store, meta) = reference.build_store_ten(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                let activation = inter::parse_inter_frame_activation(
                    key_envelope,
                    &sequence,
                    &state,
                    true,
                    key_frame_index,
                )?;
                in_band_long_term_prelude.validate_required(
                    &activation,
                    &reference,
                    key_envelope.offset,
                )?;
                parse_inter_core_with_effects(
                    key_envelope,
                    &sequence,
                    &state,
                    &output_effect_state,
                    true,
                    key_frame_index,
                )?
            }
        },
        _ => {
            return Err(unsupported_at(
                "missing_random_access_frame",
                key_envelope.offset,
                "decode runtime requires a closed-loop key, open-loop key, or RAS random-access frame",
            ));
        }
    };
    if key_envelope.header.obu_type != ObuType::RasFrame {
        ensure_intra_header_complete(&key_core, key_envelope.offset)?;
    }
    in_band_long_term_prelude.validate_required(&key_core, &reference, key_envelope.offset)?;
    let key_user_qm = output_effect_state.prepare_frame(
        key_envelope,
        &key_core,
        &sequence,
        true,
        key_frame_index,
    )?;
    let key_display_grain = film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;
    output_effect_state.observe_suffix(frame_suffix_obus(stream, key_candidate)?)?;
    let key_output_effects = output_effect_state.finish_frame();
    let mut retained_frame_bytes = 0;
    let mut next_unvalidated_following_ivf_record = 1;
    let mut next_unvalidated_following_annexb_obu = leading_frame_unit_len;
    ensure_retained_frame_byte_limits_for_core(
        options.limits(),
        retained_frame_bytes,
        &key_core,
        &sequence,
        key_envelope.offset,
    )?;
    let key_frame = if key_envelope.header.obu_type == ObuType::RasFrame {
        match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let (store, meta) = reference.build_store_eight(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                ring.reserve(decode_scratch_eight, decode_scratch_ten);
                let _user_qm_scope =
                    crate::bitstream::tile_payload::FrameUserQmScope::install(key_user_qm);
                let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                    frame_engine::intra::build_frame_qm_levels(&key_core),
                );
                let walk = frame_engine::walk_frame::<u8>(
                    decode_scratch_eight,
                    plan,
                    key_candidate,
                    bytes,
                    key_envelope,
                    key_core.clone(),
                    &sequence,
                    options,
                    &frame_engine::FrameSetup::Inter(&state),
                    BitDepth::Eight,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Eight,
                    scope,
                    admission,
                    &mut recon_lane,
                    ring,
                    0,
                )?;
                let rate = key_output_effects.frame_rate(frame_rate);
                PipelineFrame {
                    frame,
                    display_grain: key_display_grain,
                    output_effects: key_output_effects,
                    frame_cdfs: inter::FrameCdfHandle::settled(walk.frame_cdfs),
                    motion_field: inter::MotionFieldHandle::settled(walk.motion_field),
                    ccso_params: ccso_params.map(Arc::new),
                    ccso_grid: inter::CcsoGridHandle::settled(walk.ccso_grid.map(Arc::new)),
                    segment_ids: inter::SegmentIdMapHandle::settled(walk.segment_ids),
                    frame_rate_numerator: rate.numerator,
                    frame_rate_denominator: rate.denominator,
                }
            }
            BitDepthIdc::Ten => {
                let (store, meta) = reference.build_store_ten(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                ring.reserve(decode_scratch_eight, decode_scratch_ten);
                let _user_qm_scope =
                    crate::bitstream::tile_payload::FrameUserQmScope::install(key_user_qm);
                let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                    frame_engine::intra::build_frame_qm_levels(&key_core),
                );
                let walk = frame_engine::walk_frame::<u16>(
                    decode_scratch_ten,
                    plan,
                    key_candidate,
                    bytes,
                    key_envelope,
                    key_core.clone(),
                    &sequence,
                    options,
                    &frame_engine::FrameSetup::Inter(&state),
                    BitDepth::Ten,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Ten,
                    scope,
                    admission,
                    &mut recon_lane,
                    ring,
                    0,
                )?;
                let rate = key_output_effects.frame_rate(frame_rate);
                PipelineFrame {
                    frame,
                    display_grain: key_display_grain,
                    output_effects: key_output_effects,
                    frame_cdfs: inter::FrameCdfHandle::settled(walk.frame_cdfs),
                    motion_field: inter::MotionFieldHandle::settled(walk.motion_field),
                    ccso_params: ccso_params.map(Arc::new),
                    ccso_grid: inter::CcsoGridHandle::settled(walk.ccso_grid.map(Arc::new)),
                    segment_ids: inter::SegmentIdMapHandle::settled(walk.segment_ids),
                    frame_rate_numerator: rate.numerator,
                    frame_rate_denominator: rate.denominator,
                }
            }
        }
    } else {
        decode_key_frame_with_effects(
            decode_scratch_eight,
            decode_scratch_ten,
            scope,
            admission,
            &mut recon_lane,
            ring,
            0,
            bytes,
            options,
            plan,
            key_candidate,
            key_envelope,
            key_core.clone(),
            &sequence,
            frame_rate,
            key_display_grain,
            key_user_qm,
            key_output_effects,
        )?
    };
    retained_frame_bytes =
        ensure_retained_frame_byte_limits(options.limits(), retained_frame_bytes, &key_frame)?;
    let key_update = frame_ref_update_from_core(
        &key_core,
        key_envelope.offset,
        key_envelope.header.embedded_layer_id,
    )?;
    frames.push(Some(key_frame));
    let key_hint = key_update.order_hint;
    let key_implicit = key_core.implicit_output_frame == Some(true);
    let key_immediate = key_core.immediate_output_frame == Some(true);
    let evicted = scheduler.refresh(
        key_update.refresh_frame_flags,
        0,
        key_hint,
        key_implicit,
        true,
    );
    charge_emitted_outputs(
        options,
        &frames,
        &scheduler,
        &mut emission_queue,
        &evicted,
        &mut emit,
    )?;
    reference.update(0, &key_update, is_key_or_switch(&key_core));
    reference.note_frame(
        key_envelope.header.obu_type,
        true,
        &key_update,
        &key_core.ref_long_term_ids,
    );
    in_band_long_term_prelude.note_frame(&key_core, 0);
    scheduler.note_frame(
        key_envelope.header.obu_type,
        true,
        key_hint,
        key_immediate,
        key_implicit,
    );
    if key_immediate && !scheduler.already_emitted(0) {
        let emitted = scheduler.on_immediate(0, key_hint);
        charge_emitted_outputs(
            options,
            &frames,
            &scheduler,
            &mut emission_queue,
            &emitted,
            &mut emit,
        )?;
    }
    if !retain_decoded_frames {
        reclaim_unowned_frames(
            &mut frames,
            &reference,
            &scheduler,
            &emission_queue,
            ring,
            &mut retained_frame_bytes,
        )?;
    }
    if output_frame_limit_reached(options, scheduler.emitted.len()) {
        emission_queue.flush(&frames, &mut emit)?;
        return if retain_decoded_frames {
            select_output_frames(frames, scheduler.emitted)
        } else {
            Ok(Vec::new())
        };
    }

    let mut decoding_initial_tu = true;
    let mut pending_entropy = frame_pipeline::PendingEntropyQueue::default();
    let mut shared_sequence = None;
    for next_candidate in candidates {
        match next_candidate.obu_type() {
            ObuType::LeadingSef | ObuType::RegularSef => {
                frame_pipeline::drain_entropy_before_barrier(
                    &mut pending_entropy,
                    scope,
                    admission,
                    &mut recon_lane,
                );
                let (sef_prefix_obus, sef_envelope) = match stream {
                    RuntimeStream::AnnexB { obus } => following_annexb_inter_envelope(
                        obus,
                        next_candidate,
                        &mut next_unvalidated_following_annexb_obu,
                    )?,
                    RuntimeStream::Ivf { ivf, .. } => following_inter_envelope(
                        ivf,
                        next_candidate,
                        &mut next_unvalidated_following_ivf_record,
                    )?,
                };
                let ivf_frame_index = next_candidate.ivf_frame().map(IvfFrameContext::frame_index);
                output_effect_state.observe_prefix(sef_prefix_obus, &sequence)?;
                film_grain_slots.update_from_obus(sef_prefix_obus)?;
                let first_picture_in_tu = sef_prefix_obus
                    .iter()
                    .any(|obu| obu.header.obu_type == ObuType::TemporalDelimiter);
                if first_picture_in_tu {
                    decoding_initial_tu = false;
                }
                in_band_long_term_prelude.begin_frame(first_picture_in_tu);
                let flushed =
                    scheduler.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    &mut emit,
                )?;
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
                reference.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                let sef_core = match sequence.general.bit_depth_idc {
                    BitDepthIdc::Eight => {
                        let (store, meta) = reference.build_store_eight(&frames)?;
                        let state = inter::InterReferenceState::from_metadata(store, meta);
                        parse_inter_core_with_effects(
                            sef_envelope,
                            &sequence,
                            &state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?
                    }
                    BitDepthIdc::Ten => {
                        let (store, meta) = reference.build_store_ten(&frames)?;
                        let state = inter::InterReferenceState::from_metadata(store, meta);
                        parse_inter_core_with_effects(
                            sef_envelope,
                            &sequence,
                            &state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?
                    }
                };
                let next_output_frame_count = checked_add(
                    DecodeLimitName::MaxOutputFrames,
                    scheduler.emitted.len() as u64,
                    1,
                )?;
                ensure_output_frame_count_limit(options.limits(), next_output_frame_count)?;
                let _ = output_effect_state.prepare_frame(
                    sef_envelope,
                    &sef_core,
                    &sequence,
                    first_picture_in_tu,
                    ivf_frame_index,
                )?;
                let display_grain =
                    film_grain_slots.active_for_core(&sef_core, sef_envelope.offset)?;
                output_effect_state.observe_suffix(frame_suffix_obus(stream, next_candidate)?)?;
                let output_effects = output_effect_state.finish_frame();
                let output_rate = output_effects.frame_rate(frame_rate);
                let slot = sef_core.frame_to_show_map_idx.ok_or_else(|| {
                    unsupported_at(
                        "sef_missing_frame_to_show_map_idx",
                        sef_envelope.offset,
                        "show-existing-frame output requires frame_to_show_map_idx",
                    )
                })?;
                let source_index = reference.frame_index_for_slot(slot)?;
                if sef_core.derive_sef_order_hint == Some(true) {
                    reference
                        .mark_sef_derive_output(slot, scheduler.already_emitted(source_index))?;
                }
                reference.note_show_existing();
                let source = frames
                    .get(source_index)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| {
                        unsupported_at(
                            "sef_reference_frame_unavailable",
                            sef_envelope.offset,
                            "show-existing-frame output requires its retained decoded frame",
                        )
                    })?;
                let sef_frame = PipelineFrame {
                    frame: inflight::PipelineFrameSlot::completed(source.wait_ready_frame()?),
                    display_grain,
                    output_effects,
                    frame_cdfs: source.frame_cdfs.clone(),
                    motion_field: source.motion_field.clone(),
                    ccso_params: source.ccso_params.clone(),
                    ccso_grid: source.ccso_grid.clone(),
                    segment_ids: source.segment_ids.clone(),
                    frame_rate_numerator: output_rate.numerator,
                    frame_rate_denominator: output_rate.denominator,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &sef_frame,
                )?;
                let frame_index = frames.len();
                frames.push(Some(sef_frame));
                retained_frame_bytes = next_retained_frame_bytes;
                let ordering = sef_core.display_order_hint().ok_or_else(|| {
                    unsupported_at(
                        "sef_missing_display_order_hint",
                        sef_envelope.offset,
                        "show-existing-frame output requires a derived display order hint",
                    )
                })?;
                scheduler.note_frame(
                    next_candidate.obu_type(),
                    first_picture_in_tu,
                    ordering,
                    true,
                    false,
                );
                let emitted = scheduler.on_immediate(frame_index, ordering);
                charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &emitted,
                    &mut emit,
                )?;
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        &mut frames,
                        &reference,
                        &scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
            }
            ObuType::LeadingTileGroup
            | ObuType::RegularTileGroup
            | ObuType::Switch
            | ObuType::RasFrame
            | ObuType::LeadingTip
            | ObuType::RegularTip
            | ObuType::BridgeFrame => {
                let (inter_prefix_obus, inter_envelope) = match stream {
                    RuntimeStream::AnnexB { obus } => following_annexb_inter_envelope(
                        obus,
                        next_candidate,
                        &mut next_unvalidated_following_annexb_obu,
                    )?,
                    RuntimeStream::Ivf { ivf, .. } => following_inter_envelope(
                        ivf,
                        next_candidate,
                        &mut next_unvalidated_following_ivf_record,
                    )?,
                };
                let ivf_frame_index = next_candidate.ivf_frame().map(IvfFrameContext::frame_index);
                output_effect_state.observe_prefix(inter_prefix_obus, &sequence)?;
                film_grain_slots.update_from_obus(inter_prefix_obus)?;
                let first_picture_in_tu = inter_prefix_obus
                    .iter()
                    .any(|obu| obu.header.obu_type == ObuType::TemporalDelimiter);
                if first_picture_in_tu {
                    decoding_initial_tu = false;
                }
                in_band_long_term_prelude.begin_frame(first_picture_in_tu);
                let flushed =
                    scheduler.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    &mut emit,
                )?;
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
                reference.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                if matches!(
                    next_candidate.obu_type(),
                    ObuType::Switch | ObuType::RasFrame
                ) {
                    let activation = match sequence.general.bit_depth_idc {
                        BitDepthIdc::Eight => {
                            let (store, meta) = reference.build_store_eight(&frames)?;
                            let state = inter::InterReferenceState::from_metadata(store, meta);
                            inter::parse_inter_frame_activation(
                                inter_envelope,
                                &sequence,
                                &state,
                                first_picture_in_tu,
                                ivf_frame_index,
                            )?
                        }
                        BitDepthIdc::Ten => {
                            let (store, meta) = reference.build_store_ten(&frames)?;
                            let state = inter::InterReferenceState::from_metadata(store, meta);
                            inter::parse_inter_frame_activation(
                                inter_envelope,
                                &sequence,
                                &state,
                                first_picture_in_tu,
                                ivf_frame_index,
                            )?
                        }
                    };
                    if next_candidate.obu_type() == ObuType::RasFrame && decoding_initial_tu {
                        in_band_long_term_prelude.validate_required(
                            &activation,
                            &reference,
                            inter_envelope.offset,
                        )?;
                    }
                    let restricted = activation.restricted_prediction_switch;
                    if restricted == Some(true) {
                        let presence = sequence.general.mlayer_dependency_map.presence_map();
                        let slots = reference.restrict_references_for_switch(
                            inter_envelope.header.embedded_layer_id,
                            &presence,
                        );
                        let emitted = scheduler.restrict_slots(&slots);
                        charge_emitted_outputs(
                            options,
                            &frames,
                            &scheduler,
                            &mut emission_queue,
                            &emitted,
                            &mut emit,
                        )?;
                        if output_frame_limit_reached(options, scheduler.emitted.len()) {
                            break;
                        }
                    }
                }
                let frame_index = frames.len();
                let decoded = match sequence.general.bit_depth_idc {
                    BitDepthIdc::Eight => {
                        let (store, meta) = reference.build_store_eight(&frames)?;
                        let inter_state = inter::InterReferenceState::from_metadata(store, meta);
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        let user_qm = output_effect_state.prepare_frame(
                            inter_envelope,
                            &inter_core,
                            &sequence,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        if frame_is_output(&inter_core) {
                            let next_output_frame_count = checked_add(
                                DecodeLimitName::MaxOutputFrames,
                                scheduler.emitted.len() as u64,
                                1,
                            )?;
                            ensure_output_frame_count_limit(
                                options.limits(),
                                next_output_frame_count,
                            )?;
                        }
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained_frame_bytes,
                            &inter_core,
                            &sequence,
                            inter_envelope.offset,
                        )?;
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if splot_parallel::current_pool_width() > 1
                            && inter::splittable_inter_frame(next_candidate.obu_type(), &inter_core)
                        {
                            frame_pipeline::prepare_entropy_submission(
                                &mut pending_entropy,
                                ring.capacity(),
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let records = decode_scratch_eight.take_frame_filter_records();
                            let quantizer =
                                crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            let geometry = inter::FrameDecodeGeometry::new(
                                &inter_core,
                                &sequence,
                                BitDepth::Eight,
                                false,
                            )?;
                            let (slot, finish) = inflight::reserve_pending_slot(
                                geometry.info(),
                                inflight::PipelineFrameSlot::Eight,
                                ring,
                                frame_index,
                            )?;
                            let dependencies =
                                inter::entropy_dependencies(&inter_core, &sequence, &inter_state);
                            let frame_cdfs = inter::FrameCdfHandle::pending();
                            let ccso_grid = inter::CcsoGridHandle::pending();
                            let segment_ids = inter::SegmentIdMapHandle::pending();
                            let motion = inter::MotionFieldHandle::pending_with_layout(
                                geometry.motion_layout(),
                            );
                            let products = (
                                slot,
                                Arc::new(inter_core.clone()),
                                frame_cdfs.clone(),
                                ccso_grid.clone(),
                                segment_ids.clone(),
                                motion.clone(),
                            );
                            let parse_progress = Arc::new(inter::ParseProgress::default());
                            let result = frame_pipeline::schedule_entropy(
                                move |publish_early| {
                                    let _scopes = quantizer.install_frame();
                                    let (early, pending) = inter::parse_inter_frame_prologue(
                                        records,
                                        plan,
                                        next_candidate,
                                        bytes,
                                        inter_envelope,
                                        inter_core,
                                        &shared,
                                        options,
                                        inter_state,
                                        BitDepth::Eight,
                                        geometry,
                                        &motion,
                                        &parse_progress,
                                    )?;
                                    publish_early(early);
                                    pending.run()
                                },
                                frame_index,
                                frame_cdfs,
                                ccso_grid,
                                segment_ids,
                                products.5.clone(),
                                &dependencies,
                                admission,
                                scope,
                            );
                            pending_entropy.push(frame_pipeline::PendingEntropy::Eight {
                                frame_index,
                                result,
                                finish,
                            });
                            products
                        } else if inter_envelope.header.obu_type.is_tip_frame() {
                            frame_pipeline::drain_entropy_before_barrier(
                                &mut pending_entropy,
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let (
                                slot,
                                finish,
                                geometry,
                                frame_cdfs,
                                ccso_grid,
                                segment_ids,
                                motion,
                            ) = frame_pipeline::reserve_tip_output(
                                &inter_core,
                                &sequence,
                                BitDepth::Eight,
                                inflight::PipelineFrameSlot::Eight,
                                ring,
                                frame_index,
                            )?;
                            let dependencies = inter::tip_output_dependencies(
                                &inter_core,
                                &sequence,
                                &inter_state,
                            );
                            let conditions = dependencies.conditions();
                            let task_core = inter_core.clone();
                            let core = Arc::new(inter_core);
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            frame_pipeline::schedule_tip_output(
                                move |scratch| {
                                    inter::decode_tip_output_frame(
                                        scratch,
                                        next_candidate,
                                        inter_envelope,
                                        task_core,
                                        &shared,
                                        options,
                                        &inter_state,
                                        BitDepth::Eight,
                                        geometry,
                                    )
                                },
                                frame_index,
                                &conditions,
                                frame_cdfs.clone(),
                                ccso_grid.clone(),
                                segment_ids.clone(),
                                motion.clone(),
                                finish,
                                admission,
                                scope,
                                &mut recon_lane,
                            );
                            (slot, core, frame_cdfs, ccso_grid, segment_ids, motion)
                        } else {
                            frame_pipeline::drain_entropy_before_barrier(
                                &mut pending_entropy,
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let setup = if inter_core.status
                                == splot_core::headers::frame::FrameHeaderParseStatus::IntraHeaderComplete
                            {
                                frame_engine::FrameSetup::Intra
                            } else {
                                frame_engine::FrameSetup::Inter(&inter_state)
                            };
                            let walk = frame_engine::walk_frame(
                                decode_scratch_eight,
                                plan,
                                next_candidate,
                                bytes,
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &setup,
                                BitDepth::Eight,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Eight,
                                scope,
                                admission,
                                &mut recon_lane,
                                ring,
                                frame_index,
                            )?;
                            (
                                slot,
                                inter_core,
                                inter::FrameCdfHandle::settled(walk.frame_cdfs),
                                inter::CcsoGridHandle::settled(walk.ccso_grid.map(Arc::new)),
                                inter::SegmentIdMapHandle::settled(walk.segment_ids),
                                inter::MotionFieldHandle::settled(walk.motion_field),
                            )
                        }
                    }
                    BitDepthIdc::Ten => {
                        let (store, meta) = reference.build_store_ten(&frames)?;
                        let inter_state = inter::InterReferenceState::from_metadata(store, meta);
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        let user_qm = output_effect_state.prepare_frame(
                            inter_envelope,
                            &inter_core,
                            &sequence,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        if frame_is_output(&inter_core) {
                            let next_output_frame_count = checked_add(
                                DecodeLimitName::MaxOutputFrames,
                                scheduler.emitted.len() as u64,
                                1,
                            )?;
                            ensure_output_frame_count_limit(
                                options.limits(),
                                next_output_frame_count,
                            )?;
                        }
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained_frame_bytes,
                            &inter_core,
                            &sequence,
                            inter_envelope.offset,
                        )?;
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if splot_parallel::current_pool_width() > 1
                            && inter::splittable_inter_frame(next_candidate.obu_type(), &inter_core)
                        {
                            frame_pipeline::prepare_entropy_submission(
                                &mut pending_entropy,
                                ring.capacity(),
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let records = decode_scratch_ten.take_frame_filter_records();
                            let quantizer =
                                crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            let geometry = inter::FrameDecodeGeometry::new(
                                &inter_core,
                                &sequence,
                                BitDepth::Ten,
                                false,
                            )?;
                            let (slot, finish) = inflight::reserve_pending_slot(
                                geometry.info(),
                                inflight::PipelineFrameSlot::Ten,
                                ring,
                                frame_index,
                            )?;
                            let dependencies =
                                inter::entropy_dependencies(&inter_core, &sequence, &inter_state);
                            let frame_cdfs = inter::FrameCdfHandle::pending();
                            let ccso_grid = inter::CcsoGridHandle::pending();
                            let segment_ids = inter::SegmentIdMapHandle::pending();
                            let motion = inter::MotionFieldHandle::pending_with_layout(
                                geometry.motion_layout(),
                            );
                            let products = (
                                slot,
                                Arc::new(inter_core.clone()),
                                frame_cdfs.clone(),
                                ccso_grid.clone(),
                                segment_ids.clone(),
                                motion.clone(),
                            );
                            let parse_progress = Arc::new(inter::ParseProgress::default());
                            let result = frame_pipeline::schedule_entropy(
                                move |publish_early| {
                                    let _scopes = quantizer.install_frame();
                                    let (early, pending) = inter::parse_inter_frame_prologue(
                                        records,
                                        plan,
                                        next_candidate,
                                        bytes,
                                        inter_envelope,
                                        inter_core,
                                        &shared,
                                        options,
                                        inter_state,
                                        BitDepth::Ten,
                                        geometry,
                                        &motion,
                                        &parse_progress,
                                    )?;
                                    publish_early(early);
                                    pending.run()
                                },
                                frame_index,
                                frame_cdfs,
                                ccso_grid,
                                segment_ids,
                                products.5.clone(),
                                &dependencies,
                                admission,
                                scope,
                            );
                            pending_entropy.push(frame_pipeline::PendingEntropy::Ten {
                                frame_index,
                                result,
                                finish,
                            });
                            products
                        } else if inter_envelope.header.obu_type.is_tip_frame() {
                            frame_pipeline::drain_entropy_before_barrier(
                                &mut pending_entropy,
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let (
                                slot,
                                finish,
                                geometry,
                                frame_cdfs,
                                ccso_grid,
                                segment_ids,
                                motion,
                            ) = frame_pipeline::reserve_tip_output(
                                &inter_core,
                                &sequence,
                                BitDepth::Ten,
                                inflight::PipelineFrameSlot::Ten,
                                ring,
                                frame_index,
                            )?;
                            let dependencies = inter::tip_output_dependencies(
                                &inter_core,
                                &sequence,
                                &inter_state,
                            );
                            let conditions = dependencies.conditions();
                            let task_core = inter_core.clone();
                            let core = Arc::new(inter_core);
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            frame_pipeline::schedule_tip_output(
                                move |scratch| {
                                    inter::decode_tip_output_frame(
                                        scratch,
                                        next_candidate,
                                        inter_envelope,
                                        task_core,
                                        &shared,
                                        options,
                                        &inter_state,
                                        BitDepth::Ten,
                                        geometry,
                                    )
                                },
                                frame_index,
                                &conditions,
                                frame_cdfs.clone(),
                                ccso_grid.clone(),
                                segment_ids.clone(),
                                motion.clone(),
                                finish,
                                admission,
                                scope,
                                &mut recon_lane,
                            );
                            (slot, core, frame_cdfs, ccso_grid, segment_ids, motion)
                        } else {
                            frame_pipeline::drain_entropy_before_barrier(
                                &mut pending_entropy,
                                scope,
                                admission,
                                &mut recon_lane,
                            );
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let setup = if inter_core.status
                                == splot_core::headers::frame::FrameHeaderParseStatus::IntraHeaderComplete
                            {
                                frame_engine::FrameSetup::Intra
                            } else {
                                frame_engine::FrameSetup::Inter(&inter_state)
                            };
                            let walk = frame_engine::walk_frame(
                                decode_scratch_ten,
                                plan,
                                next_candidate,
                                bytes,
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &setup,
                                BitDepth::Ten,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Ten,
                                scope,
                                admission,
                                &mut recon_lane,
                                ring,
                                frame_index,
                            )?;
                            (
                                slot,
                                inter_core,
                                inter::FrameCdfHandle::settled(walk.frame_cdfs),
                                inter::CcsoGridHandle::settled(walk.ccso_grid.map(Arc::new)),
                                inter::SegmentIdMapHandle::settled(walk.segment_ids),
                                inter::MotionFieldHandle::settled(walk.motion_field),
                            )
                        }
                    }
                };
                let (inter_slot, inter_core, frame_cdfs, ccso_grid, segment_ids, motion_field) =
                    decoded;
                let inter_display_grain =
                    film_grain_slots.active_for_core(&inter_core, inter_envelope.offset)?;
                output_effect_state.observe_suffix(frame_suffix_obus(stream, next_candidate)?)?;
                let inter_output_effects = output_effect_state.finish_frame();
                let inter_frame_rate = inter_output_effects.frame_rate(frame_rate);
                let inter_update = frame_ref_update_from_core(
                    &inter_core,
                    inter_envelope.offset,
                    inter_envelope.header.embedded_layer_id,
                )?;
                let inter_frame = PipelineFrame {
                    frame: inter_slot,
                    display_grain: inter_display_grain,
                    output_effects: inter_output_effects,
                    frame_cdfs,
                    motion_field,
                    ccso_params: inter_core.ccso_params.clone().map(Arc::new),
                    ccso_grid,
                    segment_ids,
                    frame_rate_numerator: inter_frame_rate.numerator,
                    frame_rate_denominator: inter_frame_rate.denominator,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                frames.push(Some(inter_frame));
                retained_frame_bytes = next_retained_frame_bytes;
                let inter_hint = inter_update.order_hint;
                let inter_implicit = inter_core.implicit_output_frame == Some(true);
                let inter_immediate = inter_core.immediate_output_frame == Some(true);
                let inter_key_or_switch = is_key_or_switch(&inter_core);
                let evicted = scheduler.refresh(
                    inter_update.refresh_frame_flags,
                    frame_index,
                    inter_hint,
                    inter_implicit,
                    inter_key_or_switch,
                );
                charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &evicted,
                    &mut emit,
                )?;
                reference.update(frame_index, &inter_update, inter_key_or_switch);
                reference.note_frame(
                    next_candidate.obu_type(),
                    first_picture_in_tu,
                    &inter_update,
                    &inter_core.ref_long_term_ids,
                );
                scheduler.note_frame(
                    next_candidate.obu_type(),
                    first_picture_in_tu,
                    inter_hint,
                    inter_immediate,
                    inter_implicit,
                );
                if inter_immediate && !scheduler.already_emitted(frame_index) {
                    let emitted = scheduler.on_immediate(frame_index, inter_hint);
                    charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &emitted,
                        &mut emit,
                    )?;
                }
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        &mut frames,
                        &reference,
                        &scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
            }
            ObuType::ClosedLoopKey | ObuType::OpenLoopKey => {
                frame_pipeline::drain_entropy_before_barrier(
                    &mut pending_entropy,
                    scope,
                    admission,
                    &mut recon_lane,
                );
                let starts_new_sequence = next_candidate.obu_type() == ObuType::ClosedLoopKey;
                let (key_sequence_envelope, key_prefix_obus, key_envelope) = if starts_new_sequence
                {
                    let (sequence_envelope, prefix, frame) = match stream {
                        RuntimeStream::AnnexB { obus } => following_annexb_key_frame_unit(
                            obus,
                            next_candidate,
                            &mut next_unvalidated_following_annexb_obu,
                        )?,
                        RuntimeStream::Ivf { ivf, .. } => following_key_frame_unit(
                            ivf,
                            next_candidate,
                            &mut next_unvalidated_following_ivf_record,
                        )?,
                    };
                    (Some(sequence_envelope), prefix, frame)
                } else {
                    let (prefix, frame) = match stream {
                        RuntimeStream::AnnexB { obus } => following_annexb_inter_envelope(
                            obus,
                            next_candidate,
                            &mut next_unvalidated_following_annexb_obu,
                        )?,
                        RuntimeStream::Ivf { ivf, .. } => following_inter_envelope(
                            ivf,
                            next_candidate,
                            &mut next_unvalidated_following_ivf_record,
                        )?,
                    };
                    let sequence_envelope = prefix
                        .iter()
                        .rev()
                        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
                        .copied();
                    (sequence_envelope, prefix, frame)
                };
                let (key_sequence, key_sequence_offset) =
                    if let Some(envelope) = key_sequence_envelope {
                        let parsed = parse_sequence(envelope)?;
                        validate_sequence(&parsed, envelope.offset)?;
                        (parsed, envelope.offset)
                    } else {
                        (sequence.clone(), key_envelope.offset)
                    };
                let key_num_ref_frames = usize::from(
                    key_sequence
                        .inter
                        .as_ref()
                        .ok_or_else(|| {
                            unsupported_at(
                                "missing_sequence_inter_config",
                                key_sequence_offset,
                                "multi-frame decode requires the active sequence inter config (NumRefFrames)",
                            )
                        })?
                        .num_ref_frames,
                );
                if !starts_new_sequence && key_num_ref_frames != reference.len() {
                    return Err(unsupported_at(
                        "olk_reference_buffer_size_change",
                        key_envelope.offset,
                        "open-loop-key sequence activation changed NumRefFrames inside the active coded video sequence",
                    ));
                }
                let first_picture_in_tu = key_prefix_obus
                    .iter()
                    .any(|obu| obu.header.obu_type == ObuType::TemporalDelimiter);
                if first_picture_in_tu {
                    decoding_initial_tu = false;
                }
                in_band_long_term_prelude.begin_frame(first_picture_in_tu);
                if !starts_new_sequence {
                    let flushed =
                        scheduler.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                    charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        &mut emit,
                    )?;
                    if output_frame_limit_reached(options, scheduler.emitted.len()) {
                        break;
                    }
                    reference.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                }
                output_effect_state.observe_prefix(key_prefix_obus, &key_sequence)?;
                film_grain_slots.update_from_obus(key_prefix_obus)?;
                let key_core = if starts_new_sequence {
                    parse_key_core_with_effects(key_envelope, &key_sequence, &output_effect_state)?
                } else {
                    match key_sequence.general.bit_depth_idc {
                        BitDepthIdc::Eight => {
                            let (store, meta) = reference.build_store_eight(&frames)?;
                            let state = inter::InterReferenceState::from_metadata(store, meta);
                            parse_olk_core_with_effects(
                                key_envelope,
                                &key_sequence,
                                &state,
                                &output_effect_state,
                                first_picture_in_tu,
                            )?
                        }
                        BitDepthIdc::Ten => {
                            let (store, meta) = reference.build_store_ten(&frames)?;
                            let state = inter::InterReferenceState::from_metadata(store, meta);
                            parse_olk_core_with_effects(
                                key_envelope,
                                &key_sequence,
                                &state,
                                &output_effect_state,
                                first_picture_in_tu,
                            )?
                        }
                    }
                };
                if decoding_initial_tu {
                    in_band_long_term_prelude.validate_required(
                        &key_core,
                        &reference,
                        key_envelope.offset,
                    )?;
                }
                ensure_intra_header_complete(&key_core, key_envelope.offset)?;
                let key_user_qm = output_effect_state.prepare_frame(
                    key_envelope,
                    &key_core,
                    &key_sequence,
                    first_picture_in_tu,
                    next_candidate.ivf_frame().map(IvfFrameContext::frame_index),
                )?;
                let key_display_grain =
                    film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;

                if starts_new_sequence {
                    let key_reference =
                        reference_buffer::RuntimeReferenceBuffer::new(key_num_ref_frames)?;
                    let flushed = scheduler.start_new_sequence(key_num_ref_frames);
                    charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        &mut emit,
                    )?;
                    reference = key_reference;
                    if !retain_decoded_frames {
                        reclaim_unowned_frames(
                            &mut frames,
                            &reference,
                            &scheduler,
                            &emission_queue,
                            ring,
                            &mut retained_frame_bytes,
                        )?;
                    }
                    if output_frame_limit_reached(options, scheduler.emitted.len()) {
                        break;
                    }
                }

                sequence = key_sequence;
                shared_sequence = None;
                output_effect_state.observe_suffix(frame_suffix_obus(stream, next_candidate)?)?;
                let key_output_effects = output_effect_state.finish_frame();
                ensure_retained_frame_byte_limits_for_core(
                    options.limits(),
                    retained_frame_bytes,
                    &key_core,
                    &sequence,
                    key_envelope.offset,
                )?;
                let frame_index = frames.len();
                let key_frame = decode_key_frame_with_effects(
                    decode_scratch_eight,
                    decode_scratch_ten,
                    scope,
                    admission,
                    &mut recon_lane,
                    ring,
                    frame_index,
                    bytes,
                    options,
                    plan,
                    next_candidate,
                    key_envelope,
                    key_core.clone(),
                    &sequence,
                    frame_rate,
                    key_display_grain,
                    key_user_qm,
                    key_output_effects,
                )?;
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &key_frame,
                )?;
                let key_update = frame_ref_update_from_core(
                    &key_core,
                    key_envelope.offset,
                    key_envelope.header.embedded_layer_id,
                )?;
                frames.push(Some(key_frame));
                retained_frame_bytes = next_retained_frame_bytes;
                let key_hint = key_update.order_hint;
                let key_implicit = key_core.implicit_output_frame == Some(true);
                let key_immediate = key_core.immediate_output_frame == Some(true);
                let evicted = scheduler.refresh(
                    key_update.refresh_frame_flags,
                    frame_index,
                    key_hint,
                    key_implicit,
                    true,
                );
                charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &evicted,
                    &mut emit,
                )?;
                reference.update(frame_index, &key_update, is_key_or_switch(&key_core));
                reference.note_frame(
                    next_candidate.obu_type(),
                    first_picture_in_tu,
                    &key_update,
                    &key_core.ref_long_term_ids,
                );
                in_band_long_term_prelude.note_frame(&key_core, frame_index);
                scheduler.note_frame(
                    next_candidate.obu_type(),
                    first_picture_in_tu,
                    key_hint,
                    key_immediate,
                    key_implicit,
                );
                if key_immediate && !scheduler.already_emitted(frame_index) {
                    let emitted = scheduler.on_immediate(frame_index, key_hint);
                    charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &emitted,
                        &mut emit,
                    )?;
                }
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        &mut frames,
                        &reference,
                        &scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
            }
            _ => {
                return Err(unsupported_at(
                    "non_frame_candidate_in_frame_loop",
                    next_candidate.offset(),
                    "internal invariant violation: non-frame-candidate obu reached the frame decode loop",
                ));
            }
        }
    }

    frame_pipeline::drain_entropy_before_barrier(
        &mut pending_entropy,
        scope,
        admission,
        &mut recon_lane,
    );
    if !output_frame_limit_reached(options, scheduler.emitted.len()) {
        let flushed = scheduler.flush_all();
        charge_emitted_outputs(
            options,
            &frames,
            &scheduler,
            &mut emission_queue,
            &flushed,
            &mut emit,
        )?;
        emission_queue.flush(&frames, &mut emit)?;
        if !retain_decoded_frames {
            ring.harvest_all(decode_scratch_eight, decode_scratch_ten);
            reclaim_unowned_frames(
                &mut frames,
                &reference,
                &scheduler,
                &emission_queue,
                ring,
                &mut retained_frame_bytes,
            )?;
        }
    } else {
        emission_queue.flush(&frames, &mut emit)?;
    }
    if !retain_decoded_frames {
        return Ok(Vec::new());
    }
    let emitted = std::mem::take(&mut scheduler.emitted);
    let limited = match options.output_frame_limit() {
        Some(limit) => {
            let limit = usize::try_from(limit.get()).unwrap_or(usize::MAX);
            emitted.into_iter().take(limit).collect()
        }
        None => emitted,
    };
    select_output_frames(frames, limited)
}

pub(crate) mod frame_engine;
pub(crate) mod general_intra;
pub(crate) mod reconstruct;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_d135_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_d157_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_horizontal_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_d113_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_d157_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_hfollow_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_sdp_d113_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_smooth_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_tests;

#[derive(Clone, Copy)]
enum TileFactsKind {
    Intra,
    Inter,
}
#[allow(clippy::too_many_arguments)]
fn derive_tile_plan_with<'payload>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &'payload [u8],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<&Arc<FrameCdfSubset>>,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    let envelope = planned_envelope(bytes, candidate)?;
    let tq = sequence.transform_quant_entropy.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_tq_entropy_config",
            envelope.offset,
            "decode runtime requires sequence transform/quant/entropy config",
        )
    })?;
    let coeff = FrameCandidateCoeffFacts::from_tq(tq);
    let facts = match kind {
        TileFactsKind::Intra => FrameCandidateTileFacts::from_frame_core(core, coeff),
        TileFactsKind::Inter => FrameCandidateTileFacts::from_inter_frame_core(core, coeff),
    }
    .map_err(decode_tile_boundary_error)?;
    let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
    let candidates = frame_tile_group_candidates(plan, candidate);
    let recorded_header = record_frame_header(envelope, core)?;
    let group_count = candidates.len();
    let mut merged: Option<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> = None;
    for (group_index, group_candidate) in candidates.into_iter().enumerate() {
        let group_envelope = planned_envelope(bytes, group_candidate)?;
        let group_facts = if group_index == 0 {
            facts
        } else {
            facts.with_tile_group_structure_start_bits(continuation_structure_start_bits(
                group_envelope,
                &recorded_header,
            )?)
        };
        let mut input = FrameCandidateTileBoundaryInput::new(
            plan,
            group_candidate,
            bytes,
            group_envelope,
            TileGroupPositionFacts::new(group_index == 0, group_index + 1 == group_count),
            group_facts,
            cdf,
            options.limits(),
        );
        if let Some(cdfs) = initial_cdfs {
            input = input.with_initial_cdfs(Arc::clone(cdfs));
        }
        let group_plan = crate::bitstream::tile_payload::plan_derived_tile_payload_boundary(&input)
            .map_err(decode_tile_boundary_error)?;
        if let Some(plan) = merged.as_mut() {
            plan.append_continuation(group_plan)
                .map_err(FrameCandidateTileBoundaryError::from)
                .map_err(decode_tile_boundary_error)?;
        } else {
            merged = Some(group_plan);
        }
    }
    merged.ok_or_else(|| {
        unsupported_at(
            "missing_tile_group",
            envelope.offset,
            "decode runtime requires at least one tile group for a coded frame",
        )
    })
}

fn frame_tile_group_candidates<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
) -> Vec<&'a DecodePlannedObu> {
    let mut groups = vec![candidate];
    for planned in plan.obus().skip(candidate.index() as usize + 1) {
        if planned.ivf_frame() != candidate.ivf_frame() {
            break;
        }
        if planned.obu_type() == ObuType::Padding {
            continue;
        }
        if planned.role().is_frame_continuation()
            && planned.obu_type() == candidate.obu_type()
            && planned.header().temporal_layer_id == candidate.header().temporal_layer_id
            && planned.header().embedded_layer_id == candidate.header().embedded_layer_id
            && planned.header().extended_layer_id == candidate.header().extended_layer_id
        {
            groups.push(planned);
            continue;
        }
        break;
    }
    groups
}

fn planned_envelope<'a>(bytes: &'a [u8], planned: &DecodePlannedObu) -> Result<ObuEnvelope<'a>> {
    let start = usize::try_from(planned.offset().get()).map_err(|_| {
        unsupported_at(
            "source_range_out_of_bounds",
            planned.offset(),
            "planned tile-group offset is outside the decode input",
        )
    })?;
    let payload_start = start
        .checked_add(usize::from(planned.header().header_size_bytes))
        .ok_or_else(|| {
            unsupported_at(
                "source_range_out_of_bounds",
                planned.offset(),
                "planned tile-group payload offset overflowed",
            )
        })?;
    let end = start.checked_add(planned.size() as usize).ok_or_else(|| {
        unsupported_at(
            "source_range_out_of_bounds",
            planned.offset(),
            "planned tile-group end offset overflowed",
        )
    })?;
    let payload = bytes.get(payload_start..end).ok_or_else(|| {
        unsupported_at(
            "source_range_out_of_bounds",
            planned.offset(),
            "planned tile-group payload is outside the decode input",
        )
    })?;
    Ok(ObuEnvelope {
        offset: planned.offset(),
        size: planned.size(),
        header: planned.header(),
        payload,
    })
}

fn record_frame_header(
    envelope: ObuEnvelope<'_>,
    core: &FrameHeaderCore,
) -> Result<RecordedFrameHeaderBits> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if reader.read_bit().ok() != Some(1) {
        return Err(unsupported_at(
            "missing_first_tile_group",
            envelope.offset,
            "coded frame must begin with is_first_tile_group equal to 1",
        ));
    }
    RecordedFrameHeaderBits::record(&mut reader, core.consumed_bits).map_err(|_| {
        unsupported_at(
            "frame_header_copy_source_truncated",
            envelope.offset,
            "first tile-group frame header could not be recorded for continuation validation",
        )
    })
}

fn continuation_structure_start_bits(
    envelope: ObuEnvelope<'_>,
    recorded: &RecordedFrameHeaderBits,
) -> Result<u64> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if reader.read_bit().ok() != Some(0) {
        return Err(unsupported_at(
            "unexpected_first_tile_group",
            envelope.offset,
            "tile-group continuation must set is_first_tile_group to 0",
        ));
    }
    let frame_header_present = reader.read_bit().map_err(|_| {
        unsupported_at(
            "tile_group_prefix_parse",
            envelope.offset,
            "tile-group continuation ends before frame_header_present_flag",
        )
    })? != 0;
    if frame_header_present {
        match parse_frame_header_copy(&mut reader, recorded) {
            FrameHeaderCopyOutcome::Matches => {}
            FrameHeaderCopyOutcome::Mismatch { .. } => {
                return Err(unsupported_at(
                    "frame_header_copy_mismatch",
                    envelope.offset,
                    "tile-group continuation frame_header_copy differs from the first group",
                ));
            }
            FrameHeaderCopyOutcome::Truncated { .. } => {
                return Err(unsupported_at(
                    "frame_header_copy_truncated",
                    envelope.offset,
                    "tile-group continuation ends inside frame_header_copy",
                ));
            }
            _ => {
                return Err(unsupported_at(
                    "frame_header_copy_invalid",
                    envelope.offset,
                    "tile-group continuation frame_header_copy is invalid",
                ));
            }
        }
    }
    Ok(reader.consumed_bits())
}

pub(crate) fn derive_tile_plan<'payload>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &'payload [u8],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        sequence,
        core,
        options,
        TileFactsKind::Intra,
        None,
    )
}
pub(crate) fn derive_inter_tile_plan<'payload>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &'payload [u8],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    initial_cdfs: &Arc<FrameCdfSubset>,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        sequence,
        core,
        options,
        TileFactsKind::Inter,
        Some(initial_cdfs),
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
