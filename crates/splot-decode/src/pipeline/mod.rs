// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode pipeline orchestration for the supported decode runtime.
//!
//! The driver updates output and reference state in decode order. Entropy,
//! reconstruction and filters run on the row scheduler; frame delay bounds
//! their overlap, and retired publications stay with bounded physical frame
//! entries.

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
use crate::bitstream::byte_stream::{InputScratch, SourceBytes, UnitSource, recycle};
use crate::bitstream::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    GeneralIntraBlockModeError, GeneralIntraResidualError, TileGroupPositionFacts,
};
use crate::error::{DecodeError, Result};
use crate::prediction::inter;
use crate::reference::buffer as reference_buffer;
use crate::support::pipeline_limits::checked_add;
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu};

mod frame_lifecycle;
pub(crate) mod frame_store;
pub(crate) use frame_store::DecodeSession;
use frame_store::FrameStore;
pub(crate) mod frame_progress;
pub(crate) mod frame_tasks;
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
#[cfg(test)]
pub(crate) use stream_schedule::ivf_inter_envelope;
#[cfg(test)]
pub(crate) use stream_schedule::require_minimal_obu_order;
use stream_schedule::*;

pub(crate) const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
pub(crate) const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";

#[cfg(test)]
pub(crate) fn decode_frame_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<PipelineFrame> {
    let mut frames = decode_frames_from_plan(bytes, options)?;
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
) -> Result<Vec<PipelineFrame>> {
    decode_frames_from_plan_on_threads(bytes, options, 1)
}

/// [`decode_frames_from_plan`] on a row scheduler of `threads` workers, with
/// one frame in flight.
#[cfg(test)]
pub(crate) fn decode_frames_from_plan_on_threads(
    bytes: &[u8],
    options: &DecodeOptions,
    threads: usize,
) -> Result<Vec<PipelineFrame>> {
    let threads = NonZeroUsize::new(threads).unwrap_or(NonZeroUsize::MIN);
    let session = DecodeSession::new(NonZeroUsize::MIN, row_scheduler(threads)?);
    splot_parallel::run_on_thread("splot-driver", || {
        let mut source = crate::bitstream::byte_stream::ReaderUnits::new(bytes, options.limits());
        decode_frames_from_plan_impl(&mut source, options, &session, |_| Ok(()), true, |_| Ok(()))
    })?
}

