// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode pipeline orchestration for the currently supported decoder tiers.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::film_grain::{FilmGrainModel, MAX_FILM_GRAIN, parse_film_grain};
use splot_core::headers::frame::{
    CoreSeqQuantView, FilmGrainConfig, FrameHeaderCore, FrameHeaderParseInput,
    FrameHeaderParseMode, FrameHeaderParseStatus, FrameReferenceStateView, FrameSize, FrameType,
    TxMode, parse_frame_header_core,
};
use splot_core::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, SequenceHeader, parse_sequence_header,
};
use splot_core::ivf::{IvfHeader, IvfWarning};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream, parse_bitstream_partial};
use splot_core::symbol::SymbolDecoder;
use splot_core::types::ObuType;
use splot_recon::{BitDepth, DecodedFrame, DecodedFrameHashInput};

use crate::bitstream::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    GeneralIntraBlockModeError, GeneralIntraResidualError, TileGroupPositionFacts,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::filters::deblock;
use crate::prediction::inter;
use crate::reference::buffer as reference_buffer;
use crate::support::capability::missing_capability_message;
use crate::support::pipeline_limits::{checked_add, decoded_frame_storage_budget};
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

const SPEC_SECTION: &str = "7.1";

pub(crate) fn effective_allow_screen_content_tools(core: &FrameHeaderCore) -> bool {
    core.allow_screen_content_tools
        .or_else(|| {
            core.inter
                .as_ref()
                .and_then(|inter| inter.allow_screen_content_tools)
        })
        .unwrap_or(false)
}

pub(crate) const GENERAL_INTRA_PARTITION_SPEC_SECTION: &str = "5.20.3.1";
pub(crate) const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
pub(crate) const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";
pub(crate) enum PipelineDecodedFrame {
    Eight(DecodedFrame<u8>),
    Ten(DecodedFrame<u16>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveFilmGrain {
    pub(crate) model: FilmGrainModel,
    pub(crate) grain_seed: u16,
}

pub(crate) fn deblock_quant_deltas(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> deblock::DeblockQuantDeltas {
    let (Some(tq), Some(quant)) = (
        sequence.transform_quant_entropy.as_ref(),
        core.quantization_params,
    ) else {
        return deblock::DeblockQuantDeltas::ZERO;
    };
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
    deblock::DeblockQuantDeltas::from_frame_quant(quant, seq_quant.base_uv_ac_delta_q)
}

pub(crate) struct PipelineFrame {
    pub(crate) frame: PipelineDecodedFrame,
    pub(crate) display_grain: Option<ActiveFilmGrain>,
    pub(crate) frame_cdfs: FrameCdfSubset,
    pub(crate) motion_field: inter::TemporalMotionField,
    pub(crate) ccso_params: Option<splot_core::headers::frame::CcsoParams>,
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) frame_rate_numerator: u32,
    pub(crate) frame_rate_denominator: u32,
}

impl PipelineFrame {
    pub(crate) fn frame_eight(&self) -> Result<&DecodedFrame<u8>> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => Ok(frame),
            PipelineDecodedFrame::Ten(_) => Err(unsupported(
                "unsupported_10bit_reference_retention",
                None,
                missing_capability_message!("reference.retention bit_depth=10"),
            )),
        }
    }
    pub(crate) fn frame_ten(&self) -> Result<&DecodedFrame<u16>> {
        match &self.frame {
            PipelineDecodedFrame::Ten(frame) => Ok(frame),
            PipelineDecodedFrame::Eight(_) => Err(unsupported(
                "unsupported_8bit_reference_for_10bit_decode",
                None,
                "inter decode pipeline requires reference frames to match the active 10-bit storage",
            )),
        }
    }
    pub(crate) fn byte_len(&self) -> Result<usize> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => Ok(DecodedFrameHashInput::new(frame).byte_len()?),
            PipelineDecodedFrame::Ten(frame) => Ok(DecodedFrameHashInput::new(frame).byte_len()?),
        }
    }
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(crate) fn frame(&self) -> &DecodedFrame<u8> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => frame,
            PipelineDecodedFrame::Ten(_) => panic!("frame() called on a 10-bit PipelineFrame"),
        }
    }
}

struct FilmGrainSlots {
    models: [Option<FilmGrainModel>; MAX_FILM_GRAIN],
}

impl FilmGrainSlots {
    fn new() -> Self {
        Self {
            models: std::array::from_fn(|_| None),
        }
    }

