// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode pipeline orchestration for the supported decode runtime.
//!
//! The driver walks frames strictly in decode order and runs every sequential
//! state machine (output effects, film-grain slots, reference buffer, output
//! scheduler) at the same program point relative to each frame's walk. Only the
//! § 7.2 filter phase moves: at a resolved frame-delay depth above one it is
//! handed to the worker pool through [`inflight::FinishSpawner`], and the driver
//! is the only thread that blocks on a frame's samples.

use core::num::NonZeroUsize;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{FrameHeaderCore, FrameSize, FrameType, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader};
use splot_core::headers::tile_group::{
    FrameHeaderCopyOutcome, RecordedFrameHeaderBits, parse_frame_header_copy,
};
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
#[cfg(test)]
use splot_core::stream::ParsedBitstream;
use splot_core::symbol::SymbolDecoder;
use splot_core::types::ObuType;
use splot_recon::BitDepth;
#[cfg(test)]
use splot_recon::DecodedFrame;

use crate::bitstream::byte_stream::FlatParsedBitstream;
#[cfg(test)]
use crate::bitstream::byte_stream::parse_bounded_bitstream;
use crate::bitstream::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    GeneralIntraBlockModeError, GeneralIntraResidualError, TileGroupPositionFacts,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::prediction::inter;
use crate::reference::buffer as reference_buffer;
use crate::support::pipeline_limits::{checked_add, decoded_frame_storage_budget};
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

mod frame_lifecycle;
mod frame_pipeline;
pub(crate) mod frame_progress;
pub(crate) mod inflight;
pub(crate) mod output_effects;
mod output_schedule;
mod stream_schedule;

#[cfg(test)]
pub(crate) use frame_lifecycle::incomplete_intra_header_error;
use frame_lifecycle::*;
pub(crate) use frame_lifecycle::{
    ActiveFilmGrain, PipelineDecodedFrame, PipelineFrame, PipelineFrameRate, deblock_quant_deltas,
    derive_visible_luma_rect, effective_allow_screen_content_tools,
    ensure_runtime_storage_bit_depth, frame_ref_update_from_core, parse_frame_core,
    parse_frame_core_with_reference, parse_sequence,
};
use output_effects::{FrameOutputEffects, OutputEffectState};
use output_schedule::*;
pub(crate) use stream_schedule::following_inter_envelope;
#[cfg(test)]
pub(crate) use stream_schedule::require_minimal_obu_order;
use stream_schedule::*;

const SPEC_SECTION: &str = "7.1";

pub(crate) const GENERAL_INTRA_PARTITION_SPEC_SECTION: &str = "5.20.3.1";
pub(crate) const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
pub(crate) const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";

#[cfg(test)]
fn discard_runtime_noops(parsed: &mut ParsedBitstream<'_>) {
    match parsed {
        ParsedBitstream::AnnexB(partial) => partial
            .obus
            .retain(|obu| !obu.header.obu_type.is_reserved()),
        ParsedBitstream::Ivf(ivf) => {
            for frame in &mut ivf.frames {
                frame.obus.retain(|obu| !obu.header.obu_type.is_reserved());
            }
            ivf.frames.retain(|frame| frame.frame.size != 0);
        }
    }
}

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
    let runtime_parse_timer = crate::timing::start();
    let mut parsed = parse_bounded_bitstream(bytes, options.limits())?;
    crate::timing::report("runtime_reparse", runtime_parse_timer);
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