/// Decodes every temporal unit of `source` and passes each displayed frame
/// to `emit` in display order.
pub(crate) fn emit_frames(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &DecodeSession,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<()> {
    decode_frames_from_plan_impl(source, options, session, preflight, false, emit).map(drop)
}

/// Retires unowned frames into their physical entries and returns their planes.
/// Shared samples retire only when another tracked frame accounts for their
/// bytes; otherwise subtracting them would undercount `MaxReferenceStoreBytes`.
/// Every call rescans readers skipped by earlier reclamation passes.
fn reclaim_unowned_frames(
    frames: &mut FrameStore,
    reference: &reference_buffer::RuntimeReferenceBuffer,
    scheduler: &mut OutputScheduler,
    emission: &output_schedule::EmissionQueue,
    ring: &mut inflight::InflightRing,
    retained_frame_bytes: &mut u64,
) -> Result<()> {
    for slot_index in 0..frames.entries.len() {
        let frame_index = frames.entries[slot_index].index;
        let Some(frame) = frames.entries[slot_index].frame.as_ref() else {
            continue;
        };
        if reference.retains(frame_index)
            || scheduler.retains(frame_index)
            || emission.holds(frame_index)
            || ring.holds(frame_index)
            || !frame.frame.is_settled()
            || !frame.frame.is_sole_handle()
        {
            continue;
        }
        let shares = frames
            .entries
            .iter()
            .enumerate()
            .any(|(other_index, other)| {
                other_index != slot_index
                    && other
                        .frame
                        .as_ref()
                        .is_some_and(|other| frame.frame.shares_samples(&other.frame))
            });
        if frame.frame.handle_count() > 1 && !shares && !frames.detach_held {
            continue;
        }
        let Some(frame) = frames.entries[slot_index].frame.take() else {
            continue;
        };
        let frame_bytes = if shares { 0 } else { frame.byte_len()? as u64 };
        *retained_frame_bytes = retained_frame_bytes
            .checked_sub(frame_bytes)
            .ok_or_else(|| {
                unsupported(
                    "retained_frame_byte_accounting_underflow",
                    None,
                    "decode pipeline live-frame byte accounting underflowed",
                )
            })?;
        scheduler.forget(frame_index);
        frames.entries[slot_index].retired = Some(ring.keep_frame_planes(frame.frame));
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
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    frame_rate: PipelineFrameRate,
    display_grain: Option<ActiveFilmGrain>,
) -> Result<PipelineFrame> {
    let core = parse_frame_core(frame_envelope, sequence)?;
    let buffers = crate::support::decode_buffers::DecodeBuffers::new();
    let mut scratch_eight = inter::InterDecodeScratch::default();
    let mut scratch_ten = inter::InterDecodeScratch::default();
    scratch_eight.set_decode_buffers(&buffers);
    scratch_ten.set_decode_buffers(&buffers);
    let mut ring = inflight::InflightRing::new(NonZeroUsize::MIN, buffers, 1);
    let mut pools = frame_tasks::RowPools::default();
    let mut frames = FrameStore::new(false, ring.capacity());
    let decoded = decode_key_frame_with_effects(
        &mut scratch_eight,
        &mut scratch_ten,
        None,
        &mut pools,
        &mut ring,
        &mut frames,
        None,
        0,
        bytes.into(),
        options,
        candidate,
        frame_envelope,
        core,
        sequence,
        frame_rate,
        display_grain,
        None,
        FrameOutputEffects::empty(),
        crate::DataProps::default(),
    );
    ring.harvest_all(&mut scratch_eight, &mut scratch_ten);
    match ring.take_failure() {
        Some(failure) => Err(failure),
        None => decoded,
    }
}

/// The null reference set of a key / intra-only frame walk.
fn empty_key_reference<T: splot_recon::ReconSample>(
    offset: ByteOffset,
) -> Result<inter::InterReferenceState<T>> {
    inter::InterReferenceState::empty().map_err(|_| {
        unsupported_at(
            "frame_engine_intra_reference_store",
            offset,
            "intra frame decode requires a reference store",
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_key_frame_with_effects(
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
    rows: Option<&splot_parallel::RowScheduler>,
    pools: &mut frame_tasks::RowPools,
    ring: &mut inflight::InflightRing,
    frames: &mut FrameStore,
    split: Option<(
        SplitPair<'_>,
        &crate::bitstream::byte_stream::UnitBytes,
        &mut Option<Arc<SequenceHeader>>,
    )>,
    frame_index: usize,
    bytes: SourceBytes<'_>,
    options: &DecodeOptions,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    frame_rate: PipelineFrameRate,
    display_grain: Option<ActiveFilmGrain>,
    user_qm: Option<crate::bitstream::tile_payload::FrameUserQmLevels>,
    output_effects: FrameOutputEffects,
    props: crate::DataProps,
) -> Result<PipelineFrame> {
    ring.reserve(scratch_eight, scratch_ten);
    let _user_qm_scope = crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
    let core = frames.share_core(core)?;
    if let (Some(rows), Some(((split_eight, split_ten), unit, shared_sequence))) = (rows, split)
        && inter::splittable_inter_frame(frame_envelope.header.obu_type, &core)
    {
        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
            frame_engine::intra::build_frame_qm_levels(&core),
        );
        let (slot, core, products, motion_field) = match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => push_split_inter::<u8>(
                split_eight,
                rows,
                ring,
                frames,
                scratch_eight,
                options,
                sequence,
                shared_sequence,
                candidate,
                unit,
                core,
                empty_key_reference(frame_envelope.offset)?,
                frame_index,
            )?,
            BitDepthIdc::Ten => push_split_inter::<u16>(
                split_ten,
                rows,
                ring,
                frames,
                scratch_ten,
                options,
                sequence,
                shared_sequence,
                candidate,
                unit,
                core,
                empty_key_reference(frame_envelope.offset)?,
                frame_index,
            )?,
        };
        let frame_rate = output_effects.frame_rate(frame_rate);
        return Ok(PipelineFrame {
            frame: slot,
            display_grain,
            output_effects,
            frame_cdfs: products.frame_cdfs,
            motion_field,
            ccso_params: core.ccso_params.clone(),
            ccso_grid: products.ccso_grid,
            segment_ids: products.segment_ids,
            frame_rate_numerator: frame_rate.numerator,
            frame_rate_denominator: frame_rate.denominator,
            props,
        });
    }
    let mut product_writers = frames.reserve_products()?;
    let (frame, frame_cdfs, ccso_params, ccso_grid, segment_ids, motion_field) =
        match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let walk = frame_engine::walk_frame::<u8>(
                    scratch_eight,
                    candidate,
                    bytes,
                    frame_envelope,
                    core,
                    sequence,
                    options,
                    &frame_engine::FrameSetup::Intra,
                    BitDepth::Eight,
                    &mut product_writers,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Eight,
                    rows,
                    pools,
                    ring,
                    frames,
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
                    candidate,
                    bytes,
                    frame_envelope,
                    core,
                    sequence,
                    options,
                    &frame_engine::FrameSetup::Intra,
                    BitDepth::Ten,
                    &mut product_writers,
                )?;
                let ccso_params = walk.core.ccso_params.clone();
                let frame = inflight::settle_walk_stage(
                    walk.stage,
                    inflight::PipelineFrameSlot::Ten,
                    rows,
                    pools,
                    ring,
                    frames,
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
    let (frame_cdfs, ccso_grid, segment_ids) = product_writers
        .settle(frame_cdfs, ccso_grid, segment_ids)
        .into_parts();
    let frame_rate = output_effects.frame_rate(frame_rate);
    Ok(PipelineFrame {
        frame,
        display_grain,
        output_effects,
        frame_cdfs,
        motion_field: frames.settle_motion(motion_field)?,
        ccso_params,
        ccso_grid,
        segment_ids,
        frame_rate_numerator: frame_rate.numerator,
        frame_rate_denominator: frame_rate.denominator,
        props,
    })
}

/// Runs the frame loop over the retained decode state. The resolved frame
/// delay bounds in-flight storage at every worker count.
#[allow(clippy::too_many_arguments)]
fn decode_frames_from_plan_impl(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &DecodeSession,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<Vec<PipelineFrame>> {
    let pipeline_capacity = session
        .frame_delay()
        .min(NonZeroUsize::new(session.threads()).unwrap_or(NonZeroUsize::MIN));
    let mut retained = session.take(pipeline_capacity);
    retained.begin(retain_decoded_frames);
    let decoded = drive_frames(
        source,
        options,
        &mut retained,
        session.rows(),
        preflight,
        retain_decoded_frames,
        emit,
    );
    session.keep(retained);
    let decoded = decoded?;
    Ok(decoded)
}

/// Starts the row scheduler of a context with more than one worker thread.
///
/// # Errors
/// Returns [`crate::DecodeError::Pool`] when a worker thread cannot start.
pub(crate) fn row_scheduler(
    threads: NonZeroUsize,
) -> Result<Option<Arc<splot_parallel::RowScheduler>>> {
    if threads.get() <= 1 {
        return Ok(None);
    }
    Ok(Some(Arc::new(splot_parallel::RowScheduler::new(threads)?)))
}

type SplitPair<'a> = (
    &'a mut frame_tasks::SplitFrames<u8>,
    &'a mut frame_tasks::SplitFrames<u16>,
);

/// Runs one decode over its retained storage, and resolves the run's outcome
/// against the filter phases the ring collected.
///
/// A filter-phase failure outranks the frame loop's own error: serial decode
/// would have run that frame's filters before reaching the later error, so the
/// lowest-indexed collected failure is the one the caller sees.
#[allow(clippy::too_many_arguments)]
fn drive_frames(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    retained: &mut frame_store::RetainedDecode,
    rows: Option<&splot_parallel::RowScheduler>,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    emit: impl FnMut(&PipelineFrame) -> Result<()>,
) -> Result<Vec<PipelineFrame>> {
    let frame_store::RetainedDecode {
        scratch_eight,
        scratch_ten,
        ring,
        pools,
        frames,
        input_scratch,
        split_eight,
        split_ten,
        ..
    } = retained;
    let decoded = decode_frames_in_order(
        source,
        options,
        preflight,
        retain_decoded_frames,
        emit,
        rows,
        ring,
        pools,
        frames,
        (split_eight, split_ten),
        scratch_eight,
        scratch_ten,
        input_scratch,
    );
    ring.harvest_all(scratch_eight, scratch_ten);
    if let Some(rows) = rows {
        split_eight.drain(rows);
        split_ten.drain(rows);
        pools.drain(rows);
    }
    match ring.take_failure() {
        Some(failure) => Err(failure),
        None => decoded,
    }
}

/// Hands one single-tile inter frame to the row scheduler and returns the
/// handles the driver records for it before its samples exist.
/// Shares the driver's active sequence header with the frames it defers.
///
/// A deferred frame's reconstruction outlives the driver's borrow of the
/// header, and the header only ever changes at a frame the driver flushes
/// before, so the shared copy is made once per activation.
fn share_sequence(
    cached: &mut Option<Arc<SequenceHeader>>,
    sequence: &SequenceHeader,
) -> Arc<SequenceHeader> {
    Arc::clone(cached.get_or_insert_with(|| Arc::new(sequence.clone())))
}

#[allow(clippy::too_many_arguments)]
fn push_split_inter<T: frame_tasks::SplitSample>(
    split: &mut frame_tasks::SplitFrames<T>,
    rows: &splot_parallel::RowScheduler,
    ring: &mut inflight::InflightRing,
    frames: &mut FrameStore,
    decode_scratch: &mut inter::InterDecodeScratch<T>,
    options: &DecodeOptions,
    sequence: &SequenceHeader,
    shared_sequence: &mut Option<Arc<SequenceHeader>>,
    candidate: &DecodePlannedObu,
    unit: &crate::bitstream::byte_stream::UnitBytes,
    core: Arc<FrameHeaderCore>,
    reference: inter::InterReferenceState<T>,
    frame_index: usize,
) -> Result<(
    inflight::PipelineFrameSlot,
    Arc<FrameHeaderCore>,
    inter::FrameProducts,
    inter::MotionFieldHandle,
)> {
    let slot_index = split.reserve(rows, *options);
    let parse_progress = ring.claim_parse_slot();
    let records = decode_scratch.take_frame_filter_records();
    let quantizer = crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
    let shared = share_sequence(shared_sequence, sequence);
    let geometry = inter::FrameDecodeGeometry::new(
        &core,
        sequence,
        T::BIT_DEPTH,
        core.frame_is_intra == Some(true),
    )?;
    let motion = frames.reserve_motion(geometry.motion_layout())?;
    let (slot, finish) =
        inflight::reserve_pending_slot(geometry.info(), T::erase, ring, frames, frame_index)?;
    let entropy = inter::entropy_dependencies(&core, sequence, &reference);
    let motion_deps = core
        .inter
        .as_ref()
        .map_or_else(reference_buffer::RefSlots::default, |inter| {
            reference.motion_dependencies(&inter.ref_frame_idx)
        });
    let writers = frames.reserve_products()?;
    let publications = writers.handles();
    split.push(
        rows,
        slot_index,
        frame_tasks::SplitFrameInputs {
            records,
            candidate: candidate.clone(),
            core: Arc::clone(&core),
            sequence: shared,
            reference,
            bit_depth: T::BIT_DEPTH,
            geometry,
            motion: motion.clone(),
            parse_progress,
            quantizer,
            products: writers,
            unit: unit.clone(),
            entropy,
            motion_deps,
            finish,
        },
    );
    Ok((slot, core, publications, motion))
}

#[allow(clippy::too_many_arguments)]
fn decode_frames_in_order(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()>,
    retain_decoded_frames: bool,
    mut emit: impl FnMut(&PipelineFrame) -> Result<()>,
    rows: Option<&splot_parallel::RowScheduler>,
    ring: &mut inflight::InflightRing,
    pools: &mut frame_tasks::RowPools,
    frames: &mut FrameStore,
    (split_eight, split_ten): SplitPair<'_>,
    decode_scratch_eight: &mut inter::InterDecodeScratch<u8>,
    decode_scratch_ten: &mut inter::InterDecodeScratch<u16>,
    input_scratch: &mut InputScratch,
) -> Result<Vec<PipelineFrame>> {
    let InputScratch {
        records,
        obus: obu_storage,
    } = input_scratch;
    let mut stream = RuntimeStream::new(source, records, options.limits());
    let Some(leading_unit) = stream.first_unit()? else {
        return Ok(Vec::new());
    };
    let key_candidate = stream.first_candidate()?;
    let mut leading_buffer = recycle(core::mem::take(obu_storage));
    let leading_obus = RuntimeStream::obus(&leading_unit, &mut leading_buffer)?;
    let ([_, sequence_envelope, key_envelope], _) = require_leading_frame_unit(leading_obus)?;
    preflight(stream.ivf_header())?;
    let frame_rate = stream.frame_rate();

    let mut sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;
    let mut film_grain_slots = FilmGrainSlots::new();
    let leading_prefix = leading_prefix_obus(leading_obus)?;
    film_grain_slots.update_from_obus(leading_prefix)?;
    let mut output_effect_state = OutputEffectState::new();
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
    let mut scheduler = OutputScheduler::new(num_ref_frames);
    let mut emission_queue = output_schedule::EmissionQueue::default();
    let mut in_band_long_term_prelude = InBandLongTermPrelude::default();
    in_band_long_term_prelude.begin_frame(true);
    let key_candidate = &key_candidate?;
    let key_frame_index = key_candidate.ivf_frame().map(IvfFrameContext::frame_index);
    let key_core = match key_envelope.header.obu_type {
        ObuType::ClosedLoopKey => {
            parse_key_core_with_effects(key_envelope, &sequence, &output_effect_state)?
        }
        ObuType::OpenLoopKey => match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let state = reference.state_eight(frames)?;
                parse_olk_core_with_effects(
                    key_envelope,
                    &sequence,
                    &state,
                    &output_effect_state,
                    true,
                )?
            }
            BitDepthIdc::Ten => {
                let state = reference.state_ten(frames)?;
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
                let state = reference.state_eight(frames)?;
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
                let state = reference.state_ten(frames)?;
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
    output_effect_state.observe_suffix(suffix_after_candidate(leading_obus, key_candidate)?)?;
    let key_output_effects = output_effect_state.finish_frame();
    let mut retained_frame_bytes = 0;
    ensure_retained_frame_byte_limits_for_core(
        options.limits(),
        retained_frame_bytes,
        &key_core,
        &sequence,
        key_envelope.offset,
    )?;
    frames.reserve()?;
    let mut shared_sequence = None;
    let key_frame = if key_envelope.header.obu_type == ObuType::RasFrame {
        match sequence.general.bit_depth_idc {
            BitDepthIdc::Eight => {
                let state = reference.state_eight(frames)?;
                ring.reserve(decode_scratch_eight, decode_scratch_ten);
                let _user_qm_scope =
                    crate::bitstream::tile_payload::FrameUserQmScope::install(key_user_qm);
                let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                    frame_engine::intra::build_frame_qm_levels(&key_core),
                );
                if let Some(rows) =
                    rows.filter(|_| inter::splittable_inter_frame(ObuType::RasFrame, &key_core))
                {
                    let core = frames.share_core(key_core.clone())?;
                    let (slot, core, products, motion_field) = push_split_inter::<u8>(
                        split_eight,
                        rows,
                        ring,
                        frames,
                        decode_scratch_eight,
                        options,
                        &sequence,
                        &mut shared_sequence,
                        key_candidate,
                        &leading_unit,
                        core,
                        state,
                        0,
                    )?;
                    let rate = key_output_effects.frame_rate(frame_rate);
                    PipelineFrame {
                        frame: slot,
                        display_grain: key_display_grain,
                        output_effects: key_output_effects,
                        frame_cdfs: products.frame_cdfs,
                        motion_field,
                        ccso_params: core.ccso_params.clone(),
                        ccso_grid: products.ccso_grid,
                        segment_ids: products.segment_ids,
                        frame_rate_numerator: rate.numerator,
                        frame_rate_denominator: rate.denominator,
                        props: stream.props(),
                    }
                } else {
                    let mut product_writers = frames.reserve_products()?;
                    let walk = frame_engine::walk_frame::<u8>(
                        decode_scratch_eight,
                        key_candidate,
                        leading_unit.view(),
                        key_envelope,
                        frames.share_core(key_core.clone())?,
                        &sequence,
                        options,
                        &frame_engine::FrameSetup::Inter(&state),
                        BitDepth::Eight,
                        &mut product_writers,
                    )?;
                    let ccso_params = walk.core.ccso_params.clone();
                    let frame = inflight::settle_walk_stage(
                        walk.stage,
                        inflight::PipelineFrameSlot::Eight,
                        rows,
                        pools,
                        ring,
                        frames,
                        0,
                    )?;
                    let rate = key_output_effects.frame_rate(frame_rate);
                    let (frame_cdfs, ccso_grid, segment_ids) = product_writers
                        .settle(walk.frame_cdfs, walk.ccso_grid, walk.segment_ids)
                        .into_parts();
                    PipelineFrame {
                        frame,
                        display_grain: key_display_grain,
                        output_effects: key_output_effects,
                        frame_cdfs,
                        motion_field: frames.settle_motion(walk.motion_field)?,
                        ccso_params,
                        ccso_grid,
                        segment_ids,
                        frame_rate_numerator: rate.numerator,
                        frame_rate_denominator: rate.denominator,
                        props: stream.props(),
                    }
                }
            }
            BitDepthIdc::Ten => {
                let state = reference.state_ten(frames)?;
                ring.reserve(decode_scratch_eight, decode_scratch_ten);
                let _user_qm_scope =
                    crate::bitstream::tile_payload::FrameUserQmScope::install(key_user_qm);
                let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                    frame_engine::intra::build_frame_qm_levels(&key_core),
                );
                if let Some(rows) =
                    rows.filter(|_| inter::splittable_inter_frame(ObuType::RasFrame, &key_core))
                {
                    let core = frames.share_core(key_core.clone())?;
                    let (slot, core, products, motion_field) = push_split_inter::<u16>(
                        split_ten,
                        rows,
                        ring,
                        frames,
                        decode_scratch_ten,
                        options,
                        &sequence,
                        &mut shared_sequence,
                        key_candidate,
                        &leading_unit,
                        core,
                        state,
                        0,
                    )?;
                    let rate = key_output_effects.frame_rate(frame_rate);
                    PipelineFrame {
                        frame: slot,
                        display_grain: key_display_grain,
                        output_effects: key_output_effects,
                        frame_cdfs: products.frame_cdfs,
                        motion_field,
                        ccso_params: core.ccso_params.clone(),
                        ccso_grid: products.ccso_grid,
                        segment_ids: products.segment_ids,
                        frame_rate_numerator: rate.numerator,
                        frame_rate_denominator: rate.denominator,
                        props: stream.props(),
                    }
                } else {
                    let mut product_writers = frames.reserve_products()?;
                    let walk = frame_engine::walk_frame::<u16>(
                        decode_scratch_ten,
                        key_candidate,
                        leading_unit.view(),
                        key_envelope,
                        frames.share_core(key_core.clone())?,
                        &sequence,
                        options,
                        &frame_engine::FrameSetup::Inter(&state),
                        BitDepth::Ten,
                        &mut product_writers,
                    )?;
                    let ccso_params = walk.core.ccso_params.clone();
                    let frame = inflight::settle_walk_stage(
                        walk.stage,
                        inflight::PipelineFrameSlot::Ten,
                        rows,
                        pools,
                        ring,
                        frames,
                        0,
                    )?;
                    let rate = key_output_effects.frame_rate(frame_rate);
                    let (frame_cdfs, ccso_grid, segment_ids) = product_writers
                        .settle(walk.frame_cdfs, walk.ccso_grid, walk.segment_ids)
                        .into_parts();
                    PipelineFrame {
                        frame,
                        display_grain: key_display_grain,
                        output_effects: key_output_effects,
                        frame_cdfs,
                        motion_field: frames.settle_motion(walk.motion_field)?,
                        ccso_params,
                        ccso_grid,
                        segment_ids,
                        frame_rate_numerator: rate.numerator,
                        frame_rate_denominator: rate.denominator,
                        props: stream.props(),
                    }
                }
            }
        }
    } else {
        decode_key_frame_with_effects(
            decode_scratch_eight,
            decode_scratch_ten,
            rows,
            pools,
            ring,
            frames,
            Some((
                (&mut *split_eight, &mut *split_ten),
                &leading_unit,
                &mut shared_sequence,
            )),
            0,
            leading_unit.view(),
            options,
            key_candidate,
            key_envelope,
            key_core.clone(),
            &sequence,
            frame_rate,
            key_display_grain,
            key_user_qm,
            key_output_effects,
            stream.props(),
        )?
    };
    retained_frame_bytes =
        ensure_retained_frame_byte_limits(options.limits(), retained_frame_bytes, &key_frame)?;
    let key_update = frame_ref_update_from_core(
        &key_core,
        key_envelope.offset,
        key_envelope.header.embedded_layer_id,
    )?;
    frames.push(key_frame)?;
    let key_hint = key_update.order_hint;
    let key_implicit = key_core.implicit_output_frame == Some(true);
    let key_immediate = key_core.immediate_output_frame == Some(true);
    scheduler.refresh(
        key_update.refresh_frame_flags,
        0,
        key_hint,
        key_implicit,
        true,
    );
    charge_emitted_outputs(
        options,
        frames,
        &scheduler,
        &mut emission_queue,
        scheduler.newly(),
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
        scheduler.on_immediate(0, key_hint);
        charge_emitted_outputs(
            options,
            frames,
            &scheduler,
            &mut emission_queue,
            scheduler.newly(),
            &mut emit,
        )?;
    }
    if !retain_decoded_frames {
        reclaim_unowned_frames(
            frames,
            &reference,
            &mut scheduler,
            &emission_queue,
            ring,
            &mut retained_frame_bytes,
        )?;
    }
    if output_frame_limit_reached(options, scheduler.emitted_count) {
        emission_queue.flush(frames, &mut emit)?;
        return if retain_decoded_frames {
            select_output_frames(frames, scheduler.emitted)
        } else {
            Ok(Vec::new())
        };
    }

    *obu_storage = recycle(leading_buffer);
    let mut decoding_initial_tu = true;
    loop {
        if !frames.has_space() {
            ring.harvest_all(decode_scratch_eight, decode_scratch_ten);
            emission_queue.flush(frames, &mut emit)?;
            let mut reclaim = || {
                reclaim_unowned_frames(
                    frames,
                    &reference,
                    &mut scheduler,
                    &emission_queue,
                    ring,
                    &mut retained_frame_bytes,
                )
                .map(|()| frames.has_space())
            };
            match rows {
                Some(rows) => {
                    let mut outcome = Ok(false);
                    rows.wait(|| {
                        outcome = reclaim();
                        !matches!(outcome, Ok(false))
                    });
                    outcome?;
                }
                None => {
                    while !reclaim()? {
                        std::thread::yield_now();
                    }
                }
            }
        }
        frames.reserve()?;
        let Some((next_candidate, unit)) = stream.candidate(obu_storage)? else {
            break;
        };
        let next_candidate = &next_candidate;
        match next_candidate.obu_type() {
            ObuType::LeadingSef | ObuType::RegularSef => {
                let mut obu_buffer = recycle(core::mem::take(obu_storage));
                let obus = RuntimeStream::obus(&unit, &mut obu_buffer)?;
                let (sef_prefix_obus, sef_envelope) =
                    stream.inter_envelope(obus, next_candidate)?;
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
                    frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    &mut emit,
                )?;
                if output_frame_limit_reached(options, scheduler.emitted_count) {
                    break;
                }
                reference.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                let sef_core = match sequence.general.bit_depth_idc {
                    BitDepthIdc::Eight => {
                        let state = reference.state_eight(frames)?;
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
                        let state = reference.state_ten(frames)?;
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
                    scheduler.emitted_count as u64,
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
                output_effect_state
                    .observe_suffix(suffix_after_candidate(obus, next_candidate)?)?;
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
                let source = frames.get(source_index).ok_or_else(|| {
                    unsupported_at(
                        "sef_reference_frame_unavailable",
                        sef_envelope.offset,
                        "show-existing-frame output requires its retained decoded frame",
                    )
                })?;
                let sef_frame = PipelineFrame {
                    display_grain,
                    output_effects,
                    frame_cdfs: source.frame_cdfs.clone(),
                    motion_field: source.motion_field.clone(),
                    ccso_params: source.ccso_params.clone(),
                    ccso_grid: source.ccso_grid.clone(),
                    segment_ids: source.segment_ids.clone(),
                    frame_rate_numerator: output_rate.numerator,
                    frame_rate_denominator: output_rate.denominator,
                    props: stream.props(),
                    frame: inflight::PipelineFrameSlot::completed_recycled(
                        source.wait_ready_frame()?,
                        frames,
                    )?,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &sef_frame,
                )?;
                let frame_index = frames.len();
                frames.push(sef_frame)?;
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
                scheduler.on_immediate(frame_index, ordering);
                charge_emitted_outputs(
                    options,
                    frames,
                    &scheduler,
                    &mut emission_queue,
                    scheduler.newly(),
                    &mut emit,
                )?;
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        frames,
                        &reference,
                        &mut scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, scheduler.emitted_count) {
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
                let mut obu_buffer = recycle(core::mem::take(obu_storage));
                let obus = RuntimeStream::obus(&unit, &mut obu_buffer)?;
                let (inter_prefix_obus, inter_envelope) =
                    stream.inter_envelope(obus, next_candidate)?;
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
                    frames,
                    &scheduler,
                    &mut emission_queue,
                    &flushed,
                    &mut emit,
                )?;
                if output_frame_limit_reached(options, scheduler.emitted_count) {
                    break;
                }
                reference.prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                if matches!(
                    next_candidate.obu_type(),
                    ObuType::Switch | ObuType::RasFrame
                ) {
                    let activation = match sequence.general.bit_depth_idc {
                        BitDepthIdc::Eight => {
                            let state = reference.state_eight(frames)?;
                            inter::parse_inter_frame_activation(
                                inter_envelope,
                                &sequence,
                                &state,
                                first_picture_in_tu,
                                ivf_frame_index,
                            )?
                        }
                        BitDepthIdc::Ten => {
                            let state = reference.state_ten(frames)?;
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
                            frames,
                            &scheduler,
                            &mut emission_queue,
                            &emitted,
                            &mut emit,
                        )?;
                        if output_frame_limit_reached(options, scheduler.emitted_count) {
                            break;
                        }
                    }
                }
                let frame_index = frames.len();
                let decoded = match sequence.general.bit_depth_idc {
                    BitDepthIdc::Eight => {
                        let inter_state = reference.state_eight(frames)?;
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        let inter_core = frames.share_core(inter_core)?;
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
                                scheduler.emitted_count as u64,
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
                        let split = rows.filter(|_| {
                            inter::splittable_inter_frame(next_candidate.obu_type(), &inter_core)
                        });
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if let Some(rows) = split {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            push_split_inter::<u8>(
                                split_eight,
                                rows,
                                ring,
                                frames,
                                decode_scratch_eight,
                                options,
                                &sequence,
                                &mut shared_sequence,
                                next_candidate,
                                &unit,
                                inter_core,
                                inter_state,
                                frame_index,
                            )?
                        } else if inter_envelope.header.obu_type.is_tip_frame() {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let (slot, finish, geometry, products, motion) =
                                frame_tasks::reserve_tip_output(
                                    &inter_core,
                                    &sequence,
                                    BitDepth::Eight,
                                    inflight::PipelineFrameSlot::Eight,
                                    frames,
                                    ring,
                                    frame_index,
                                )?;
                            let publications = products.handles();
                            let entropy =
                                inter::entropy_dependencies(&inter_core, &sequence, &inter_state);
                            let motion_deps = inter_core
                                .inter
                                .as_ref()
                                .map_or_else(reference_buffer::RefSlots::default, |inter| {
                                    inter_state.motion_dependencies(&inter.ref_frame_idx)
                                });
                            let core = Arc::clone(&inter_core);
                            pools.push_tip::<u8>(
                                rows,
                                *options,
                                frame_tasks::TipFrameInputs {
                                    candidate: next_candidate.clone(),
                                    core: inter_core,
                                    sequence: share_sequence(&mut shared_sequence, &sequence),
                                    reference: inter_state,
                                    geometry,
                                    products,
                                    motion: motion.clone(),
                                    finish,
                                    entropy,
                                    motion_deps,
                                    offset: next_candidate.offset(),
                                },
                            )?;
                            (slot, core, publications, motion)
                        } else {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let setup = if inter_core.status
                                == splot_core::headers::frame::FrameHeaderParseStatus::IntraHeaderComplete
                            {
                                frame_engine::FrameSetup::Intra
                            } else {
                                frame_engine::FrameSetup::Inter(&inter_state)
                            };
                            let mut product_writers = frames.reserve_products()?;
                            let walk = frame_engine::walk_frame(
                                decode_scratch_eight,
                                next_candidate,
                                unit.view(),
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &setup,
                                BitDepth::Eight,
                                &mut product_writers,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Eight,
                                rows,
                                pools,
                                ring,
                                frames,
                                frame_index,
                            )?;
                            let products = product_writers.settle(
                                walk.frame_cdfs,
                                walk.ccso_grid,
                                walk.segment_ids,
                            );
                            (
                                slot,
                                inter_core,
                                products,
                                frames.settle_motion(walk.motion_field)?,
                            )
                        }
                    }
                    BitDepthIdc::Ten => {
                        let inter_state = reference.state_ten(frames)?;
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                            &output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        let inter_core = frames.share_core(inter_core)?;
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
                                scheduler.emitted_count as u64,
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
                        let split = rows.filter(|_| {
                            inter::splittable_inter_frame(next_candidate.obu_type(), &inter_core)
                        });
                        let _user_qm_scope =
                            crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                            frame_engine::intra::build_frame_qm_levels(&inter_core),
                        );
                        if let Some(rows) = split {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            push_split_inter::<u16>(
                                split_ten,
                                rows,
                                ring,
                                frames,
                                decode_scratch_ten,
                                options,
                                &sequence,
                                &mut shared_sequence,
                                next_candidate,
                                &unit,
                                inter_core,
                                inter_state,
                                frame_index,
                            )?
                        } else if inter_envelope.header.obu_type.is_tip_frame() {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let (slot, finish, geometry, products, motion) =
                                frame_tasks::reserve_tip_output(
                                    &inter_core,
                                    &sequence,
                                    BitDepth::Ten,
                                    inflight::PipelineFrameSlot::Ten,
                                    frames,
                                    ring,
                                    frame_index,
                                )?;
                            let publications = products.handles();
                            let entropy =
                                inter::entropy_dependencies(&inter_core, &sequence, &inter_state);
                            let motion_deps = inter_core
                                .inter
                                .as_ref()
                                .map_or_else(reference_buffer::RefSlots::default, |inter| {
                                    inter_state.motion_dependencies(&inter.ref_frame_idx)
                                });
                            let core = Arc::clone(&inter_core);
                            pools.push_tip::<u16>(
                                rows,
                                *options,
                                frame_tasks::TipFrameInputs {
                                    candidate: next_candidate.clone(),
                                    core: inter_core,
                                    sequence: share_sequence(&mut shared_sequence, &sequence),
                                    reference: inter_state,
                                    geometry,
                                    products,
                                    motion: motion.clone(),
                                    finish,
                                    entropy,
                                    motion_deps,
                                    offset: next_candidate.offset(),
                                },
                            )?;
                            (slot, core, publications, motion)
                        } else {
                            ring.reserve(decode_scratch_eight, decode_scratch_ten);
                            let setup = if inter_core.status
                                == splot_core::headers::frame::FrameHeaderParseStatus::IntraHeaderComplete
                            {
                                frame_engine::FrameSetup::Intra
                            } else {
                                frame_engine::FrameSetup::Inter(&inter_state)
                            };
                            let mut product_writers = frames.reserve_products()?;
                            let walk = frame_engine::walk_frame(
                                decode_scratch_ten,
                                next_candidate,
                                unit.view(),
                                inter_envelope,
                                inter_core,
                                &sequence,
                                options,
                                &setup,
                                BitDepth::Ten,
                                &mut product_writers,
                            )?;
                            let inter_core = Arc::clone(&walk.core);
                            let slot = inflight::settle_walk_stage(
                                walk.stage,
                                inflight::PipelineFrameSlot::Ten,
                                rows,
                                pools,
                                ring,
                                frames,
                                frame_index,
                            )?;
                            let products = product_writers.settle(
                                walk.frame_cdfs,
                                walk.ccso_grid,
                                walk.segment_ids,
                            );
                            (
                                slot,
                                inter_core,
                                products,
                                frames.settle_motion(walk.motion_field)?,
                            )
                        }
                    }
                };
                let (inter_slot, inter_core, products, motion_field) = decoded;
                let inter_display_grain =
                    film_grain_slots.active_for_core(&inter_core, inter_envelope.offset)?;
                output_effect_state
                    .observe_suffix(suffix_after_candidate(obus, next_candidate)?)?;
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
                    frame_cdfs: products.frame_cdfs,
                    motion_field,
                    ccso_params: inter_core.ccso_params.clone(),
                    ccso_grid: products.ccso_grid,
                    segment_ids: products.segment_ids,
                    frame_rate_numerator: inter_frame_rate.numerator,
                    frame_rate_denominator: inter_frame_rate.denominator,
                    props: stream.props(),
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                frames.push(inter_frame)?;
                retained_frame_bytes = next_retained_frame_bytes;
                let inter_hint = inter_update.order_hint;
                let inter_implicit = inter_core.implicit_output_frame == Some(true);
                let inter_immediate = inter_core.immediate_output_frame == Some(true);
                let inter_key_or_switch = is_key_or_switch(&inter_core);
                scheduler.refresh(
                    inter_update.refresh_frame_flags,
                    frame_index,
                    inter_hint,
                    inter_implicit,
                    inter_key_or_switch,
                );
                charge_emitted_outputs(
                    options,
                    frames,
                    &scheduler,
                    &mut emission_queue,
                    scheduler.newly(),
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
                    scheduler.on_immediate(frame_index, inter_hint);
                    charge_emitted_outputs(
                        options,
                        frames,
                        &scheduler,
                        &mut emission_queue,
                        scheduler.newly(),
                        &mut emit,
                    )?;
                }
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        frames,
                        &reference,
                        &mut scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, scheduler.emitted_count) {
                    break;
                }
            }
            ObuType::ClosedLoopKey | ObuType::OpenLoopKey => {
                let starts_new_sequence = next_candidate.obu_type() == ObuType::ClosedLoopKey;
                let mut obu_buffer = recycle(core::mem::take(obu_storage));
                let obus = RuntimeStream::obus(&unit, &mut obu_buffer)?;
                let (key_sequence_envelope, key_prefix_obus, key_envelope) = if starts_new_sequence
                {
                    let (sequence_envelope, prefix, frame) =
                        ivf_key_frame_unit(obus, next_candidate)?;
                    (Some(sequence_envelope), prefix, frame)
                } else {
                    let (prefix, frame) = stream.inter_envelope(obus, next_candidate)?;
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
                        frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        &mut emit,
                    )?;
                    if output_frame_limit_reached(options, scheduler.emitted_count) {
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
                            let state = reference.state_eight(frames)?;
                            parse_olk_core_with_effects(
                                key_envelope,
                                &key_sequence,
                                &state,
                                &output_effect_state,
                                first_picture_in_tu,
                            )?
                        }
                        BitDepthIdc::Ten => {
                            let state = reference.state_ten(frames)?;
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
                        frames,
                        &scheduler,
                        &mut emission_queue,
                        &flushed,
                        &mut emit,
                    )?;
                    reference = key_reference;
                    if !retain_decoded_frames {
                        reclaim_unowned_frames(
                            frames,
                            &reference,
                            &mut scheduler,
                            &emission_queue,
                            ring,
                            &mut retained_frame_bytes,
                        )?;
                    }
                    if output_frame_limit_reached(options, scheduler.emitted_count) {
                        break;
                    }
                }

                sequence = key_sequence;
                shared_sequence = None;
                output_effect_state
                    .observe_suffix(suffix_after_candidate(obus, next_candidate)?)?;
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
                    rows,
                    pools,
                    ring,
                    frames,
                    Some((
                        (&mut *split_eight, &mut *split_ten),
                        &unit,
                        &mut shared_sequence,
                    )),
                    frame_index,
                    unit.view(),
                    options,
                    next_candidate,
                    key_envelope,
                    key_core.clone(),
                    &sequence,
                    frame_rate,
                    key_display_grain,
                    key_user_qm,
                    key_output_effects,
                    stream.props(),
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
                frames.push(key_frame)?;
                retained_frame_bytes = next_retained_frame_bytes;
                let key_hint = key_update.order_hint;
                let key_implicit = key_core.implicit_output_frame == Some(true);
                let key_immediate = key_core.immediate_output_frame == Some(true);
                scheduler.refresh(
                    key_update.refresh_frame_flags,
                    frame_index,
                    key_hint,
                    key_implicit,
                    true,
                );
                charge_emitted_outputs(
                    options,
                    frames,
                    &scheduler,
                    &mut emission_queue,
                    scheduler.newly(),
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
                    scheduler.on_immediate(frame_index, key_hint);
                    charge_emitted_outputs(
                        options,
                        frames,
                        &scheduler,
                        &mut emission_queue,
                        scheduler.newly(),
                        &mut emit,
                    )?;
                }
                if !retain_decoded_frames {
                    reclaim_unowned_frames(
                        frames,
                        &reference,
                        &mut scheduler,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                }
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, scheduler.emitted_count) {
                    break;
                }
            }
            _ => {
                return Err(unsupported(
                    "non_frame_candidate_in_frame_loop",
                    None,
                    "internal invariant violation: non-frame-candidate obu reached the frame decode loop",
                ));
            }
        }
    }

    if !output_frame_limit_reached(options, scheduler.emitted_count) {
        let flushed = scheduler.flush_all();
        charge_emitted_outputs(
            options,
            frames,
            &scheduler,
            &mut emission_queue,
            &flushed,
            &mut emit,
        )?;
        emission_queue.flush(frames, &mut emit)?;
        if !retain_decoded_frames {
            ring.harvest_all(decode_scratch_eight, decode_scratch_ten);
            reclaim_unowned_frames(
                frames,
                &reference,
                &mut scheduler,
                &emission_queue,
                ring,
                &mut retained_frame_bytes,
            )?;
        }
    } else {
        emission_queue.flush(frames, &mut emit)?;
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
pub(crate) enum TileFactsKind {
    Intra,
    Inter,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_tile_plan_with<'payload>(
    candidate: &DecodePlannedObu,
    bytes: SourceBytes<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<&Arc<FrameCdfSubset>>,
    scratch: &mut crate::bitstream::tile_payload::TilePayloadScratch,
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
    let candidates = core::iter::once(candidate.clone()).chain(
        crate::bitstream::stream_plan::tile_group_continuations(bytes, candidate),
    );
    let group_count = candidates.clone().count();
    let recorded_header = record_frame_header(envelope, core, group_count > 1)?;
    let mut merged: Option<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> = None;
    for (group_index, group_candidate) in candidates.enumerate() {
        if group_index != 0 {
            core::mem::swap(
                &mut scratch.work_units,
                &mut scratch.continuation_work_units,
            );
        }
        let group_envelope = planned_envelope(bytes, &group_candidate)?;
        let group_facts = if group_index == 0 {
            facts
        } else {
            facts.with_tile_group_structure_start_bits(continuation_structure_start_bits(
                group_envelope,
                recorded_header.as_ref().ok_or_else(|| {
                    unsupported_at(
                        "frame_header_copy_source_truncated",
                        envelope.offset,
                        "first tile-group frame header was not recorded for continuation validation",
                    )
                })?,
            )?)
        };
        let mut input = FrameCandidateTileBoundaryInput::new(
            &group_candidate,
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
        let group_plan =
            crate::bitstream::tile_payload::plan_derived_tile_payload_boundary_with_scratch(
                &input, scratch,
            )
            .map_err(decode_tile_boundary_error)?;
        if let Some(plan) = merged.as_mut() {
            plan.append_continuation(group_plan, &mut scratch.continuation_work_units)
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

fn planned_envelope<'a>(
    bytes: SourceBytes<'a>,
    planned: &DecodePlannedObu,
) -> Result<ObuEnvelope<'a>> {
    let start = planned.offset().get();
    let payload = start
        .checked_add(u64::from(planned.header().header_size_bytes))
        .zip(start.checked_add(u64::from(planned.size())))
        .and_then(|(payload_start, end)| bytes.get(payload_start, end))
        .ok_or_else(|| {
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

/// Checks the first tile group's frame header and, only when continuation
/// groups will compare against it, keeps a copy of its bits.
fn record_frame_header(
    envelope: ObuEnvelope<'_>,
    core: &FrameHeaderCore,
    continuations: bool,
) -> Result<Option<RecordedFrameHeaderBits>> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if reader.read_bit().ok() != Some(1) {
        return Err(unsupported_at(
            "missing_first_tile_group",
            envelope.offset,
            "coded frame must begin with is_first_tile_group equal to 1",
        ));
    }
    let truncated = || {
        unsupported_at(
            "frame_header_copy_source_truncated",
            envelope.offset,
            "first tile-group frame header could not be recorded for continuation validation",
        )
    };
    if !continuations {
        return if reader.remaining_bits() < core.consumed_bits {
            Err(truncated())
        } else {
            Ok(None)
        };
    }
    RecordedFrameHeaderBits::record(&mut reader, core.consumed_bits)
        .map(Some)
        .map_err(|_| truncated())
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
    candidate: &DecodePlannedObu,
    bytes: SourceBytes<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    let mut scratch = crate::bitstream::tile_payload::TilePayloadScratch::default();
    derive_tile_plan_with(
        candidate,
        bytes,
        sequence,
        core,
        options,
        TileFactsKind::Intra,
        None,
        &mut scratch,
    )
}
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