    fn update_from_obus(&mut self, obus: &[ObuEnvelope<'_>]) -> Result<()> {
        for envelope in obus {
            self.update_from_obu(*envelope)?;
        }
        Ok(())
    }

    fn update_from_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let film_grain = parse_film_grain(&mut reader).map_err(|_| {
            unsupported_feature_at(
                "film_grain_obu_parse",
                envelope.offset,
                "runtime film-grain synthesis requires a fully parseable film_grain_obu",
                "5.14",
            )
        })?;
        for update in film_grain.models {
            let slot = usize::from(update.slot);
            let Some(target) = self.models.get_mut(slot) else {
                return Err(unsupported_feature_at(
                    "film_grain_slot_out_of_range",
                    envelope.offset,
                    "film_grain_obu updated a model slot outside MAX_FILM_GRAIN",
                    "5.14",
                ));
            };
            *target = Some(update.model);
        }
        Ok(())
    }

    fn active_for_core(
        &self,
        core: &FrameHeaderCore,
        frame_offset: ByteOffset,
    ) -> Result<Option<ActiveFilmGrain>> {
        let Some(config) = film_grain_config_for_core(core, frame_offset)? else {
            return Ok(None);
        };
        if !config.apply_grain {
            return Ok(None);
        }
        let fgm_id = config.fgm_id.ok_or_else(|| {
            unsupported_feature_at(
                "film_grain_config_missing_fgm_id",
                frame_offset,
                "apply_grain frames must carry fgm_id for load_grain_model",
                "5.18.10.1",
            )
        })?;
        let grain_seed = config.grain_seed.ok_or_else(|| {
            unsupported_feature_at(
                "film_grain_config_missing_seed",
                frame_offset,
                "apply_grain frames must carry grain_seed for film-grain synthesis",
                "5.18.10.1",
            )
        })?;
        let model = self
            .models
            .get(usize::from(fgm_id))
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| {
                unsupported_feature_at(
                    "film_grain_model_unavailable",
                    frame_offset,
                    "apply_grain references an fgm_id whose model slot is unavailable",
                    "6.17.10.1",
                )
            })?;
        Ok(Some(ActiveFilmGrain { model, grain_seed }))
    }
}