pub(crate) fn decode_frames_from_prepared(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
) -> Result<Vec<PipelineFrame>> {
    decode_frames_from_plan_impl(
        parsed,
        bytes,
        options,
        plan,
        frame_delay,
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
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<()> {
    decode_frames_from_plan_impl(
        parsed,
        bytes,
        options,
        plan,
        frame_delay,
        |_| Ok(()),
        false,
        emit,
    )
    .map(drop)
}

/// Retires the settled frames nothing owns any more, subtracting their bytes
/// from the live-frame accounting.
///
/// A frame the in-flight ring still holds is skipped: the ring keeps a second
/// slot handle, so its planes stay alive whatever else releases them, and
/// subtracting the bytes would let the live-frame peak run above
/// [`crate::DecodeLimitName::MaxReferenceStoreBytes`]. The driver rescans every
/// frame on each call, so the skipped frame is reclaimed on the first pass after
/// its finish is harvested.
fn reclaim_unowned_frames(
    frames: &mut [Option<PipelineFrame>],
    reference: &reference_buffer::RuntimeReferenceBuffer,
    scheduler: &OutputScheduler,
    emission: &output_schedule::EmissionQueue,
    ring: &inflight::InflightRing,
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
        frame.frame.reclaim_planes();
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
    let record = resolve_mfh_record(envelope, sequence, effects, activation.cur_mfh_id)?;
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
    let record = resolve_mfh_record(envelope, sequence, effects, activation.cur_mfh_id)?;
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

fn resolve_mfh_record<'a>(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    effects: &'a OutputEffectState,
    mfh_id: splot_core::hls::MfhId,
) -> Result<&'a splot_core::hls::MultiFrameHeaderRecord> {
    let record = effects.mfh_record(mfh_id).ok_or_else(|| {
        unsupported_feature_at(
            "multi_frame_header_unavailable",
            envelope.offset,
            "frame references a multi-frame header that is not available in-band",
            "7.3.8.7",
        )
    })?;
    if record.mfh_seq_header_id != sequence.general.seq_header_id {
        return Err(unsupported_feature_at(
            "multi_frame_header_sequence_mismatch",
            envelope.offset,
            "referenced multi-frame header resolves to a different sequence header",
            "7.3.8.7",
        ));
    }
    if !sequence
        .general
        .mlayer_dependency_map
        .depends_on(envelope.header.embedded_layer_id, record.mfh_mlayer_id)
        || !sequence.general.tlayer_dependency_map.depends_on(
            envelope.header.embedded_layer_id,
            envelope.header.temporal_layer_id,
            record.mfh_tlayer_id,
        )
    {
        return Err(unsupported_feature_at(
            "multi_frame_header_layer_dependency",
            envelope.offset,
            "referenced multi-frame header violates the active layer dependency maps",
            "7.3.8.7",
        ));
    }
    Ok(record)
}

fn parse_inter_core_with_effects(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &inter::InterReferenceState<impl splot_recon::ReconSample>,
    effects: &OutputEffectState,
    first_picture_in_tu: bool,
) -> Result<FrameHeaderCore> {
    let activation =
        inter::parse_inter_frame_activation(envelope, sequence, reference, first_picture_in_tu)?;
    let record = if activation.cur_mfh_id.is_zero() {
        None
    } else {
        Some(resolve_mfh_record(
            envelope,
            sequence,
            effects,
            activation.cur_mfh_id,
        )?)
    };
    inter::parse_validated_inter_frame_core_with_mfh(
        envelope,
        sequence,
        reference,
        first_picture_in_tu,
        record,
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
    let mut scratch_eight = inter::InterDecodeScratch::default();
    let mut scratch_ten = inter::InterDecodeScratch::default();
    let mut ring = inflight::InflightRing::new(NonZeroUsize::MIN);
    decode_key_frame_with_effects(
        &mut scratch_eight,
        &mut scratch_ten,
        &inflight::FinishSpawner::Inline,
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
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_key_frame_with_effects(
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
    spawner: &inflight::FinishSpawner<'_, '_>,
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
) -> Result<PipelineFrame> {
    ring.reserve(scratch_eight, scratch_ten);
    let _user_qm_scope = crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
    let (frame, frame_cdfs, ccso_params, ccso_grid, motion_field) =
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
                    spawner,
                    ring,
                    frame_index,
                    scratch_eight,
                )?;
                (
                    frame,
                    walk.frame_cdfs,
                    ccso_params,
                    walk.ccso_grid,
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
                    spawner,
                    ring,
                    frame_index,
                    scratch_ten,
                )?;
                (
                    frame,
                    walk.frame_cdfs,
                    ccso_params,
                    walk.ccso_grid,
                    walk.motion_field,
                )
            }
        };
    let frame_rate = output_effects.frame_rate(frame_rate);
    Ok(PipelineFrame {
        frame,
        display_grain,
        output_effects,
        frame_cdfs,
        motion_field: inter::MotionFieldHandle::settled(motion_field),
        ccso_params,
        ccso_grid,
        frame_rate_numerator: frame_rate.numerator,
        frame_rate_denominator: frame_rate.denominator,
    })
}

pub(crate) fn decode_frames_from_prepared_with_ivf_preflight(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
) -> Result<Vec<PipelineFrame>> {
    decode_frames_from_plan_impl(
        parsed,
        bytes,
        options,
        plan,
        frame_delay,
        preflight,
        true,
        |_| Ok(()),
    )
}

/// Runs the frame loop, pipelined when the resolved frame-delay depth is above
/// one and the caller is inside a multi-worker pool, and serially otherwise.
#[allow(clippy::too_many_arguments)]
fn decode_frames_from_plan_impl(
    parsed: &FlatParsedBitstream<'_>,
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<Vec<PipelineFrame>> {
    if frame_delay.get() == 1 || !splot_parallel::on_multiworker_pool() {
        return drive_frames(
            parsed,
            bytes,
            options,
            plan,
            NonZeroUsize::MIN,
            preflight,
            retain_decoded_frames,
            emit,
            &inflight::FinishSpawner::Inline,
        );
    }
    splot_parallel::ready_task_scope(|scope| {
        drive_frames(
            parsed,
            bytes,
            options,
            plan,
            frame_delay,
            preflight,
            retain_decoded_frames,
            emit,
            &inflight::FinishSpawner::Deferred(scope),
        )
    })?
}

/// Owns the decode scratch and the in-flight ring for one decode, and resolves
/// the run's outcome against the filter phases the ring collected.
///
/// A filter-phase failure outranks the frame loop's own error: serial decode
/// would have run that frame's filters before reaching the later error, so the
/// lowest-indexed collected failure is the one the caller sees.
#[allow(clippy::too_many_arguments)]
fn drive_frames(
    parsed: &FlatParsedBitstream<'_>,
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()>,
    spawner: &inflight::FinishSpawner<'_, '_>,
) -> Result<Vec<PipelineFrame>> {
    let inflight_timer = crate::timing::start();
    let mut decode_scratch_eight = inter::InterDecodeScratch::default();
    let mut decode_scratch_ten = inter::InterDecodeScratch::default();
    let mut ring = inflight::InflightRing::new(frame_delay);
    let decoded = decode_frames_in_order(
        parsed,
        bytes,
        options,
        plan,
        preflight,
        retain_decoded_frames,
        emit,
        spawner,
        &mut ring,
        &mut decode_scratch_eight,
        &mut decode_scratch_ten,
    );
    ring.harvest_all(&mut decode_scratch_eight, &mut decode_scratch_ten);
    if inflight_timer.is_some() {
        crate::timing::report_detail(
            "pipeline_inflight",
            inflight_timer,
            &format!("max_in_flight={}", ring.max_in_flight()),
        );
    }
    crate::timing::report_phases();
    match ring.take_failure() {
        Some(failure) => Err(failure),
        None => decoded,
    }
}

/// Whether one frame's walk runs split, with its reconstruction deferred past
/// the next frame's entropy pass.
///
/// Only the pipelined driver defers: a serial driver has no scope to hand the
/// filter phase to and no next frame to overlap with, so it keeps the fused
/// walk.
fn split_walk(
    spawner: &inflight::FinishSpawner<'_, '_>,
    obu_type: ObuType,
    core: &FrameHeaderCore,
) -> bool {
    matches!(spawner, inflight::FinishSpawner::Deferred(_))
        && inter::splittable_inter_frame(obu_type, core)
}

#[allow(clippy::too_many_arguments)]
fn decode_frames_in_order(
    parsed: &FlatParsedBitstream<'_>,
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    mut emit: impl FnMut(&PipelineFrame) -> Result<()>,
    spawner: &inflight::FinishSpawner<'_, '_>,
    ring: &mut inflight::InflightRing,
    decode_scratch_eight: &mut inter::InterDecodeScratch<u8>,
    decode_scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<Vec<PipelineFrame>> {
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
    let ([_td_envelope, sequence_envelope, key_envelope], leading_frame_unit_len) =
        require_leading_frame_unit(leading_obus)?;

    let mut sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;
    let mut film_grain_slots = FilmGrainSlots::new();
    let leading_prefix = leading_prefix_obus(leading_obus)?;
    film_grain_slots.update_from_obus(leading_prefix)?;
    let mut output_effect_state = OutputEffectState::new();
    output_effect_state.observe_prefix(leading_prefix, &sequence)?;

    ensure_runtime_storage_bit_depth(&sequence, sequence_envelope.offset)?;
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
                let activation =
                    inter::parse_inter_frame_activation(key_envelope, &sequence, &state, true)?;
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
                )?
            }
            BitDepthIdc::Ten => {
                let (store, meta) = reference.build_store_ten(&frames)?;
                let state = inter::InterReferenceState::from_metadata(store, meta);
                let activation =
                    inter::parse_inter_frame_activation(key_envelope, &sequence, &state, true)?;
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
    let key_user_qm =
        output_effect_state.prepare_frame(key_envelope, &key_core, &sequence, true)?;
    let key_display_grain = film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "decode runtime requires one selected key frame candidate",
        )
    })?;
    output_effect_state.observe_suffix(frame_suffix_obus(stream, key_candidate)?)?;
    let key_output_effects = output_effect_state.finish_frame();
    let mut retained_frame_bytes = 0;
    let mut output_frame_bytes = 0;
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
                    spawner,
                    ring,
                    0,
                    decode_scratch_eight,
                )?;
                let rate = key_output_effects.frame_rate(frame_rate);
                PipelineFrame {
                    frame,
                    display_grain: key_display_grain,
                    output_effects: key_output_effects,
                    frame_cdfs: walk.frame_cdfs,
                    motion_field: inter::MotionFieldHandle::settled(walk.motion_field),
                    ccso_params,
                    ccso_grid: walk.ccso_grid,
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
                    spawner,
                    ring,
                    0,
                    decode_scratch_ten,
                )?;
                let rate = key_output_effects.frame_rate(frame_rate);
                PipelineFrame {
                    frame,
                    display_grain: key_display_grain,
                    output_effects: key_output_effects,
                    frame_cdfs: walk.frame_cdfs,
                    motion_field: inter::MotionFieldHandle::settled(walk.motion_field),
                    ccso_params,
                    ccso_grid: walk.ccso_grid,
                    frame_rate_numerator: rate.numerator,
                    frame_rate_denominator: rate.denominator,
                }
            }
        }
    } else {
        decode_key_frame_with_effects(
            decode_scratch_eight,
            decode_scratch_ten,
            spawner,
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
        Arc::clone(&key_frame.frame_cdfs),
        key_frame.ccso_params.clone(),
        key_frame.ccso_grid.clone(),
        key_frame.motion_field.clone(),
        key_envelope.header.embedded_layer_id,
    )?;
    let key_saved_grain = key_frame.display_grain.clone();
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
    output_frame_bytes = charge_emitted_outputs(
        options,
        &frames,
        &scheduler,
        &mut emission_queue,
        &evicted,
        output_frame_bytes,
        retain_decoded_frames,
        &mut emit,
    )?;
    reference.update(0, &key_update);
    reference
        .save_grain_for_refreshed_slots(key_update.refresh_frame_flags, key_saved_grain.as_ref());
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
        output_frame_bytes = charge_emitted_outputs(
            options,
            &frames,
            &scheduler,
            &mut emission_queue,
            &emitted,
            output_frame_bytes,
            retain_decoded_frames,
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
        emission_queue.flush(
            options,
            &frames,
            output_frame_bytes,
            retain_decoded_frames,
            &mut emit,
        )?;
        return if retain_decoded_frames {
            select_output_frames(frames, scheduler.emitted)
        } else {
            Ok(Vec::new())
        };
    }

    let mut decoding_initial_tu = true;
    let mut pending: Option<frame_pipeline::PendingWalk> = None;
    let mut shared_sequence = None;
    for next_candidate in candidates {
        match next_candidate.obu_type() {
            ObuType::LeadingSef | ObuType::RegularSef => {
                frame_pipeline::flush_pending(
                    &mut pending,
                    spawner,
                    decode_scratch_eight,
                    decode_scratch_ten,
                )?;
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
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    output_frame_bytes,
                    retain_decoded_frames,
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
                    reference.save_grain_for_slot(slot, display_grain.clone())?;
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
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &emitted,
                    output_frame_bytes,
                    retain_decoded_frames,
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
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    output_frame_bytes,
                    retain_decoded_frames,
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
                        output_frame_bytes = charge_emitted_outputs(
                            options,
                            &frames,
                            &scheduler,
                            &mut emission_queue,
                            &emitted,
                            output_frame_bytes,
                            retain_decoded_frames,
                            &mut emit,
                        )?;
                        if output_frame_limit_reached(options, scheduler.emitted.len()) {
                            break;
                        }
                    }
                }
                let inter_frame_timer = crate::timing::start();
                let frame_index = frames.len();
                let (inter_slot, inter_core, frame_cdfs, ccso_grid, motion_field) = match sequence
                    .general
                    .bit_depth_idc
                {
                    BitDepthIdc::Eight => {
                        let (store, meta) = reference.build_store_eight(&frames)?;
                        let inter_state = inter::InterReferenceState::from_metadata(store, meta);
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                            &output_effect_state,
                            first_picture_in_tu,
                        )?;
                        let user_qm = output_effect_state.prepare_frame(
                            inter_envelope,
                            &inter_core,
                            &sequence,
                            first_picture_in_tu,
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
                        ring.reserve(decode_scratch_eight, decode_scratch_ten);
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if split_walk(spawner, next_candidate.obu_type(), &inter_core) {
                            let records = decode_scratch_eight.take_frame_filter_records();
                            let quantizer =
                                crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            let deferred = frame_pipeline::parse_beside_pending(
                                move || {
                                    let _scopes = quantizer.install_frame();
                                    inter::parse_inter_frame(
                                        records,
                                        plan,
                                        next_candidate,
                                        bytes,
                                        inter_envelope,
                                        inter_core,
                                        shared,
                                        options,
                                        inter_state,
                                        BitDepth::Eight,
                                    )
                                },
                                pending.take(),
                                spawner,
                                decode_scratch_eight,
                                decode_scratch_ten,
                            )??;
                            let (slot, finish) = inflight::reserve_pending_slot(
                                deferred.info,
                                inflight::PipelineFrameSlot::Eight,
                                ring,
                                frame_index,
                            )?;
                            let products = (
                                slot,
                                Arc::clone(&deferred.core),
                                Arc::clone(&deferred.frame_cdfs),
                                deferred.ccso_grid.clone(),
                                deferred.motion.clone(),
                            );
                            pending = Some(frame_pipeline::PendingWalk::Eight(deferred, finish));
                            products
                        } else {
                            frame_pipeline::flush_pending(
                                &mut pending,
                                spawner,
                                decode_scratch_eight,
                                decode_scratch_ten,
                            )?;
                            let walk = frame_engine::walk_frame(
                                decode_scratch_eight,
                                plan,
                                next_candidate,
                                bytes,
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &frame_engine::FrameSetup::Inter(&inter_state),
                                BitDepth::Eight,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Eight,
                                spawner,
                                ring,
                                frame_index,
                                decode_scratch_eight,
                            )?;
                            (
                                slot,
                                inter_core,
                                walk.frame_cdfs,
                                walk.ccso_grid,
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
                        )?;
                        let user_qm = output_effect_state.prepare_frame(
                            inter_envelope,
                            &inter_core,
                            &sequence,
                            first_picture_in_tu,
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
                        ring.reserve(decode_scratch_eight, decode_scratch_ten);
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if split_walk(spawner, next_candidate.obu_type(), &inter_core) {
                            let records = decode_scratch_ten.take_frame_filter_records();
                            let quantizer =
                                crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
                            let shared =
                                frame_pipeline::shared_sequence(&mut shared_sequence, &sequence);
                            let deferred = frame_pipeline::parse_beside_pending(
                                move || {
                                    let _scopes = quantizer.install_frame();
                                    inter::parse_inter_frame(
                                        records,
                                        plan,
                                        next_candidate,
                                        bytes,
                                        inter_envelope,
                                        inter_core,
                                        shared,
                                        options,
                                        inter_state,
                                        BitDepth::Ten,
                                    )
                                },
                                pending.take(),
                                spawner,
                                decode_scratch_eight,
                                decode_scratch_ten,
                            )??;
                            let (slot, finish) = inflight::reserve_pending_slot(
                                deferred.info,
                                inflight::PipelineFrameSlot::Ten,
                                ring,
                                frame_index,
                            )?;
                            let products = (
                                slot,
                                Arc::clone(&deferred.core),
                                Arc::clone(&deferred.frame_cdfs),
                                deferred.ccso_grid.clone(),
                                deferred.motion.clone(),
                            );
                            pending = Some(frame_pipeline::PendingWalk::Ten(deferred, finish));
                            products
                        } else {
                            frame_pipeline::flush_pending(
                                &mut pending,
                                spawner,
                                decode_scratch_eight,
                                decode_scratch_ten,
                            )?;
                            let walk = frame_engine::walk_frame(
                                decode_scratch_ten,
                                plan,
                                next_candidate,
                                bytes,
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &frame_engine::FrameSetup::Inter(&inter_state),
                                BitDepth::Ten,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Ten,
                                spawner,
                                ring,
                                frame_index,
                                decode_scratch_ten,
                            )?;
                            (
                                slot,
                                inter_core,
                                walk.frame_cdfs,
                                walk.ccso_grid,
                                inter::MotionFieldHandle::settled(walk.motion_field),
                            )
                        }
                    }
                };
                crate::timing::report("inter_frame_decode", inter_frame_timer);
                let inter_display_grain =
                    film_grain_slots.active_for_core(&inter_core, inter_envelope.offset)?;
                output_effect_state.observe_suffix(frame_suffix_obus(stream, next_candidate)?)?;
                let inter_output_effects = output_effect_state.finish_frame();
                let inter_frame_rate = inter_output_effects.frame_rate(frame_rate);
                let inter_update = frame_ref_update_from_core(
                    &inter_core,
                    inter_envelope.offset,
                    frame_cdfs,
                    inter_core.ccso_params.clone(),
                    ccso_grid.clone(),
                    motion_field,
                    inter_envelope.header.embedded_layer_id,
                )?;
                let inter_frame = PipelineFrame {
                    frame: inter_slot,
                    display_grain: inter_display_grain,
                    output_effects: inter_output_effects,
                    frame_cdfs: Arc::clone(&inter_update.frame_cdfs),
                    motion_field: inter_update.motion_field.clone(),
                    ccso_params: inter_core.ccso_params.clone(),
                    ccso_grid,
                    frame_rate_numerator: inter_frame_rate.numerator,
                    frame_rate_denominator: inter_frame_rate.denominator,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                let inter_saved_grain = inter_frame.display_grain.clone();
                frames.push(Some(inter_frame));
                retained_frame_bytes = next_retained_frame_bytes;
                let inter_hint = inter_update.order_hint;
                let inter_implicit = inter_core.implicit_output_frame == Some(true);
                let inter_immediate = inter_core.immediate_output_frame == Some(true);
                let inter_key_or_switch =
                    inter_core.is_key_frame || inter_core.frame_type == Some(FrameType::Switch);
                let evicted = scheduler.refresh(
                    inter_update.refresh_frame_flags,
                    frame_index,
                    inter_hint,
                    inter_implicit,
                    inter_key_or_switch,
                );
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &evicted,
                    output_frame_bytes,
                    retain_decoded_frames,
                    &mut emit,
                )?;
                reference.update(frame_index, &inter_update);
                reference.save_grain_for_refreshed_slots(
                    inter_update.refresh_frame_flags,
                    inter_saved_grain.as_ref(),
                );
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
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &emitted,
                        output_frame_bytes,
                        retain_decoded_frames,
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
                frame_pipeline::flush_pending(
                    &mut pending,
                    spawner,
                    decode_scratch_eight,
                    decode_scratch_ten,
                )?;
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
                ensure_runtime_storage_bit_depth(&key_sequence, key_sequence_offset)?;
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
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        output_frame_bytes,
                        retain_decoded_frames,
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
                )?;
                let key_display_grain =
                    film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;

                if starts_new_sequence {
                    let key_reference =
                        reference_buffer::RuntimeReferenceBuffer::new(key_num_ref_frames)?;
                    let flushed = scheduler.start_new_sequence(key_num_ref_frames);
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        output_frame_bytes,
                        retain_decoded_frames,
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
                let key_frame_timer = crate::timing::start();
                let frame_index = frames.len();
                let key_frame = decode_key_frame_with_effects(
                    decode_scratch_eight,
                    decode_scratch_ten,
                    spawner,
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
                crate::timing::report("key_frame_decode", key_frame_timer);
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &key_frame,
                )?;
                let key_update = frame_ref_update_from_core(
                    &key_core,
                    key_envelope.offset,
                    Arc::clone(&key_frame.frame_cdfs),
                    key_frame.ccso_params.clone(),
                    key_frame.ccso_grid.clone(),
                    key_frame.motion_field.clone(),
                    key_envelope.header.embedded_layer_id,
                )?;
                let key_saved_grain = key_frame.display_grain.clone();
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
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &mut emission_queue,
                    &evicted,
                    output_frame_bytes,
                    retain_decoded_frames,
                    &mut emit,
                )?;
                reference.update(frame_index, &key_update);
                reference.save_grain_for_refreshed_slots(
                    key_update.refresh_frame_flags,
                    key_saved_grain.as_ref(),
                );
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
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &mut emission_queue,
                        &emitted,
                        output_frame_bytes,
                        retain_decoded_frames,
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

    frame_pipeline::flush_pending(
        &mut pending,
        spawner,
        decode_scratch_eight,
        decode_scratch_ten,
    )?;
    if !output_frame_limit_reached(options, scheduler.emitted.len()) {
        let flushed = scheduler.flush_all();
        output_frame_bytes = charge_emitted_outputs(
            options,
            &frames,
            &scheduler,
            &mut emission_queue,
            &flushed,
            output_frame_bytes,
            retain_decoded_frames,
            &mut emit,
        )?;
        output_frame_bytes = emission_queue.flush(
            options,
            &frames,
            output_frame_bytes,
            retain_decoded_frames,
            &mut emit,
        )?;
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
        let _ = output_frame_bytes;
    } else {
        emission_queue.flush(
            options,
            &frames,
            output_frame_bytes,
            retain_decoded_frames,
            &mut emit,
        )?;
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
    envelope: ObuEnvelope<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<&Arc<FrameCdfSubset>>,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
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
        let group_envelope = if group_index == 0 {
            envelope
        } else {
            planned_envelope(bytes, group_candidate)?
        };
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
    envelope: ObuEnvelope<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        envelope,
        sequence,
        core,
        options,
        TileFactsKind::Intra,
        None,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_inter_tile_plan<'payload>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &'payload [u8],
    envelope: ObuEnvelope<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    initial_cdfs: &Arc<FrameCdfSubset>,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        envelope,
        sequence,
        core,
        options,
        TileFactsKind::Inter,
        Some(initial_cdfs),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn decode_tile_boundary_error(error: FrameCandidateTileBoundaryError) -> DecodeError {
    match error {
        FrameCandidateTileBoundaryError::Limit(source) => DecodeError::Limit { source },
        FrameCandidateTileBoundaryError::Malformed(malformed) => unsupported(
            malformed_tile_boundary_reason(malformed),
            None,
            "decode runtime could not derive a source-backed tile payload boundary",
        ),
        FrameCandidateTileBoundaryError::MissingFact { .. } => unsupported(
            "missing_tile_fact",
            None,
            "decode runtime requires complete parser-derived tile facts",
        ),
        FrameCandidateTileBoundaryError::Unsupported { .. }
        | FrameCandidateTileBoundaryError::Boundary(_) => unsupported(
            "unsupported_tile_boundary",
            None,
            "decode runtime requires source-backed tile work units",
        ),
    }
}

fn malformed_tile_boundary_reason(
    malformed: crate::bitstream::tile_payload::FrameCandidateTileMalformed,
) -> &'static str {
    match malformed {
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::CandidateNotInPlan => {
            "candidate_not_in_plan"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::PlanSourceKindMismatch { .. } => {
            "plan_source_kind_mismatch"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field } => {
            match field {
                "payload_source" => "payload_source_mismatch",
                "offset" => "candidate_offset_mismatch",
                "size" => "candidate_size_mismatch",
                "header" => "candidate_header_mismatch",
                "payload_len" => "candidate_payload_len_mismatch",
                "payload" => "candidate_payload_mismatch",
                "input_len_bytes" => "input_len_mismatch",
                "ivf_frame" => "ivf_frame_mismatch",
                _ => "candidate_envelope_mismatch",
            }
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::ObuSizeSmallerThanHeader { .. } => {
            "obu_size_smaller_than_header"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::SourceRangeOutOfBounds { .. } => {
            "source_range_out_of_bounds"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::TileGroupStructureIncomplete => {
            "tile_group_structure_incomplete"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::TileGroupStructureInvalid => {
            "tile_group_structure_invalid"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid => {
            "tile_group_payload_range_invalid"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::TileGroupRangeInvalid { .. } => {
            "tile_group_range_invalid"
        }
        crate::bitstream::tile_payload::FrameCandidateTileMalformed::TileGroupPositionMismatch { .. } => {
            "tile_group_position_mismatch"
        }
    }
}
pub(crate) fn ensure_runtime_limits(
    limits: crate::DecodeLimits,
    width: u32,
    height: u32,
    tile_payload_bytes: u64,
    bit_depth: BitDepth,
    chroma_format: ChromaFormatIdc,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
    limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
    let budget = decoded_frame_storage_budget(
        FrameSize::new(width, height),
        chroma_format,
        bytes_per_sample(bit_depth),
    )?;
    limits.ensure(DecodeLimitName::MaxLumaSamplesPerFrame, budget.luma_samples)?;
    limits.ensure(DecodeLimitName::MaxDecodedFrameBytes, budget.decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxTileCount, 1)?;
    limits.ensure(DecodeLimitName::MaxTilePayloadBytes, tile_payload_bytes)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, budget.luma_samples)?;
    limits.ensure_allocation_len(
        DecodeLimitName::MaxDecodedFrameBytes,
        budget.chroma_samples_per_plane,
    )?;
    Ok(())
}

pub(crate) fn unsupported_with_spec(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            spec_section,
            message,
            byte_offset,
        )),
    }
}

pub(crate) fn unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
) -> DecodeError {
    unsupported_with_spec(reason, byte_offset, message, SPEC_SECTION)
}

pub(crate) fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
) -> DecodeError {
    unsupported(reason, Some(byte_offset), message)
}

pub(crate) fn unsupported_feature_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    unsupported_with_spec(reason, Some(byte_offset), message, spec_section)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
