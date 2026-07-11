// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode pipeline orchestration for the supported decode runtime.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameSize, FrameType, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader};
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
#[cfg(test)]
use splot_core::stream::ParsedBitstream;
use splot_core::stream::parse_bitstream_partial;
use splot_core::symbol::SymbolDecoder;
use splot_core::types::ObuType;
use splot_recon::BitDepth;
#[cfg(test)]
use splot_recon::DecodedFrame;

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
mod output_schedule;
mod stream_schedule;

#[cfg(test)]
pub(crate) use frame_lifecycle::incomplete_intra_header_error;
use frame_lifecycle::*;
pub(crate) use frame_lifecycle::{
    ActiveFilmGrain, PipelineDecodedFrame, PipelineFrame, PipelineFrameRate, deblock_quant_deltas,
    effective_allow_screen_content_tools, ensure_runtime_storage_bit_depth,
    frame_ref_update_from_core, parse_frame_core, parse_sequence,
};
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
pub(crate) fn decode_frames_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<PipelineFrame>> {
    decode_frames_from_plan_with_ivf_preflight(bytes, options, plan, |_| Ok(()))
}
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
    let (frame, frame_cdfs, ccso_params, ccso_grid, motion_field) =
        match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let (frame, core, frame_cdfs, ccso_grid, motion_field) =
                    frame_engine::decode_frame::<u8>(
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
                (
                    PipelineDecodedFrame::Eight(frame),
                    frame_cdfs,
                    core.ccso_params,
                    ccso_grid,
                    motion_field,
                )
            }
            BitDepthIdc::Ten => {
                let (frame, core, frame_cdfs, ccso_grid, motion_field) =
                    frame_engine::decode_frame::<u16>(
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
                (
                    PipelineDecodedFrame::Ten(frame),
                    frame_cdfs,
                    core.ccso_params,
                    ccso_grid,
                    motion_field,
                )
            }
        };
    Ok(PipelineFrame {
        frame,
        display_grain,
        frame_cdfs,
        motion_field,
        ccso_params,
        ccso_grid,
        frame_rate_numerator: frame_rate.numerator,
        frame_rate_denominator: frame_rate.denominator,
    })
}