fn film_grain_config_for_core(
    core: &FrameHeaderCore,
    frame_offset: ByteOffset,
) -> Result<Option<FilmGrainConfig>> {
    if let Some(tail) = core.intra_tail.as_ref() {
        return Ok(Some(tail.film_grain));
    }
    if let Some(config) = core.sef_film_grain {
        return Ok(Some(config));
    }
    if core
        .inter_tail
        .as_ref()
        .is_some_and(|tail| tail.apply_grain)
    {
        return Err(unsupported_feature_at(
            "inter_film_grain_config_unmodeled",
            frame_offset,
            "inter header parsing does not yet preserve fgm_id and grain_seed for apply_grain",
            "5.18.10.1",
        ));
    }
    Ok(None)
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
            "minimal tier requires at least one decoded frame",
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
    header: IvfHeader,
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
                        header,
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
                        header,
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
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

pub(crate) fn decode_frames_from_plan_with_ivf_preflight(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(IvfHeader) -> Result<()>,
) -> Result<Vec<PipelineFrame>> {
    ensure_multiframe_plan_shape(plan)?;
    let runtime_parse_timer = crate::timing::start();
    let parsed = parse_bitstream_partial(bytes);
    crate::timing::report("runtime_reparse", runtime_parse_timer);
    let (ivf, header) = require_multiframe_ivf(&parsed)?;
    preflight(header)?;

    let first_ivf_frame = ivf.frames.first().ok_or_else(|| {
        unsupported(
            "missing_first_ivf_frame",
            None,
            "minimal tier requires at least one IVF frame",
        )
    })?;
    let leading_obus = first_ivf_frame.obus.as_slice();
    let ([_td_envelope, sequence_envelope, key_envelope], _) =
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
            "minimal tier requires one selected key frame candidate",
        )
    })?;
    ensure_runtime_storage_bit_depth(&sequence, sequence_envelope.offset)?;

    let sequence_inter = sequence.inter.as_ref().ok_or_else(|| {
        unsupported(
            "missing_sequence_inter_config",
            None,
            "minimal multi-frame decode requires the sequence inter config (NumRefFrames)",
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
        header,
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
            ObuType::RegularTileGroup => {
                let inter_envelope = following_inter_envelope(
                    ivf,
                    next_candidate,
                    &mut next_unvalidated_following_ivf_record,
                )?;
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
                                header,
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
                                header,
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
                    frame_rate_numerator: header.timebase_denominator,
                    frame_rate_denominator: header.timebase_numerator,
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
                let (key_sequence_envelope, key_film_grain_obus, key_envelope) =
                    following_key_frame_unit(
                        ivf,
                        next_candidate,
                        &mut next_unvalidated_following_ivf_record,
                    )?;
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
                    header,
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
                    "multiple_frames_unimplemented",
                    next_candidate.offset(),
                    missing_capability_message!("frame.sequence key_plus_inter"),
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

fn charge_emitted_outputs(
    options: &DecodeOptions,
    frames: &[PipelineFrame],
    scheduler: &OutputScheduler,
    newly: &[usize],
    mut output_frame_bytes: u64,
) -> Result<u64> {
    if newly.is_empty() {
        return Ok(output_frame_bytes);
    }
    let requested = options
        .output_frame_limit()
        .map_or(u64::MAX, std::num::NonZeroU64::get);
    let emitted_total = (scheduler.emitted.len() as u64).min(requested);
    ensure_output_frame_count_limit(options.limits(), emitted_total)?;
    let first_new = scheduler.emitted.len() - newly.len();
    for (offset, &frame_index) in newly.iter().enumerate() {
        if (first_new + offset) as u64 >= requested {
            break;
        }
        let frame = frames.get(frame_index).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "decode pipeline output ordering references a decoded frame that is unavailable",
            )
        })?;
        output_frame_bytes =
            ensure_output_frame_byte_limits(options.limits(), output_frame_bytes, frame)?;
    }
    Ok(output_frame_bytes)
}

fn output_frame_limit_reached(options: &DecodeOptions, output_frame_count: usize) -> bool {
    options
        .output_frame_limit()
        .is_some_and(|limit| output_frame_count as u64 >= limit.get())
}

fn frame_is_output(core: &FrameHeaderCore) -> bool {
    core.immediate_output_frame == Some(true) || core.implicit_output_frame == Some(true)
}

struct OutputScheduler {
    pending: Vec<Option<(usize, u32)>>,
    emitted: Vec<usize>,
}

impl OutputScheduler {
    fn new(num_slots: usize) -> Self {
        Self {
            pending: vec![None; num_slots],
            emitted: Vec::new(),
        }
    }

    fn emit(&mut self, frame_index: usize, newly: &mut Vec<usize>) {
        if !self.emitted.contains(&frame_index) {
            self.emitted.push(frame_index);
            newly.push(frame_index);
        }
        for slot in &mut self.pending {
            if slot.is_some_and(|(held, _)| held == frame_index) {
                *slot = None;
            }
        }
    }

    fn flush_lower_than(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        loop {
            let next = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held < ordering)
                .min_by_key(|(_, held)| *held)
                .copied();
            let Some((frame_index, _)) = next else {
                return;
            };
            self.emit(frame_index, newly);
        }
    }

    fn output_successive(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        let mut target = ordering.saturating_add(1);
        loop {
            let matches: Vec<usize> = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held == target)
                .map(|(frame_index, _)| *frame_index)
                .collect();
            if matches.is_empty() {
                return;
            }
            for frame_index in matches {
                self.emit(frame_index, newly);
            }
            target = target.saturating_add(1);
        }
    }

    fn on_immediate(&mut self, frame_index: usize, ordering: u32) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(ordering, &mut newly);
        self.emit(frame_index, &mut newly);
        self.output_successive(ordering, &mut newly);
        newly
    }

    fn refresh(
        &mut self,
        refresh_frame_flags: u32,
        frame_index: usize,
        ordering: u32,
        implicit: bool,
        is_key_or_switch: bool,
    ) -> Vec<usize> {
        let mut newly = Vec::new();
        let mut first = true;
        for slot in 0..self.pending.len() {
            if (refresh_frame_flags >> slot) & 1 == 0 {
                continue;
            }
            if let Some((held_index, held_ordering)) = self.pending[slot] {
                self.flush_lower_than(held_ordering, &mut newly);
                self.emit(held_index, &mut newly);
                self.output_successive(held_ordering, &mut newly);
            }
            let valid = !is_key_or_switch || first;
            self.pending[slot] = (implicit && valid && !self.emitted.contains(&frame_index))
                .then_some((frame_index, ordering));
            first = false;
        }
        newly
    }

    fn already_emitted(&self, frame_index: usize) -> bool {
        self.emitted.contains(&frame_index)
    }

    fn flush_all(&mut self) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(u32::MAX, &mut newly);
        newly
    }
}

struct DispOrderHints {
    order_hint_bits: u32,
    slots: Vec<Option<(u32, bool)>>,
}

impl DispOrderHints {
    fn new(order_hint_bits: u8, num_slots: usize) -> Self {
        Self {
            order_hint_bits: u32::from(order_hint_bits),
            slots: vec![None; num_slots],
        }
    }

    fn extend(&self, core: &FrameHeaderCore) -> Result<u32> {
        let Some(lsb) = core.order_hint_lsb else {
            if core.implicit_output_frame == Some(true) {
                return Err(unsupported(
                    "implicit_output_requires_order_hints",
                    None,
                    "§ 7.21 implicit-output scheduling requires coded order hints",
                ));
            }
            return Ok(0);
        };
        let restricted_switch = core.frame_type == Some(FrameType::Switch)
            && core.restricted_prediction_switch == Some(true);
        if core.is_key_frame || restricted_switch {
            return Ok(lsb);
        }
        let max_disp = self
            .slots
            .iter()
            .flatten()
            .filter(|(_, showable)| *showable)
            .map(|(hint, _)| *hint)
            .max()
            .unwrap_or(0);
        let mut disp = lsb;
        let offset = i64::from(max_disp) - ((1i64 << self.order_hint_bits) >> 1) - i64::from(lsb);
        if offset >= 0 {
            let wraps = u32::try_from(offset).unwrap_or(u32::MAX) >> self.order_hint_bits;
            disp = disp.saturating_add((wraps + 1) << self.order_hint_bits);
        }
        if disp != lsb {
            return Err(unsupported(
                "order_hint_extension_beyond_parse_frontier",
                None,
                "the § 5.18.2 order-hint extension diverged from the coded LSB; parse-side hint consumers are still LSB-windowed",
            ));
        }
        Ok(disp)
    }

    fn refresh(
        &mut self,
        refresh_frame_flags: u32,
        hint: u32,
        showable: bool,
        is_key_or_switch: bool,
    ) {
        let mut first = true;
        for slot in 0..self.slots.len() {
            if (refresh_frame_flags >> slot) & 1 == 0 {
                continue;
            }
            self.slots[slot] = (!is_key_or_switch || first).then_some((hint, showable));
            first = false;
        }
    }
}

fn select_output_frames(
    frames: Vec<PipelineFrame>,
    output_frame_indices: Vec<usize>,
) -> Result<Vec<PipelineFrame>> {
    let mut frames = frames.into_iter().map(Some).collect::<Vec<_>>();
    let mut outputs = Vec::with_capacity(output_frame_indices.len());
    for index in output_frame_indices {
        let output = frames.get_mut(index).and_then(Option::take).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "decode pipeline output ordering references a decoded frame that is unavailable",
            )
        })?;
        outputs.push(output);
    }
    Ok(outputs)
}
pub(crate) fn following_inter_envelope<'a>(
    ivf: &'a ParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<ObuEnvelope<'a>> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let Some(position) = ivf_frame
            .obus
            .iter()
            .position(|envelope| envelope.offset == candidate.offset())
        else {
            continue;
        };
        require_following_ivf_obu_order_through(
            ivf,
            next_unvalidated_following_ivf_record,
            ivf_frame_index,
        )?;
        let inter_envelope = ivf_frame.obus[position];
        require_obu_type(
            inter_envelope,
            ObuType::RegularTileGroup,
            "missing_inter_regular_tile_group",
        )?;
        if is_leading_record_regular_after_key(ivf_frame_index, position, ivf_frame.obus.as_slice())
        {
            return Ok(inter_envelope);
        }
        let Some(td_envelope) = position
            .checked_sub(1)
            .and_then(|previous| ivf_frame.obus.get(previous))
            .copied()
        else {
            return Err(unsupported_at(
                "missing_inter_temporal_delimiter",
                candidate.offset(),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_obu_type(
            td_envelope,
            ObuType::TemporalDelimiter,
            "missing_inter_temporal_delimiter",
        )?;
        return Ok(inter_envelope);
    }
    Err(unsupported_at(
        "missing_inter_ivf_obu",
        candidate.offset(),
        "the planned inter candidate offset was not found in the parsed IVF payloads",
    ))
}

fn following_key_frame_unit<'a>(
    ivf: &'a ParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<(ObuEnvelope<'a>, &'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let Some(position) = ivf_frame
            .obus
            .iter()
            .position(|envelope| envelope.offset == candidate.offset())
        else {
            continue;
        };
        require_following_key_ivf_obu_order_through(
            ivf,
            next_unvalidated_following_ivf_record,
            ivf_frame_index,
        )?;
        let obus = ivf_frame.obus.as_slice();
        let ([_, sequence_envelope, key_envelope], _) = require_leading_frame_unit(obus)?;
        if key_envelope.offset != candidate.offset() {
            return Err(unsupported_at(
                "unexpected_key_obu_order",
                candidate.offset(),
                missing_capability_message!("frame.sequence repeated_key_frame_unit"),
            ));
        }
        let indices = minimal_frame_unit_indices(obus)?;
        if position != indices.frame {
            return Err(unsupported_at(
                "unexpected_key_obu_order",
                candidate.offset(),
                missing_capability_message!("frame.sequence repeated_key_frame_unit"),
            ));
        }
        return Ok((
            sequence_envelope,
            leading_film_grain_obus(obus)?,
            key_envelope,
        ));
    }
    Err(unsupported_at(
        "missing_key_ivf_obu",
        candidate.offset(),
        "the planned key candidate offset was not found in the parsed IVF payloads",
    ))
}