pub(crate) fn decode_frames_from_plan_with_ivf_preflight(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
) -> Result<Vec<PipelineFrame>> {
    ensure_multiframe_plan_shape(plan)?;
    let runtime_parse_timer = crate::timing::start();
    let parsed = parse_bitstream_partial(bytes);
    crate::timing::report("runtime_reparse", runtime_parse_timer);
    let stream = require_runtime_stream(&parsed)?;
    preflight(stream.ivf_header())?;
    let frame_rate = stream.frame_rate();

    let leading_obus = stream.leading_obus()?;
    let ([_td_envelope, sequence_envelope, key_envelope], leading_frame_unit_len) =
        require_leading_frame_unit(leading_obus)?;

    let sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;
    let mut film_grain_slots = FilmGrainSlots::new();
    film_grain_slots.update_from_obus(leading_film_grain_obus(leading_obus)?)?;

    let key_core = parse_frame_core(key_envelope, &sequence)?;
    ensure_intra_header_complete(&key_core, key_envelope.offset)?;
    let key_display_grain = film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "decode runtime requires one selected key frame candidate",
        )
    })?;
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
    let mut disp_hints = DispOrderHints::new(sequence_inter.order_hint_bits, num_ref_frames);

    let mut frames = Vec::new();
    let mut scheduler = OutputScheduler::new(num_ref_frames);
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
    let key_frame = decode_key_frame(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        &sequence,
        frame_rate,
        key_display_grain,
    )?;
    retained_frame_bytes =
        ensure_retained_frame_byte_limits(options.limits(), retained_frame_bytes, &key_frame)?;
    frames.push(key_frame);
    let key_hint = disp_hints.extend(&key_core)?;
    let key_update = frame_ref_update_from_core(
        &key_core,
        key_envelope.offset,
        frames[0].frame_cdfs.clone(),
        frames[0].ccso_params.clone(),
        frames[0].ccso_grid.clone(),
        frames[0].motion_field.clone(),
        key_hint,
    )?;
    let key_implicit = key_core.implicit_output_frame == Some(true);
    let key_immediate = key_core.immediate_output_frame == Some(true);
    let evicted = scheduler.refresh(
        key_update.refresh_frame_flags,
        0,
        key_hint,
        key_implicit,
        true,
    );
    output_frame_bytes =
        charge_emitted_outputs(options, &frames, &scheduler, &evicted, output_frame_bytes)?;
    reference.update(0, &key_update);
    disp_hints.refresh(
        key_update.refresh_frame_flags,
        key_hint,
        key_implicit || key_immediate,
        true,
    );
    if key_immediate && !scheduler.already_emitted(0) {
        let emitted = scheduler.on_immediate(0, key_hint);
        output_frame_bytes =
            charge_emitted_outputs(options, &frames, &scheduler, &emitted, output_frame_bytes)?;
    }
    if output_frame_limit_reached(options, scheduler.emitted.len()) {
        return select_output_frames(frames, scheduler.emitted);
    }

    for next_candidate in candidates {
        match next_candidate.obu_type() {
            ObuType::RegularTileGroup | ObuType::RegularTip => {
                let (inter_film_grain_obus, inter_envelope) = match stream {
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
                film_grain_slots.update_from_obus(inter_film_grain_obus)?;
                let inter_frame_timer = crate::timing::start();
                let (inter_frame, inter_core, frame_cdfs, ccso_grid, motion_field) = match sequence
                    .general
                    .bit_depth_idc
                {
                    BitDepthIdc::Eight => {
                        let (store, meta) = reference.build_store_eight(&frames)?;
                        let inter_state = inter::InterReferenceState::from_metadata(&store, meta);
                        let inter_core = inter::parse_validated_inter_frame_core(
                            inter_envelope,
                            &sequence,
                            &inter_state,
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
                        let (frame, inter_core, frame_cdfs, ccso_grid, motion_field) =
                            frame_engine::decode_frame(
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
                        (
                            PipelineDecodedFrame::Eight(frame),
                            inter_core,
                            frame_cdfs,
                            ccso_grid,
                            motion_field,
                        )
                    }
                    BitDepthIdc::Ten => {
                        let (store, meta) = reference.build_store_ten(&frames)?;
                        let inter_state = inter::InterReferenceState::from_metadata(&store, meta);
                        let inter_core = inter::parse_validated_inter_frame_core(
                            inter_envelope,
                            &sequence,
                            &inter_state,
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
                        let (frame, inter_core, frame_cdfs, ccso_grid, motion_field) =
                            frame_engine::decode_frame(
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
                        (
                            PipelineDecodedFrame::Ten(frame),
                            inter_core,
                            frame_cdfs,
                            ccso_grid,
                            motion_field,
                        )
                    }
                };
                crate::timing::report("inter_frame_decode", inter_frame_timer);
                let inter_display_grain =
                    film_grain_slots.active_for_core(&inter_core, inter_envelope.offset)?;
                let inter_frame = PipelineFrame {
                    frame: inter_frame,
                    display_grain: inter_display_grain,
                    frame_cdfs,
                    motion_field,
                    ccso_params: inter_core.ccso_params.clone(),
                    ccso_grid,
                    frame_rate_numerator: frame_rate.numerator,
                    frame_rate_denominator: frame_rate.denominator,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                let frame_index = frames.len();
                frames.push(inter_frame);
                retained_frame_bytes = next_retained_frame_bytes;
                let inter_hint = disp_hints.extend(&inter_core)?;
                let inter_update = frame_ref_update_from_core(
                    &inter_core,
                    inter_envelope.offset,
                    frames[frame_index].frame_cdfs.clone(),
                    frames[frame_index].ccso_params.clone(),
                    frames[frame_index].ccso_grid.clone(),
                    frames[frame_index].motion_field.clone(),
                    inter_hint,
                )?;
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
                    &evicted,
                    output_frame_bytes,
                )?;
                reference.update(frame_index, &inter_update);
                disp_hints.refresh(
                    inter_update.refresh_frame_flags,
                    inter_hint,
                    inter_implicit || inter_immediate,
                    inter_key_or_switch,
                );
                if inter_immediate && !scheduler.already_emitted(frame_index) {
                    let emitted = scheduler.on_immediate(frame_index, inter_hint);
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &emitted,
                        output_frame_bytes,
                    )?;
                }
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
            }
            ObuType::ClosedLoopKey => {
                let (key_sequence_envelope, key_film_grain_obus, key_envelope) = match stream {
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
                let key_sequence = parse_sequence(key_sequence_envelope)?;
                validate_sequence(&key_sequence, key_sequence_envelope.offset)?;
                film_grain_slots.update_from_obus(key_film_grain_obus)?;
                ensure_repeated_key_sequence_compatible(
                    &sequence,
                    &key_sequence,
                    key_sequence_envelope.offset,
                )?;
                let key_core = parse_frame_core(key_envelope, &key_sequence)?;
                ensure_intra_header_complete(&key_core, key_envelope.offset)?;
                let key_display_grain =
                    film_grain_slots.active_for_core(&key_core, key_envelope.offset)?;
                ensure_retained_frame_byte_limits_for_core(
                    options.limits(),
                    retained_frame_bytes,
                    &key_core,
                    &key_sequence,
                    key_envelope.offset,
                )?;
                let key_frame_timer = crate::timing::start();
                let key_frame = decode_key_frame(
                    bytes,
                    options,
                    plan,
                    next_candidate,
                    key_envelope,
                    &key_sequence,
                    frame_rate,
                    key_display_grain,
                )?;
                crate::timing::report("key_frame_decode", key_frame_timer);
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &key_frame,
                )?;
                let frame_index = frames.len();
                frames.push(key_frame);
                retained_frame_bytes = next_retained_frame_bytes;
                let key_hint = disp_hints.extend(&key_core)?;
                let key_update = frame_ref_update_from_core(
                    &key_core,
                    key_envelope.offset,
                    frames[frame_index].frame_cdfs.clone(),
                    frames[frame_index].ccso_params.clone(),
                    frames[frame_index].ccso_grid.clone(),
                    frames[frame_index].motion_field.clone(),
                    key_hint,
                )?;
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
                    &evicted,
                    output_frame_bytes,
                )?;
                reference.update(frame_index, &key_update);
                disp_hints.refresh(
                    key_update.refresh_frame_flags,
                    key_hint,
                    key_implicit || key_immediate,
                    true,
                );
                if key_immediate && !scheduler.already_emitted(frame_index) {
                    let emitted = scheduler.on_immediate(frame_index, key_hint);
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &emitted,
                        output_frame_bytes,
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

    if !output_frame_limit_reached(options, scheduler.emitted.len()) {
        let flushed = scheduler.flush_all();
        output_frame_bytes =
            charge_emitted_outputs(options, &frames, &scheduler, &flushed, output_frame_bytes)?;
        let _ = output_frame_bytes;
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
fn derive_tile_plan_with<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<FrameCdfSubset>,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'a>> {
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
    let mut input = FrameCandidateTileBoundaryInput::new(
        plan,
        candidate,
        bytes,
        envelope,
        TileGroupPositionFacts::new(true, true),
        facts,
        cdf,
        options.limits(),
    );
    if let Some(cdfs) = initial_cdfs {
        input = input.with_initial_cdfs(cdfs);
    }
    crate::bitstream::tile_payload::plan_derived_tile_payload_boundary(&input)
        .map_err(decode_tile_boundary_error)
}

pub(crate) fn derive_tile_plan<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'a>> {
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
pub(crate) fn derive_inter_tile_plan<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
    initial_cdfs: FrameCdfSubset,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'a>> {
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
    limits.ensure(DecodeLimitName::MaxOutputBytes, budget.decoded_bytes)?;
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