fn is_leading_record_regular_after_key(
    ivf_frame_index: usize,
    position: usize,
    obus: &[ObuEnvelope<'_>],
) -> bool {
    if ivf_frame_index != 0 {
        return false;
    }
    let Ok((_, frame_unit_len)) = require_leading_frame_unit(obus) else {
        return false;
    };
    position >= frame_unit_len
        && obus
            .iter()
            .skip(frame_unit_len)
            .all(|envelope| envelope.header.obu_type == ObuType::RegularTileGroup)
}

fn require_following_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for (ivf_frame_index, frame) in ivf
        .frames
        .iter()
        .enumerate()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_following_ivf_record_obu_order(frame.obus.as_slice(), ivf_frame_index)?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

fn require_following_key_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for frame in ivf
        .frames
        .iter()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_leading_ivf_obu_order(frame.obus.as_slice())?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

fn require_following_ivf_record_obu_order(
    obus: &[ObuEnvelope<'_>],
    ivf_frame_index: usize,
) -> Result<()> {
    if ivf_frame_index == 0 {
        require_leading_ivf_obu_order(obus)
    } else {
        require_inter_obu_order(obus)
    }
}

fn require_leading_ivf_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let (_, frame_unit_len) = require_leading_frame_unit(obus)?;
    for envelope in obus.iter().skip(frame_unit_len) {
        require_obu_type(
            *envelope,
            ObuType::RegularTileGroup,
            "unexpected_leading_obu_after_key",
        )?;
    }
    Ok(())
}

fn require_inter_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    for (index, envelope) in obus.iter().enumerate() {
        let expected = if index % 2 == 0 {
            ObuType::TemporalDelimiter
        } else {
            ObuType::RegularTileGroup
        };
        if envelope.header.obu_type != expected {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        }
    }
    if !obus.len().is_multiple_of(2) {
        let offset = obus
            .last()
            .map_or(ByteOffset::new(0), |envelope| envelope.offset);
        return Err(unsupported_at(
            "unexpected_inter_obu_order",
            offset,
            missing_capability_message!("inter.ivf_frame_unit_order"),
        ));
    }
    Ok(())
}
fn ensure_multiframe_plan_shape(plan: &DecodeStreamPlan) -> Result<()> {
    let frame_count = plan.frame_candidate_count();
    if frame_count == 0 {
        return Err(unsupported(
            "unsupported_frame_candidate_count",
            None,
            "minimal tier requires at least one selected key frame candidate",
        ));
    }
    if plan.obu_count() >= 3 {
        Ok(())
    } else {
        Err(unsupported(
            "unexpected_planned_stream_shape",
            None,
            "minimal tier requires a leading [TD, SEQ, CLK] frame unit",
        ))
    }
}
fn require_multiframe_ivf<'a>(
    parsed: &'a ParsedBitstream<'a>,
) -> Result<(&'a ParsedIvfBitstream<'a>, IvfHeader)> {
    let ParsedBitstream::Ivf(ivf) = parsed else {
        return Err(unsupported(
            "non_ivf_input",
            None,
            missing_capability_message!("container.ivf"),
        ));
    };
    let Some(header) = ivf.header else {
        return Err(unsupported(
            "missing_ivf_header",
            None,
            "minimal tier requires a complete IVF header",
        ));
    };
    let parsed_frame_count = ivf.frames.len() as u64;
    let header_frame_count = u64::from(header.frame_count);
    let header_count_matches = header_frame_count == 0 || header_frame_count == parsed_frame_count;
    let all_frame_records_positive = ivf.frames.iter().all(|frame| frame.frame.size > 0);
    if header.fourcc != *b"AV02"
        || header.width == 0
        || header.height == 0
        || ivf.frames.is_empty()
        || !header_count_matches
        || !all_frame_records_positive
        || !supported_ivf_warnings(&ivf.warnings)
        || ivf.error.is_some()
    {
        return Err(unsupported(
            "unsupported_ivf_shape",
            None,
            missing_capability_message!("container.ivf_av02_frame_records"),
        ));
    }
    Ok((ivf, header))
}

fn supported_ivf_warnings(warnings: &[IvfWarning]) -> bool {
    warnings
        .iter()
        .all(|warning| matches!(warning, IvfWarning::TrailingPartialFrameHeader { .. }))
}

fn ensure_output_frame_count_limit(
    limits: crate::DecodeLimits,
    output_frame_count: u64,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxOutputFrames, output_frame_count)?;
    Ok(())
}

fn ensure_retained_frame_byte_limits(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame: &PipelineFrame,
) -> Result<u64> {
    let frame_bytes = retained_decoded_frame_bytes(frame)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

fn ensure_retained_frame_byte_limits_for_core(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<u64> {
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_retained_limit",
            offset,
            "decode pipeline requires parsed frame dimensions before charging retained decoded-frame bytes",
        )
    })?;
    let bit_depth = match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    };
    let frame_bytes = decoded_frame_storage_budget(
        frame_size,
        sequence.general.chroma_format_idc,
        bytes_per_sample(bit_depth),
    )
    .map(|budget| budget.decoded_bytes)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

fn ensure_retained_frame_byte_limits_for_bytes(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame_bytes: u64,
) -> Result<u64> {
    let next_retained_frame_bytes = checked_add(
        DecodeLimitName::MaxReferenceStoreBytes,
        retained_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(
        DecodeLimitName::MaxReferenceStoreBytes,
        next_retained_frame_bytes,
    )?;
    Ok(next_retained_frame_bytes)
}

fn retained_decoded_frame_bytes(frame: &PipelineFrame) -> Result<u64> {
    Ok(frame.byte_len()? as u64)
}

fn ensure_output_frame_byte_limits(
    limits: crate::DecodeLimits,
    output_frame_bytes: u64,
    frame: &PipelineFrame,
) -> Result<u64> {
    let frame_bytes = frame.byte_len()? as u64;
    let next_output_frame_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        output_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, next_output_frame_bytes)?;
    Ok(next_output_frame_bytes)
}

pub(crate) fn require_minimal_obu_order<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<[ObuEnvelope<'a>; 3]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok([
        obus[indices.temporal_delimiter],
        obus[indices.sequence],
        obus[indices.frame],
    ])
}

fn require_leading_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<([ObuEnvelope<'a>; 3], usize)> {
    let frame_unit = require_minimal_obu_order(obus)?;
    require_obu_type(
        frame_unit[0],
        ObuType::TemporalDelimiter,
        "missing_temporal_delimiter",
    )?;
    require_obu_type(
        frame_unit[1],
        ObuType::SequenceHeader,
        "missing_sequence_header",
    )?;
    require_obu_type(
        frame_unit[2],
        ObuType::ClosedLoopKey,
        "missing_closed_loop_key",
    )?;
    Ok((frame_unit, minimal_frame_unit_indices(obus)?.len()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MinimalFrameUnitIndices {
    temporal_delimiter: usize,
    sequence: usize,
    first_film_grain: usize,
    frame: usize,
}

impl MinimalFrameUnitIndices {
    const fn len(self) -> usize {
        self.frame + 1
    }
}

fn leading_film_grain_obus<'a>(obus: &'a [ObuEnvelope<'a>]) -> Result<&'a [ObuEnvelope<'a>]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok(&obus[indices.first_film_grain..indices.frame])
}

fn minimal_frame_unit_indices(obus: &[ObuEnvelope<'_>]) -> Result<MinimalFrameUnitIndices> {
    if obus.is_empty() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "minimal tier requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    let mut sequence_index = 1usize;
    while obus
        .get(sequence_index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::OperatingPointSet)
    {
        sequence_index += 1;
    }
    let frame_index = sequence_index.checked_add(1).ok_or_else(|| {
        unsupported(
            "unexpected_obu_order",
            None,
            "minimal tier requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        )
    })?;
    let mut frame_index = frame_index;
    while obus
        .get(frame_index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::FilmGrain)
    {
        frame_index += 1;
    }
    if frame_index >= obus.len() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "minimal tier requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    Ok(MinimalFrameUnitIndices {
        temporal_delimiter: 0,
        sequence: sequence_index,
        first_film_grain: sequence_index + 1,
        frame: frame_index,
    })
}

fn require_obu_type(
    envelope: ObuEnvelope<'_>,
    expected: ObuType,
    reason: &'static str,
) -> Result<()> {
    if envelope.header.obu_type == expected {
        Ok(())
    } else {
        Err(unsupported_at(
            reason,
            envelope.offset,
            missing_capability_message!("obu.order minimal_frame_unit"),
        ))
    }
}

pub(crate) fn parse_sequence(envelope: ObuEnvelope<'_>) -> Result<SequenceHeader> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    parse_sequence_header(&mut reader).map_err(|_| {
        unsupported_at(
            "sequence_header_parse",
            envelope.offset,
            "minimal tier requires a fully parseable sequence header",
        )
    })
}

fn validate_sequence(sequence: &SequenceHeader, offset: ByteOffset) -> Result<()> {
    let general = &sequence.general;
    if !sequence.is_fully_parsed() {
        return Err(unsupported_at(
            "sequence_header_not_fully_parsed",
            offset,
            "minimal tier requires a fully parsed sequence header",
        ));
    }
    if !supported_profile_chroma(general.seq_profile_idc.get(), general.chroma_format_idc) {
        return Err(unsupported_at(
            "unsupported_profile",
            offset,
            "minimal tier requires a supported Annex A profile/chroma combination",
        ));
    }
    if general.max_tlayer_id.get() != 0 || general.max_mlayer_id.get() != 0 {
        return Err(unsupported_at(
            "non_base_layer_sequence",
            offset,
            "minimal tier requires a single base temporal and embedded layer",
        ));
    }
    if general.seq_cropping_window_present_flag {
        return Err(unsupported_at(
            "crop_window_present",
            offset,
            "minimal tier does not support sequence crop windows",
        ));
    }
    if sequence.intra.is_none() {
        return Err(unsupported_at(
            "missing_sequence_intra_config",
            offset,
            "minimal tier requires a fully parsed sequence intra config",
        ));
    }
    Ok(())
}

fn ensure_repeated_key_sequence_compatible(
    leading: &SequenceHeader,
    repeated: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    if repeated != leading {
        return Err(unsupported_at(
            "repeated_key_sequence_changed",
            offset,
            "repeated closed-loop-key frame units must repeat the leading sequence until runtime sequence switching is implemented",
        ));
    }
    Ok(())
}

fn supported_profile_chroma(profile_idc: u8, chroma: ChromaFormatIdc) -> bool {
    match profile_idc {
        0..=2 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
        ),
        3 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv422
        ),
        4 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv444
        ),
        _ => false,
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(crate) fn ensure_runtime_storage_bit_depth(
    sequence: &SequenceHeader,
    _offset: ByteOffset,
) -> Result<()> {
    match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight | BitDepthIdc::Ten => Ok(()),
    }
}

pub(crate) fn parse_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        unsupported_at(
            "tile_group_prefix_parse",
            envelope.offset,
            "minimal tier requires a parseable first tile-group prefix",
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(unsupported_at(
            "non_first_tile_group",
            envelope.offset,
            "minimal tier requires the frame header in the first tile group",
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu: true,
        active_sequence: Some(sequence),
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "frame_header_parse",
            envelope.offset,
            "minimal tier requires a fully parseable closed-loop-key frame header",
        )
    })
}
pub(crate) fn frame_ref_update_from_core(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_cdfs: FrameCdfSubset,
    ccso_params: Option<splot_core::headers::frame::CcsoParams>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    motion_field: inter::TemporalMotionField,
    order_hint: u32,
) -> Result<reference_buffer::FrameRefUpdate> {
    let refresh_frame_flags = core.refresh_frame_flags.ok_or_else(|| {
        unsupported_at(
            "missing_refresh_frame_flags",
            offset,
            "minimal multi-frame decode requires a parsed refresh_frame_flags for the §7.23 update",
        )
    })?;
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_ref_update",
            offset,
            "minimal multi-frame decode requires a parsed frame size for the §7.23 update",
        )
    })?;
    let base_q_idx = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            unsupported_at(
                "missing_base_q_for_ref_update",
                offset,
                "minimal multi-frame decode requires a parsed base_q_idx for the §7.23 update",
            )
        })?;
    let is_inter = core.frame_type == Some(FrameType::Inter);
    let adapted = core.disable_cdf_update != Some(true);
    Ok(reference_buffer::FrameRefUpdate {
        refresh_frame_flags,
        order_hint,
        width: frame_size.width,
        height: frame_size.height,
        base_q_idx,
        is_key_or_switch: core.is_key_frame || core.frame_type == Some(FrameType::Switch),
        is_inter,
        adapted,
        frame_cdfs,
        ccso_params,
        ccso_grid,
        motion_field,
        lr_frame_filter_class_counts: lr_frame_filter_class_counts(core),
        lr_frame_filter_taps: lr_frame_filter_taps(core),
    })
}

fn lr_frame_filter_taps(core: &FrameHeaderCore) -> [Vec<Vec<i16>>; 3] {
    let mut taps: [Vec<Vec<i16>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let Some(lr) = core.lr_params.as_ref() else {
        return taps;
    };
    for (plane, params) in lr.planes.iter().enumerate().take(3) {
        if !params.frame_filters_on {
            continue;
        }
        let Some(bank) = params.frame_filter_bank.as_ref() else {
            continue;
        };
        taps[plane] = bank
            .classes
            .iter()
            .map(|class| class.coeffs.clone())
            .collect();
    }
    taps
}

fn lr_frame_filter_class_counts(core: &FrameHeaderCore) -> [u8; 3] {
    let mut counts = [0u8; 3];
    let Some(lr) = core.lr_params.as_ref() else {
        return counts;
    };
    for (plane, params) in lr.planes.iter().enumerate().take(3) {
        if !params.frame_filters_on {
            continue;
        }
        let classes = params
            .frame_filter_bank
            .as_ref()
            .map(|bank| bank.classes.len())
            .or_else(|| params.num_filter_classes.map(usize::from))
            .unwrap_or(1);
        counts[plane] = u8::try_from(classes).unwrap_or(u8::MAX);
    }
    counts
}

fn ensure_intra_header_complete(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(incomplete_intra_header_error(core.status, offset));
    }
    Ok(())
}

pub(crate) fn incomplete_intra_header_error(
    status: FrameHeaderParseStatus,
    offset: ByteOffset,
) -> DecodeError {
    match status {
        FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { .. } => unsupported_feature_at(
            "unsupported_wienerns_filter",
            offset,
            missing_capability_message!("filters.wiener_ns read_wienerns_filter §5.18.7.11"),
            "5.18.7.11",
        ),
        _ => unsupported_at(
            "incomplete_frame_header",
            offset,
            "minimal tier requires a complete intra frame header",
        ),
    }
}

pub(crate) mod frame_engine;
pub(crate) mod general_intra;
pub(crate) mod reconstruct;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_lossless_d157_tests;
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
            "minimal tier requires sequence transform/quant/entropy config",
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
            "minimal tier could not derive a source-backed tile payload boundary",
        ),
        FrameCandidateTileBoundaryError::MissingFact { .. } => unsupported(
            "missing_tile_fact",
            None,
            "minimal tier requires complete parser-derived tile facts",
        ),
        FrameCandidateTileBoundaryError::Unsupported { .. }
        | FrameCandidateTileBoundaryError::Boundary(_) => unsupported(
            "unsupported_tile_boundary",
            None,
            "minimal tier requires source-backed tile work units",
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
fn bytes_per_sample(bit_depth: BitDepth) -> u64 {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
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
