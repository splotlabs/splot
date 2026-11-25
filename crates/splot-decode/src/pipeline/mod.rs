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
use splot_core::headers::frame::{FrameHeaderCore, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader};
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::types::{EmbeddedLayerId, ObuType};
#[cfg(test)]
use splot_recon::DecodedFrame;

use crate::DecodeIvfFrameContext as IvfFrameContext;
use crate::bitstream::byte_stream::{InputScratch, SourceBytes, UnitSource, recycle};
use crate::bitstream::tile_payload::{GeneralIntraBlockModeError, GeneralIntraResidualError};
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
pub(crate) use frame_tasks::DecoderRowScheduler;
use frame_tasks::SplitSample;
pub(crate) mod inflight;
pub(crate) mod output_effects;
mod output_schedule;
mod runtime_support;
mod stream_schedule;
mod tile_plan;
mod xlayer_state;

use frame_lifecycle::*;
pub(crate) use frame_lifecycle::{
    ActiveFilmGrain, PipelineDecodedFrame, PipelineFrame, PipelineFrameRate, deblock_quant_deltas,
    derive_visible_luma_rect, effective_allow_screen_content_tools, frame_ref_update_from_core,
    is_key_or_switch, parse_frame_core, parse_frame_core_with_reference, parse_sequence,
};
use output_effects::{FrameOutputEffects, OutputEffectState};
use output_schedule::*;
pub(crate) use runtime_support::{
    ensure_runtime_limits, malformed_tile_payload, unsupported, unsupported_at,
    unsupported_feature_at, unsupported_with_spec,
};
#[cfg(test)]
pub(crate) use stream_schedule::ivf_inter_envelope;
#[cfg(test)]
pub(crate) use stream_schedule::require_minimal_obu_order;
use stream_schedule::*;
#[cfg(test)]
use xlayer_state::{InBandLongTermFrame, InBandLongTermPrelude};
use xlayer_state::{XLayerState, XLayerTable};

pub(crate) const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
pub(crate) const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";

#[cfg(test)]
pub(crate) fn decode_frame_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<PipelineDecodedFrame> {
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
) -> Result<Vec<PipelineDecodedFrame>> {
    decode_frames_from_plan_on_threads(bytes, options, 1)
}

/// [`decode_frames_from_plan`] on a row scheduler of `threads` workers, with
/// one frame in flight.
#[cfg(test)]
pub(crate) fn decode_frames_from_plan_on_threads(
    bytes: &[u8],
    options: &DecodeOptions,
    threads: usize,
) -> Result<Vec<PipelineDecodedFrame>> {
    collect_frames_from_plan(bytes, options, threads, PipelineFrame::ready_frame)
}

/// Decodes `bytes` through [`emit_frames`] and keeps `collect` of each emitted
/// frame. The session detaches held frames, as the streaming decoder does.
#[cfg(test)]
pub(crate) fn collect_frames_from_plan<R: Send>(
    bytes: &[u8],
    options: &DecodeOptions,
    threads: usize,
    mut collect: impl FnMut(&PipelineFrame) -> Result<R> + Send,
) -> Result<Vec<R>> {
    let threads = NonZeroUsize::new(threads).unwrap_or(NonZeroUsize::MIN);
    let backend = splot_core::simd::resolve_backend(splot_core::simd::SimdPolicy::Auto)?;
    let session =
        DecodeSession::detaching_held(NonZeroUsize::MIN, row_scheduler(threads, backend)?, backend);
    let mut collected = Vec::new();
    splot_parallel::run_on_thread("splot-driver", || {
        let mut source = crate::bitstream::byte_stream::ReaderUnits::new(bytes, options.limits());
        emit_frames(
            &mut source,
            options,
            &session,
            |_| Ok(()),
            |frame| {
                collected.push(collect(frame)?);
                Ok(())
            },
        )
    })??;
    Ok(collected)
}

/// Decodes `bytes` serially on the driver thread while capturing per-frame BRU
/// mode maps and pre/post BRU loop-restoration source-block spans.
#[cfg(test)]
pub(crate) fn collect_bru_lr_audit(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<(
    Vec<Arc<FrameHeaderCore>>,
    Vec<crate::filters::wienerns_lr::recon::BruLrFrameAudit>,
)> {
    let backend = splot_core::simd::resolve_backend(splot_core::simd::SimdPolicy::Auto)?;
    let session = DecodeSession::detaching_held(
        NonZeroUsize::MIN,
        row_scheduler(NonZeroUsize::MIN, backend)?,
        backend,
    );
    let (headers, audits) = splot_parallel::run_on_thread("splot-driver", || {
        crate::filters::wienerns_lr::recon::with_bru_lr_audit(|| {
            let mut source =
                crate::bitstream::byte_stream::ReaderUnits::new(bytes, options.limits());
            let mut headers = Vec::new();
            emit_frames(
                &mut source,
                options,
                &session,
                |_| Ok(()),
                |frame| {
                    if let Some(core) = frame.ccso_header.as_ref() {
                        headers.push(Arc::clone(core));
                    }
                    let _ = frame.ready_frame()?;
                    Ok(())
                },
            )?;
            Result::Ok(headers)
        })
    })?;
    Ok((headers?, audits))
}

/// Decodes every temporal unit of `source` and passes each displayed frame
/// to `emit` in display order.
pub(crate) fn emit_frames(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &DecodeSession,
    preflight: impl FnOnce(Option<IvfHeader>) -> Result<()> + Send,
    mut emit: impl FnMut(&PipelineFrame) -> Result<()> + Send,
) -> Result<()> {
    let mut preflight = Some(preflight);
    let mut preflight = |header| match preflight.take() {
        Some(preflight) => preflight(header),
        None => Ok(()),
    };
    let pipeline_capacity = session
        .frame_delay()
        .min(NonZeroUsize::new(session.threads()).unwrap_or(NonZeroUsize::MIN));
    let mut retained = session.take(pipeline_capacity);
    retained.begin();
    let decoded = drive_frames(
        source,
        options,
        &mut retained,
        session.rows(),
        session.backend(),
        &mut preflight,
        &mut emit,
    );
    session.keep(retained);
    decoded
}

/// Retires unowned frames into their physical entries and returns their planes.
/// Shared samples retire only when another tracked frame accounts for their
/// bytes; otherwise subtracting them would undercount `MaxReferenceStoreBytes`.
/// Every call rescans readers skipped by earlier reclamation passes.
pub(in crate::pipeline) trait FrameRetention {
    fn retains(&self, frame_index: usize) -> bool;
    fn forget(&mut self, frame_index: usize);
}

impl FrameRetention for XLayerTable {
    fn retains(&self, frame_index: usize) -> bool {
        XLayerTable::retains(self, frame_index)
    }

    fn forget(&mut self, frame_index: usize) {
        XLayerTable::forget(self, frame_index);
    }
}

fn reclaim_unowned_frames(
    frames: &mut FrameStore,
    xlayers: &mut XLayerTable,
    emission: &output_schedule::EmissionQueue,
    ring: &mut inflight::InflightRing,
    retained_frame_bytes: &mut u64,
) -> Result<()> {
    reclaim_unowned_frames_with(frames, xlayers, emission, ring, retained_frame_bytes)
}

pub(in crate::pipeline) fn reclaim_unowned_frames_with(
    frames: &mut FrameStore,
    retention: &mut impl FrameRetention,
    emission: &output_schedule::EmissionQueue,
    ring: &mut inflight::InflightRing,
    retained_frame_bytes: &mut u64,
) -> Result<()> {
    for slot_index in 0..frames.entries.len() {
        let frame_index = frames.entries[slot_index].index;
        let Some(frame) = frames.entries[slot_index].frame.as_ref() else {
            continue;
        };
        if retention.retains(frame_index)
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
        retention.forget(frame_index);
        frames.entries[slot_index].retired = Some(ring.keep_frame_planes(frame.frame));
    }
    Ok(())
}

/// Runs the `max_reference_store_bytes` admission `check` of the next frame.
/// While it fails, collects the oldest in-flight frame and reclaims what that
/// releases, so a deep pipeline shrinks to the limit instead of failing. The
/// error stands once nothing is in flight and every pool work has finished.
#[expect(clippy::too_many_arguments)]
fn admit_within_reference_store(
    mut check: impl FnMut(&XLayerTable, u64) -> Result<()>,
    options: &DecodeOptions,
    rows: Option<&DecoderRowScheduler>,
    pools: &mut frame_tasks::RowPools,
    ring: &mut inflight::InflightRing,
    frames: &mut FrameStore,
    xlayers: &mut XLayerTable,
    emission: &mut output_schedule::EmissionQueue,
    emit: &mut dyn FnMut(&PipelineFrame) -> Result<()>,
    retained_frame_bytes: &mut u64,
) -> Result<()> {
    let mut drained = false;
    loop {
        match check(xlayers, *retained_frame_bytes) {
            Err(crate::DecodeError::Limit { .. }) if !drained => {}
            outcome => return outcome,
        }
        if !ring.harvest_oldest(pools) {
            if let Some(rows) = rows {
                pools.drain(rows);
            }
            drained = true;
        }
        charge_emitted_outputs(
            options,
            frames,
            xlayers.total_emitted_count(),
            emission,
            &[],
            emit,
        )?;
        pools.retire_idle();
        reclaim_unowned_frames(frames, xlayers, emission, ring, retained_frame_bytes)?;
    }
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
    reference: &inter::ReferenceMetadata,
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

fn parse_inter_core_with_effects(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &inter::ReferenceMetadata,
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
/// Runs `$body` with `$t` bound to the sample type of `$sequence`: the one
/// place the driver picks a bit depth.
macro_rules! with_sample_bit_depth {
    ($bit_depth:expr, $t:ident, $body:expr) => {
        match $bit_depth {
            BitDepthIdc::Eight => {
                type $t = u8;
                $body
            }
            BitDepthIdc::Ten => {
                type $t = u16;
                $body
            }
        }
    };
}

macro_rules! with_sample {
    ($sequence:expr, $t:ident, $body:expr) => {
        with_sample_bit_depth!($sequence.general.bit_depth_idc, $t, $body)
    };
}

#[cfg(test)]
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
    let mut pools = frame_tasks::RowPools::new(1, 1, &buffers);
    let mut ring = inflight::InflightRing::new(NonZeroUsize::MIN, buffers, 1);
    let mut frames = FrameStore::new(ring.capacity());
    let backend = splot_core::simd::resolve_backend(splot_core::simd::SimdPolicy::Auto)?;
    let decoded = decode_key_frame_with_effects(
        None,
        backend,
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
    ring.harvest_all(&mut pools);
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

#[expect(clippy::too_many_arguments)]
fn walk_frame_with_backend<T: splot_recon::ReconSample>(
    backend: splot_core::simd::ResolvedBackend,
    scratch: &mut inter::InterDecodeScratch<T>,
    candidate: &DecodePlannedObu,
    bytes: crate::bitstream::byte_stream::SourceBytes<'_>,
    frame_envelope: ObuEnvelope<'_>,
    core: Arc<FrameHeaderCore>,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    setup: &frame_engine::FrameSetup<'_, T>,
    bit_depth: splot_recon::BitDepth,
    products: &mut inter::FrameProductWriters,
) -> Result<frame_engine::finish::FrameWalk<T>> {
    struct WalkLaunch<'a, T: splot_recon::ReconSample> {
        scratch: &'a mut inter::InterDecodeScratch<T>,
        candidate: &'a DecodePlannedObu,
        bytes: crate::bitstream::byte_stream::SourceBytes<'a>,
        frame_envelope: ObuEnvelope<'a>,
        core: Arc<FrameHeaderCore>,
        sequence: &'a SequenceHeader,
        options: &'a DecodeOptions,
        setup: &'a frame_engine::FrameSetup<'a, T>,
        bit_depth: splot_recon::BitDepth,
        products: &'a mut inter::FrameProductWriters,
    }
    impl<T: splot_recon::ReconSample>
        splot_core::simd::SimdScope<Result<frame_engine::finish::FrameWalk<T>>>
        for WalkLaunch<'_, T>
    {
        fn run<B: splot_core::simd::SimdBackend>(
            self,
            backend: B,
        ) -> Result<frame_engine::finish::FrameWalk<T>> {
            frame_engine::walk_frame(
                backend.narrow(),
                self.scratch,
                self.candidate,
                self.bytes,
                self.frame_envelope,
                self.core,
                self.sequence,
                self.options,
                self.setup,
                self.bit_depth,
                self.products,
            )
        }
    }
    backend.enter(WalkLaunch {
        scratch,
        candidate,
        bytes,
        frame_envelope,
        core,
        sequence,
        options,
        setup,
        bit_depth,
        products,
    })
}

#[expect(clippy::too_many_arguments)]
fn decode_key_frame_with_effects(
    rows: Option<&DecoderRowScheduler>,
    backend: splot_core::simd::ResolvedBackend,
    pools: &mut frame_tasks::RowPools,
    ring: &mut inflight::InflightRing,
    frames: &mut FrameStore,
    split: Option<(
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
    ring.reserve(pools);
    let _user_qm_scope = crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
    let core = frames.share_core(core)?;
    if let (Some(rows), Some((unit, shared_sequence))) = (rows, split)
        && inter::splittable_inter_frame(frame_envelope.header.obu_type, &core)
    {
        let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
            frame_engine::intra::build_frame_qm_levels(&core),
        );
        let (slot, core, products, motion_field) = with_sample!(sequence, T, {
            push_split_inter::<T>(
                pools,
                rows,
                ring,
                frames,
                options,
                sequence,
                shared_sequence,
                candidate,
                unit,
                core,
                empty_key_reference(frame_envelope.offset)?,
                frame_index,
            )?
        });
        let frame_rate = output_effects.frame_rate(frame_rate);
        return Ok(PipelineFrame {
            frame: slot,
            display_grain,
            output_effects,
            frame_cdfs: products.frame_cdfs,
            motion_field,
            ccso_header: Some(core),
            ccso_grid: products.ccso_grid,
            segment_ids: products.segment_ids,
            frame_rate,
            extended_layer_id: frame_envelope.header.extended_layer_id,
            embedded_layer_id: frame_envelope.header.embedded_layer_id,
            temporal_layer_id: frame_envelope.header.temporal_layer_id,
            props,
        });
    }
    let mut product_writers = frames.reserve_products()?;
    let (frame, frame_cdfs, ccso_header, ccso_grid, segment_ids, motion_field) =
        with_sample!(sequence, T, {
            let walk = walk_frame_with_backend::<T>(
                backend,
                &mut T::pools(pools).scratch,
                candidate,
                bytes,
                frame_envelope,
                core,
                sequence,
                options,
                &frame_engine::FrameSetup::Intra,
                T::BIT_DEPTH,
                &mut product_writers,
            )?;
            let ccso_header = Some(walk.core);
            let frame = inflight::settle_walk_stage(
                backend,
                walk.stage,
                T::erase,
                rows,
                pools,
                ring,
                frames,
                frame_index,
            )?;
            (
                frame,
                walk.frame_cdfs,
                ccso_header,
                walk.ccso_grid,
                walk.segment_ids,
                walk.motion_field,
            )
        });
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
        ccso_header,
        ccso_grid,
        segment_ids,
        frame_rate,
        extended_layer_id: frame_envelope.header.extended_layer_id,
        embedded_layer_id: frame_envelope.header.embedded_layer_id,
        temporal_layer_id: frame_envelope.header.temporal_layer_id,
        props,
    })
}

struct RowSchedulerLaunch {
    threads: NonZeroUsize,
}

impl splot_core::simd::SimdScope<Result<Arc<DecoderRowScheduler>>> for RowSchedulerLaunch {
    fn run<B: splot_core::simd::SimdBackend>(self, backend: B) -> Result<Arc<DecoderRowScheduler>> {
        spawn_row_scheduler(self.threads, backend.narrow())
    }
}

fn spawn_row_scheduler<B: splot_core::simd::SimdBackend>(
    threads: NonZeroUsize,
    backend: B,
) -> Result<Arc<DecoderRowScheduler>> {
    Ok(Arc::new(DecoderRowScheduler::new_with(
        threads,
        move |worker_loop| run_typed_worker(backend, worker_loop),
    )?))
}

fn run_typed_worker<B: splot_core::simd::SimdBackend>(
    backend: B,
    worker_loop: splot_parallel::WorkerLoop<frame_tasks::DecoderWork>,
) {
    worker_loop.run(|work, claim, worker| {
        backend.vectorize(|| work.run_with_backend(backend, claim, worker));
    });
}

/// Starts the typed row scheduler of a context with more than one worker thread.
///
/// # Errors
/// Returns [`crate::DecodeError::Pool`] when a worker thread cannot start.
pub(crate) fn row_scheduler(
    threads: NonZeroUsize,
    backend: splot_core::simd::ResolvedBackend,
) -> Result<Option<Arc<DecoderRowScheduler>>> {
    if threads.get() <= 1 {
        return Ok(None);
    }
    Ok(Some(backend.enter(RowSchedulerLaunch { threads })?))
}

/// Runs one decode over its retained storage, and resolves the run's outcome
/// against the filter phases the ring collected.
///
/// A filter-phase failure outranks the frame loop's own error: serial decode
/// would have run that frame's filters before reaching the later error, so the
/// lowest-indexed collected failure is the one the caller sees.
fn drive_frames(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    retained: &mut frame_store::RetainedDecode,
    rows: Option<&DecoderRowScheduler>,
    backend: splot_core::simd::ResolvedBackend,
    preflight: &mut dyn FnMut(Option<IvfHeader>) -> Result<()>,
    emit: &mut dyn FnMut(&PipelineFrame) -> Result<()>,
) -> Result<()> {
    let frame_store::RetainedDecode {
        ring,
        pools,
        frames,
        input_scratch,
        ..
    } = retained;
    let decoded = decode_frames_in_order(
        source,
        options,
        preflight,
        emit,
        rows,
        backend,
        ring,
        pools,
        frames,
        input_scratch,
    );
    ring.harvest_all(pools);
    if let Some(rows) = rows {
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

#[expect(clippy::too_many_arguments)]
fn push_split_inter<T: SplitSample>(
    pools: &mut frame_tasks::RowPools,
    rows: &DecoderRowScheduler,
    ring: &mut inflight::InflightRing,
    frames: &mut FrameStore,
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
    let frame_tasks::SamplePools { scratch, split, .. } = T::pools(pools);
    let (slot_index, parse_progress) = split.reserve(rows, options);
    let records = scratch.take_frame_filter_records();
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
            reference: Box::new(reference),
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
    )?;
    Ok((slot, core, publications, motion))
}

fn drain_pending_cmvs_flushed(
    options: &DecodeOptions,
    frames: &FrameStore,
    xlayers: &mut XLayerTable,
    emission_queue: &mut output_schedule::EmissionQueue,
    emit: &mut dyn FnMut(&PipelineFrame) -> Result<()>,
) -> Result<()> {
    let flushed = xlayers.take_pending_cmvs_flushed();
    if flushed.is_empty() {
        return Ok(());
    }
    charge_emitted_outputs(
        options,
        frames,
        xlayers.total_emitted_count(),
        emission_queue,
        &flushed,
        emit,
    )
}

fn global_lcr_activation(
    xlayers: &XLayerTable,
    unit: &crate::bitstream::byte_stream::UnitBytes,
    prefix: &[ObuEnvelope<'_>],
    tu_has_clk: bool,
) -> Result<Option<ByteOffset>> {
    if !tu_has_clk
        || !xlayers.has_active_cmvs()
        || prefix
            .iter()
            .any(|env| env.header.obu_type == ObuType::Msdo)
    {
        return Ok(None);
    }
    let mut obus = Vec::new();
    crate::bitstream::byte_stream::runtime_obus(unit.view(), &mut obus)?;
    xlayers.global_lcr_activation_in_tu(&obus)
}

#[expect(clippy::too_many_arguments)]
fn observe_global_and_cmvs(
    xlayers: &mut XLayerTable,
    prefix_obus: &[ObuEnvelope<'_>],
    unit: &crate::bitstream::byte_stream::UnitBytes,
    checked_rap_flags: Option<(bool, bool)>,
    options: &DecodeOptions,
    frames: &FrameStore,
    emission_queue: &mut output_schedule::EmissionQueue,
    emit: &mut dyn FnMut(&PipelineFrame) -> Result<()>,
) -> Result<()> {
    let check_cmvs_end = xlayers.has_active_cmvs()
        && prefix_obus
            .iter()
            .any(|env| env.header.obu_type == ObuType::TemporalDelimiter);
    let (tu_has_rap, tu_has_clk) = if check_cmvs_end
        || prefix_obus.iter().any(|env| {
            env.header.obu_type == ObuType::Msdo && env.header.extended_layer_id.is_global()
        }) {
        if let Some(flags) = checked_rap_flags {
            flags
        } else {
            unit_rap_flags(unit)?
        }
    } else {
        (false, false)
    };
    let global_lcr = global_lcr_activation(xlayers, unit, prefix_obus, tu_has_clk)?;
    xlayers.observe_global_obus(prefix_obus, false, tu_has_rap, tu_has_clk, global_lcr)?;
    drain_pending_cmvs_flushed(options, frames, xlayers, emission_queue, emit)
}

#[expect(clippy::too_many_arguments)]
fn decode_frames_in_order(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    preflight: &mut dyn FnMut(Option<IvfHeader>) -> Result<()>,
    emit: &mut dyn FnMut(&PipelineFrame) -> Result<()>,
    rows: Option<&DecoderRowScheduler>,
    backend: splot_core::simd::ResolvedBackend,
    ring: &mut inflight::InflightRing,
    pools: &mut frame_tasks::RowPools,
    frames: &mut FrameStore,
    input_scratch: &mut InputScratch,
) -> Result<()> {
    options.validate_selections()?;
    let InputScratch {
        records,
        obus: obu_storage,
    } = input_scratch;
    let mut stream = RuntimeStream::new(source, records, options);
    let Some(mut leading_unit) = stream.first_unit()? else {
        return Ok(());
    };
    let mut xlayers = XLayerTable::new();
    let mut skipped_unselected_leading = false;
    let mut key_candidate = stream.first_candidate()?;
    if key_candidate.is_err() {
        let mut scan_buffer = recycle(core::mem::take(obu_storage));
        if stream.unit_only_filtered_unselected_xlayer(&leading_unit, &mut scan_buffer) {
            skipped_unselected_leading = true;
            let (tu_has_rap, tu_has_clk) = unit_rap_flags(&leading_unit)?;
            let filtered = stream.obus(&leading_unit, &mut scan_buffer)?;
            xlayers.observe_global_obus(filtered, true, tu_has_rap, tu_has_clk, None)?;
            *obu_storage = recycle(scan_buffer);
            if let Some((candidate, unit)) = stream.candidate(
                obu_storage,
                &mut || Ok(()),
                &mut |unit, skipped, has_rap, has_clk| {
                    let global_lcr = global_lcr_activation(&xlayers, unit, skipped, has_clk)?;
                    xlayers.observe_skipped(skipped, has_rap, has_clk, global_lcr)
                },
            )? {
                leading_unit = unit;
                key_candidate = Ok(candidate);
            }
        } else {
            *obu_storage = recycle(scan_buffer);
        }
    }
    let mut leading_buffer = recycle(core::mem::take(obu_storage));
    let key_candidate = if skipped_unselected_leading && key_candidate.is_err() {
        *obu_storage = recycle(leading_buffer);
        key_candidate?
    } else {
        let leading_obus = stream.leading_obus(&leading_unit, &mut leading_buffer)?;
        if let Err(error) = require_leading_frame_unit(leading_obus)
            && !stream.has_unfiltered_multistream_rap(&leading_unit)?
        {
            return Err(error);
        }
        preflight(stream.ivf_header())?;
        *obu_storage = recycle(leading_buffer);
        key_candidate?
    };
    let frame_rate = stream.frame_rate();
    let mut pending_first = Some((key_candidate, leading_unit));
    let mut emission_queue = output_schedule::EmissionQueue::default();
    let mut retained_frame_bytes = 0;

    loop {
        if !frames.has_space() {
            ring.harvest_all(pools);
            emission_queue.flush(frames, emit)?;
            let mut reclaim = || {
                pools.retire_idle();
                reclaim_unowned_frames(
                    frames,
                    &mut xlayers,
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
        let candidate = if let Some(first) = pending_first.take() {
            Some(first)
        } else {
            let found = stream.candidate(
                obu_storage,
                &mut || emission_queue.flush(frames, emit),
                &mut |unit, skipped, has_rap, has_clk| {
                    let global_lcr = global_lcr_activation(&xlayers, unit, skipped, has_clk)?;
                    xlayers.observe_skipped(skipped, has_rap, has_clk, global_lcr)
                },
            )?;
            drain_pending_cmvs_flushed(options, frames, &mut xlayers, &mut emission_queue, emit)?;
            if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                break;
            }
            found
        };
        let Some((next_candidate, unit)) = candidate else {
            break;
        };
        let next_candidate = &next_candidate;
        let candidate_xlayer = next_candidate.header().extended_layer_id;
        if xlayers.is_discarding_leading_frames(candidate_xlayer)
            && next_candidate.obu_type().is_leading_frame()
        {
            let mut obu_buffer = recycle(core::mem::take(obu_storage));
            let obus = stream.obus(&unit, &mut obu_buffer)?;
            let (prefix_obus, _) = stream.inter_envelope(obus, next_candidate)?;
            observe_global_and_cmvs(
                &mut xlayers,
                prefix_obus,
                &unit,
                stream.checked_rap_flags(),
                options,
                frames,
                &mut emission_queue,
                emit,
            )?;
            xlayers.note_celu_order(candidate_xlayer, next_candidate.offset())?;
            xlayers.observe_other_non_global_prefix(prefix_obus, candidate_xlayer)?;
            if let Some(layer) = xlayers.get_mut(candidate_xlayer) {
                layer.begin_discarded_leading_frame();
            }
            *obu_storage = recycle(obu_buffer);
            if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                break;
            }
            continue;
        }
        match next_candidate.obu_type() {
            ObuType::LeadingSef | ObuType::RegularSef => {
                let mut obu_buffer = recycle(core::mem::take(obu_storage));
                let obus = stream.obus(&unit, &mut obu_buffer)?;
                let (sef_prefix_obus, sef_envelope) =
                    stream.inter_envelope(obus, next_candidate)?;
                let sef_emit_allowed = stream.layer_filter().is_layer_output_for(
                    sef_envelope.header.extended_layer_id,
                    sef_envelope.header.embedded_layer_id,
                    sef_envelope.header.temporal_layer_id,
                );
                let ivf_frame_index = next_candidate.ivf_frame().map(IvfFrameContext::frame_index);
                observe_global_and_cmvs(
                    &mut xlayers,
                    sef_prefix_obus,
                    &unit,
                    stream.checked_rap_flags(),
                    options,
                    frames,
                    &mut emission_queue,
                    emit,
                )?;
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                    *obu_storage = recycle(obu_buffer);
                    break;
                }
                xlayers.note_celu_order(candidate_xlayer, sef_envelope.offset)?;
                xlayers.observe_other_non_global_prefix(sef_prefix_obus, candidate_xlayer)?;
                if !xlayers.contains(candidate_xlayer) {
                    let (tu_has_rap, _) = unit_rap_flags(&unit)?;
                    if xlayers.should_skip_pre_rap_frame(candidate_xlayer, tu_has_rap) {
                        *obu_storage = recycle(obu_buffer);
                        continue;
                    }
                }
                let Some(layer) = xlayers.get_mut(candidate_xlayer) else {
                    return Err(unsupported_at(
                        "missing_random_access_frame",
                        sef_envelope.offset,
                        "decode runtime requires a closed-loop key, open-loop key, or RAS random-access frame",
                    ));
                };
                XLayerTable::apply_non_global_prefix(
                    sef_prefix_obus,
                    candidate_xlayer,
                    &layer.sequence,
                    &mut layer.output_effect_state,
                    &mut layer.film_grain_slots,
                )?;
                let first_picture_in_tu = layer.begin_coded_picture();
                let flushed = layer
                    .scheduler
                    .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                layer
                    .reference
                    .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                let sef_order_hint_bits = layer
                    .sequence
                    .inter
                    .as_ref()
                    .map_or(0, |inter| inter.order_hint_bits);
                let sef_core = {
                    let bit_depth = layer.sequence.general.bit_depth_idc;
                    with_sample_bit_depth!(bit_depth, T, {
                        let state = layer.reference.state::<T>(frames)?;
                        parse_inter_core_with_effects(
                            sef_envelope,
                            &layer.sequence,
                            &state,
                            &layer.output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?
                    })
                };
                xlayers.check_tu_doh(
                    candidate_xlayer,
                    sef_order_hint_bits,
                    sef_core.display_order_hint(),
                    true,
                    sef_envelope.offset,
                )?;
                let total_emitted = xlayers.total_emitted_count();
                if sef_emit_allowed {
                    let next_output_frame_count =
                        checked_add(DecodeLimitName::MaxOutputFrames, total_emitted as u64, 1)?;
                    ensure_output_frame_count_limit(options.limits(), next_output_frame_count)?;
                }
                charge_emitted_outputs(
                    options,
                    frames,
                    total_emitted,
                    &mut emission_queue,
                    &flushed,
                    emit,
                )?;
                if output_frame_limit_reached(options, total_emitted) {
                    *obu_storage = recycle(obu_buffer);
                    break;
                }
                let display_grain = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            sef_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let _ = layer.output_effect_state.prepare_frame(
                        sef_envelope,
                        &sef_core,
                        &layer.sequence,
                        first_picture_in_tu,
                        ivf_frame_index,
                    )?;
                    layer.film_grain_slots.active_for_core(
                        sef_envelope,
                        &sef_core,
                        &layer.sequence,
                        sef_envelope.offset,
                    )?
                };
                xlayers.observe_suffix_for_xlayer(
                    suffix_after_candidate(obus, next_candidate)?,
                    candidate_xlayer,
                )?;
                let (sef_frame, ordering, output_mlayer) = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            sef_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let output_effects = layer
                        .output_effect_state
                        .finish_frame(sef_envelope.header.embedded_layer_id);
                    let output_rate = output_effects.frame_rate(frame_rate);
                    let slot = sef_core.frame_to_show_map_idx.ok_or_else(|| {
                        unsupported_at(
                            "sef_missing_frame_to_show_map_idx",
                            sef_envelope.offset,
                            "show-existing-frame output requires frame_to_show_map_idx",
                        )
                    })?;
                    let source_index = layer.reference.frame_index_for_slot(slot)?;
                    let derive_order_hint = sef_core.derive_sef_order_hint == Some(true);
                    if derive_order_hint {
                        layer.reference.mark_sef_derive_output(
                            slot,
                            layer.scheduler.already_emitted(source_index),
                        )?;
                    }
                    layer.reference.note_show_existing();
                    let source = frames.get(source_index).ok_or_else(|| {
                        unsupported_at(
                            "sef_reference_frame_unavailable",
                            sef_envelope.offset,
                            "show-existing-frame output requires its retained decoded frame",
                        )
                    })?;
                    let output_mlayer = show_existing_output_layer(
                        derive_order_hint,
                        sef_envelope.header.embedded_layer_id,
                        source.embedded_layer_id,
                    );
                    let sef_frame = PipelineFrame {
                        display_grain,
                        output_effects,
                        frame_cdfs: source.frame_cdfs.clone(),
                        motion_field: source.motion_field.clone(),
                        ccso_header: source.ccso_header.clone(),
                        ccso_grid: source.ccso_grid.clone(),
                        segment_ids: source.segment_ids.clone(),
                        frame_rate: output_rate,
                        extended_layer_id: sef_envelope.header.extended_layer_id,
                        embedded_layer_id: sef_envelope.header.embedded_layer_id,
                        temporal_layer_id: sef_envelope.header.temporal_layer_id,
                        props: stream.props(),
                        frame: inflight::PipelineFrameSlot::completed_recycled(
                            source.wait_ready_frame()?,
                            frames,
                        )?,
                    };
                    let ordering = sef_core.display_order_hint().ok_or_else(|| {
                        unsupported_at(
                            "sef_missing_display_order_hint",
                            sef_envelope.offset,
                            "show-existing-frame output requires a derived display order hint",
                        )
                    })?;
                    (sef_frame, ordering, output_mlayer)
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &sef_frame,
                )?;
                let frame_index = frames.len();
                frames.push(sef_frame)?;
                retained_frame_bytes = next_retained_frame_bytes;
                {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            sef_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let sef_ordering = output_ordering(
                        ordering,
                        output_mlayer,
                        layer.sequence.general.max_mlayer_id,
                    );
                    let sef_tu_limit = output_ordering(
                        ordering,
                        EmbeddedLayerId::BASE,
                        layer.sequence.general.max_mlayer_id,
                    );
                    layer.scheduler.note_frame(
                        next_candidate.obu_type(),
                        first_picture_in_tu,
                        sef_tu_limit,
                        true,
                        false,
                    );
                    if !sef_emit_allowed {
                        layer.scheduler.suppress(frame_index);
                    }
                    layer.scheduler.on_immediate(frame_index, sef_ordering);
                }
                let (newly, total_emitted) = xlayers.newly_and_total(candidate_xlayer);
                charge_emitted_outputs(
                    options,
                    frames,
                    total_emitted,
                    &mut emission_queue,
                    newly,
                    emit,
                )?;
                reclaim_unowned_frames(
                    frames,
                    &mut xlayers,
                    &emission_queue,
                    ring,
                    &mut retained_frame_bytes,
                )?;
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
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
                let obus = stream.obus(&unit, &mut obu_buffer)?;
                let (inter_prefix_obus, inter_envelope) =
                    stream.inter_envelope(obus, next_candidate)?;
                let inter_emit_allowed = stream.layer_filter().is_layer_output_for(
                    inter_envelope.header.extended_layer_id,
                    inter_envelope.header.embedded_layer_id,
                    inter_envelope.header.temporal_layer_id,
                );
                let ivf_frame_index = next_candidate.ivf_frame().map(IvfFrameContext::frame_index);
                observe_global_and_cmvs(
                    &mut xlayers,
                    inter_prefix_obus,
                    &unit,
                    stream.checked_rap_flags(),
                    options,
                    frames,
                    &mut emission_queue,
                    emit,
                )?;
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                    *obu_storage = recycle(obu_buffer);
                    break;
                }
                xlayers.note_celu_order(candidate_xlayer, inter_envelope.offset)?;
                xlayers.observe_other_non_global_prefix(inter_prefix_obus, candidate_xlayer)?;
                let initializing_layer = !xlayers.contains(candidate_xlayer);
                if initializing_layer {
                    if next_candidate.obu_type() != ObuType::RasFrame {
                        let (tu_has_rap, _) = unit_rap_flags(&unit)?;
                        if xlayers.should_skip_pre_rap_frame(candidate_xlayer, tu_has_rap) {
                            *obu_storage = recycle(obu_buffer);
                            continue;
                        }
                        return Err(unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires a closed-loop key, open-loop key, or RAS random-access frame",
                        ));
                    }
                    let sequence_envelope = inter_prefix_obus
                        .iter()
                        .rev()
                        .find(|obu| {
                            obu.header.obu_type == ObuType::SequenceHeader
                                && obu.header.extended_layer_id == candidate_xlayer
                        })
                        .copied()
                        .ok_or_else(|| {
                            unsupported_at(
                                "missing_sequence_header",
                                inter_envelope.offset,
                                "decode runtime requires a sequence header before the first RAS frame of an extended layer",
                            )
                        })?;
                    let sequence = parse_sequence(sequence_envelope)?;
                    validate_sequence(&sequence, sequence_envelope.offset)?;
                    let num_ref_frames = usize::from(
                        sequence
                            .inter
                            .as_ref()
                            .ok_or_else(|| {
                                unsupported(
                                    "missing_sequence_inter_config",
                                    None,
                                    "multi-frame decode requires the sequence inter config (NumRefFrames)",
                                )
                            })?
                            .num_ref_frames,
                    );
                    let (mut output_effect_state, mut film_grain_slots) =
                        xlayers.take_layer_hls(candidate_xlayer);
                    XLayerTable::apply_non_global_prefix(
                        inter_prefix_obus,
                        candidate_xlayer,
                        &sequence,
                        &mut output_effect_state,
                        &mut film_grain_slots,
                    )?;
                    let has_local_lcr =
                        output_effect_state.has_local_lcr_id(sequence.general.seq_lcr_id.get());
                    xlayers.validate_xlayer_lcr(
                        candidate_xlayer,
                        &sequence,
                        has_local_lcr,
                        sequence_envelope.offset,
                        true,
                    )?;
                    let reference = reference_buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;
                    let scheduler = OutputScheduler::new(num_ref_frames);
                    let mut prelude = xlayer_state::InBandLongTermPrelude::default();
                    prelude.begin_frame(true);
                    xlayers.insert(XLayerState::new(
                        candidate_xlayer,
                        sequence,
                        reference,
                        scheduler,
                        output_effect_state,
                        film_grain_slots,
                        prelude,
                        false,
                    ));
                    frames.ensure_capacity_for_layers(xlayers.len(), ring.capacity());
                }
                let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                    unsupported_at(
                        "missing_random_access_frame",
                        inter_envelope.offset,
                        "decode runtime requires an active extended layer",
                    )
                })?;
                if !initializing_layer {
                    XLayerTable::apply_non_global_prefix(
                        inter_prefix_obus,
                        candidate_xlayer,
                        &layer.sequence,
                        &mut layer.output_effect_state,
                        &mut layer.film_grain_slots,
                    )?;
                }
                let first_picture_in_tu = if initializing_layer {
                    true
                } else {
                    layer.begin_coded_picture()
                };
                let bit_depth = layer.sequence.general.bit_depth_idc;
                let flushed = layer
                    .scheduler
                    .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                let total_emitted = xlayers.total_emitted_count();
                charge_emitted_outputs(
                    options,
                    frames,
                    total_emitted,
                    &mut emission_queue,
                    &flushed,
                    emit,
                )?;
                if output_frame_limit_reached(options, total_emitted) {
                    *obu_storage = recycle(obu_buffer);
                    break;
                }
                let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                    unsupported_at(
                        "missing_random_access_frame",
                        inter_envelope.offset,
                        "decode runtime requires an active extended layer",
                    )
                })?;
                layer
                    .reference
                    .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                if matches!(
                    next_candidate.obu_type(),
                    ObuType::Switch | ObuType::RasFrame
                ) {
                    let activation = with_sample_bit_depth!(bit_depth, T, {
                        let state = layer.reference.state::<T>(frames)?;
                        inter::parse_inter_frame_activation(
                            inter_envelope,
                            &layer.sequence,
                            &state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?
                    });
                    if next_candidate.obu_type() == ObuType::RasFrame && layer.decoding_initial_tu {
                        layer.in_band_long_term_prelude.validate_required(
                            &activation,
                            &layer.reference,
                            inter_envelope.offset,
                        )?;
                    }
                    let restricted = activation.restricted_prediction_switch;
                    if restricted == Some(true) {
                        let presence = layer.sequence.general.mlayer_dependency_map.presence_map();
                        let slots = layer.reference.restrict_references_for_switch(
                            inter_envelope.header.embedded_layer_id,
                            &presence,
                        );
                        let emitted = layer.scheduler.restrict_slots(&slots);
                        let total_emitted = xlayers.total_emitted_count();
                        charge_emitted_outputs(
                            options,
                            frames,
                            total_emitted,
                            &mut emission_queue,
                            &emitted,
                            emit,
                        )?;
                        if output_frame_limit_reached(options, total_emitted) {
                            *obu_storage = recycle(obu_buffer);
                            break;
                        }
                    }
                }
                let frame_index = frames.len();
                let (inter_core, user_qm, inter_order_hint_bits) = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let order_hint_bits = layer
                        .sequence
                        .inter
                        .as_ref()
                        .map_or(0, |inter| inter.order_hint_bits);
                    with_sample_bit_depth!(bit_depth, T, {
                        let inter_state = layer.reference.state::<T>(frames)?;
                        let inter_core = parse_inter_core_with_effects(
                            inter_envelope,
                            &layer.sequence,
                            &inter_state,
                            &layer.output_effect_state,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        if initializing_layer {
                            layer.in_band_long_term_prelude.validate_required(
                                &inter_core,
                                &layer.reference,
                                inter_envelope.offset,
                            )?;
                        }
                        let inter_core = frames.share_core(inter_core)?;
                        let user_qm = layer.output_effect_state.prepare_frame(
                            inter_envelope,
                            &inter_core,
                            &layer.sequence,
                            first_picture_in_tu,
                            ivf_frame_index,
                        )?;
                        Result::Ok((inter_core, user_qm, order_hint_bits))
                    })?
                };
                xlayers.check_tu_doh(
                    candidate_xlayer,
                    inter_order_hint_bits,
                    inter_core.display_order_hint(),
                    frame_is_output(&inter_core),
                    inter_envelope.offset,
                )?;
                if inter_emit_allowed && frame_is_output(&inter_core) {
                    let next_output_frame_count = checked_add(
                        DecodeLimitName::MaxOutputFrames,
                        xlayers.total_emitted_count() as u64,
                        1,
                    )?;
                    ensure_output_frame_count_limit(options.limits(), next_output_frame_count)?;
                }
                admit_within_reference_store(
                    |xlayers, retained| {
                        let layer = xlayers.get(candidate_xlayer).ok_or_else(|| {
                            unsupported_at(
                                "missing_random_access_frame",
                                inter_envelope.offset,
                                "decode runtime requires an active extended layer",
                            )
                        })?;
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained,
                            &inter_core,
                            &layer.sequence,
                            inter_envelope.offset,
                        )
                    },
                    options,
                    rows,
                    pools,
                    ring,
                    frames,
                    &mut xlayers,
                    &mut emission_queue,
                    emit,
                    &mut retained_frame_bytes,
                )?;
                let decoded = with_sample_bit_depth!(bit_depth, T, {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let inter_state = layer.reference.state::<T>(frames)?;
                    let split = rows.filter(|_| {
                        inter::splittable_inter_frame(next_candidate.obu_type(), &inter_core)
                    });
                    let _user_qm_scope =
                        crate::bitstream::tile_payload::FrameUserQmScope::install(user_qm);
                    let _qm_scope = crate::bitstream::tile_payload::FrameQmScope::install(
                        frame_engine::intra::build_frame_qm_levels(&inter_core),
                    );
                    if let Some(rows) = split {
                        ring.reserve(pools);
                        push_split_inter::<T>(
                            pools,
                            rows,
                            ring,
                            frames,
                            options,
                            &layer.sequence,
                            &mut layer.shared_sequence,
                            next_candidate,
                            &unit,
                            inter_core,
                            inter_state,
                            frame_index,
                        )?
                    } else if inter_envelope.header.obu_type.is_tip_frame() {
                        ring.reserve(pools);
                        let (slot, finish, geometry, products, motion) =
                            frame_tasks::reserve_tip_output(
                                &inter_core,
                                &layer.sequence,
                                T::BIT_DEPTH,
                                T::erase,
                                frames,
                                ring,
                                frame_index,
                            )?;
                        let publications = products.handles();
                        let entropy =
                            inter::entropy_dependencies(&inter_core, &layer.sequence, &inter_state);
                        let motion_deps = inter_core
                            .inter
                            .as_ref()
                            .map_or_else(reference_buffer::RefSlots::default, |inter| {
                                inter_state.motion_dependencies(&inter.ref_frame_idx)
                            });
                        let core = Arc::clone(&inter_core);
                        pools.push_tip::<T>(
                            backend,
                            rows,
                            options,
                            frame_tasks::TipFrameInputs {
                                candidate: next_candidate.clone(),
                                core: inter_core,
                                sequence: share_sequence(
                                    &mut layer.shared_sequence,
                                    &layer.sequence,
                                ),
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
                        ring.reserve(pools);
                        let setup = if inter_core.status
                            == splot_core::headers::frame::FrameHeaderParseStatus::IntraHeaderComplete
                        {
                            frame_engine::FrameSetup::Intra
                        } else {
                            frame_engine::FrameSetup::Inter(&inter_state)
                        };
                        let mut product_writers = frames.reserve_products()?;
                        let walk = walk_frame_with_backend(
                            backend,
                            &mut T::pools(pools).scratch,
                            next_candidate,
                            unit.view(),
                            inter_envelope,
                            inter_core,
                            &layer.sequence,
                            options,
                            &setup,
                            T::BIT_DEPTH,
                            &mut product_writers,
                        )?;
                        let inter_core = Arc::clone(&walk.core);
                        let slot = inflight::settle_walk_stage(
                            backend,
                            walk.stage,
                            T::erase,
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
                });
                let (inter_slot, inter_core, products, motion_field) = decoded;
                let inter_display_grain = {
                    let layer = xlayers.get(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    layer.film_grain_slots.active_for_core(
                        inter_envelope,
                        &inter_core,
                        &layer.sequence,
                        inter_envelope.offset,
                    )?
                };
                xlayers.observe_suffix_for_xlayer(
                    suffix_after_candidate(obus, next_candidate)?,
                    candidate_xlayer,
                )?;
                let inter_output_effects = xlayers
                    .get_mut(candidate_xlayer)
                    .ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?
                    .output_effect_state
                    .finish_frame(inter_envelope.header.embedded_layer_id);
                let inter_frame_rate = inter_output_effects.frame_rate(frame_rate);
                let inter_update = frame_ref_update_from_core(
                    &inter_core,
                    inter_envelope.offset,
                    inter_envelope.header.embedded_layer_id,
                    inter_envelope.header.temporal_layer_id,
                )?;
                let inter_frame = PipelineFrame {
                    frame: inter_slot,
                    display_grain: inter_display_grain,
                    output_effects: inter_output_effects,
                    frame_cdfs: products.frame_cdfs,
                    motion_field,
                    ccso_header: Some(Arc::clone(&inter_core)),
                    ccso_grid: products.ccso_grid,
                    segment_ids: products.segment_ids,
                    frame_rate: inter_frame_rate,
                    extended_layer_id: inter_envelope.header.extended_layer_id,
                    embedded_layer_id: inter_envelope.header.embedded_layer_id,
                    temporal_layer_id: inter_envelope.header.temporal_layer_id,
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
                let (inter_ordering, inter_tu_limit) = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let inter_ordering = output_ordering(
                        inter_hint,
                        inter_envelope.header.embedded_layer_id,
                        layer.sequence.general.max_mlayer_id,
                    );
                    let inter_tu_limit = output_ordering(
                        inter_hint,
                        EmbeddedLayerId::BASE,
                        layer.sequence.general.max_mlayer_id,
                    );
                    if !inter_emit_allowed {
                        layer.scheduler.suppress(frame_index);
                    }
                    layer.scheduler.refresh(
                        inter_update.refresh_frame_flags,
                        frame_index,
                        inter_ordering,
                        inter_implicit,
                        inter_key_or_switch || initializing_layer,
                    );
                    (inter_ordering, inter_tu_limit)
                };
                let (newly_refresh, total_emitted) = xlayers.newly_and_total(candidate_xlayer);
                charge_emitted_outputs(
                    options,
                    frames,
                    total_emitted,
                    &mut emission_queue,
                    newly_refresh,
                    emit,
                )?;
                let trigger_immediate = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            inter_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    layer
                        .reference
                        .update(frame_index, &inter_update, inter_key_or_switch);
                    layer.reference.note_frame(
                        next_candidate.obu_type(),
                        first_picture_in_tu,
                        &inter_update,
                    );
                    layer.scheduler.note_frame(
                        next_candidate.obu_type(),
                        first_picture_in_tu,
                        inter_tu_limit,
                        inter_immediate,
                        inter_implicit,
                    );
                    if inter_immediate && !layer.scheduler.already_emitted(frame_index) {
                        layer.scheduler.on_immediate(frame_index, inter_ordering);
                        true
                    } else {
                        false
                    }
                };
                if trigger_immediate {
                    let (newly_immediate, total_emitted) =
                        xlayers.newly_and_total(candidate_xlayer);
                    charge_emitted_outputs(
                        options,
                        frames,
                        total_emitted,
                        &mut emission_queue,
                        newly_immediate,
                        emit,
                    )?;
                }
                reclaim_unowned_frames(
                    frames,
                    &mut xlayers,
                    &emission_queue,
                    ring,
                    &mut retained_frame_bytes,
                )?;
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                    break;
                }
            }
            ObuType::ClosedLoopKey | ObuType::OpenLoopKey => {
                let starts_new_sequence = next_candidate.obu_type() == ObuType::ClosedLoopKey;
                let mut obu_buffer = recycle(core::mem::take(obu_storage));
                let obus = stream.obus(&unit, &mut obu_buffer)?;
                let (key_sequence_envelope, key_prefix_obus, key_envelope) = if starts_new_sequence
                {
                    ivf_key_frame_unit(obus, next_candidate)?
                } else {
                    let (prefix, frame) = stream.inter_envelope(obus, next_candidate)?;
                    let sequence_envelope = prefix
                        .iter()
                        .rev()
                        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
                        .copied();
                    (sequence_envelope, prefix, frame)
                };
                let (key_sequence, key_sequence_offset) = if let Some(envelope) =
                    key_sequence_envelope
                {
                    let parsed = parse_sequence(envelope)?;
                    validate_sequence(&parsed, envelope.offset)?;
                    (parsed, envelope.offset)
                } else if let Some(sequence) = xlayers.available_sequence(candidate_xlayer) {
                    (sequence.clone(), key_envelope.offset)
                } else {
                    return Err(unsupported_at(
                        "missing_sequence_header",
                        key_envelope.offset,
                        "decode runtime requires a sequence header before the first frame of an extended layer",
                    ));
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
                if !starts_new_sequence
                    && let Some(layer) = xlayers.get(candidate_xlayer)
                    && key_num_ref_frames != layer.reference.len()
                {
                    return Err(unsupported_at(
                        "olk_reference_buffer_size_change",
                        key_envelope.offset,
                        "open-loop-key sequence activation changed NumRefFrames inside the active coded video sequence",
                    ));
                }
                observe_global_and_cmvs(
                    &mut xlayers,
                    key_prefix_obus,
                    &unit,
                    stream.checked_rap_flags(),
                    options,
                    frames,
                    &mut emission_queue,
                    emit,
                )?;
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                    *obu_storage = recycle(obu_buffer);
                    break;
                }
                xlayers.note_celu_order(candidate_xlayer, key_envelope.offset)?;
                xlayers.observe_other_non_global_prefix(key_prefix_obus, candidate_xlayer)?;
                let initializing_layer = !xlayers.contains(candidate_xlayer);
                if initializing_layer {
                    let (mut output_effect_state, mut film_grain_slots) =
                        xlayers.take_layer_hls(candidate_xlayer);
                    XLayerTable::apply_non_global_prefix(
                        key_prefix_obus,
                        candidate_xlayer,
                        &key_sequence,
                        &mut output_effect_state,
                        &mut film_grain_slots,
                    )?;
                    let has_local_lcr =
                        output_effect_state.has_local_lcr_id(key_sequence.general.seq_lcr_id.get());
                    xlayers.validate_xlayer_lcr(
                        candidate_xlayer,
                        &key_sequence,
                        has_local_lcr,
                        key_sequence_offset,
                        true,
                    )?;
                    let reference =
                        reference_buffer::RuntimeReferenceBuffer::new(key_num_ref_frames)?;
                    let scheduler = OutputScheduler::new(key_num_ref_frames);
                    let mut prelude = xlayer_state::InBandLongTermPrelude::default();
                    prelude.begin_frame(true);
                    xlayers.insert(XLayerState::new(
                        candidate_xlayer,
                        key_sequence.clone(),
                        reference,
                        scheduler,
                        output_effect_state,
                        film_grain_slots,
                        prelude,
                        next_candidate.obu_type() == ObuType::OpenLoopKey,
                    ));
                    frames.ensure_capacity_for_layers(xlayers.len(), ring.capacity());
                } else {
                    let has_local_lcr = {
                        let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                            unsupported_at(
                                "missing_random_access_frame",
                                key_envelope.offset,
                                "decode runtime requires an active extended layer",
                            )
                        })?;
                        XLayerTable::apply_non_global_prefix(
                            key_prefix_obus,
                            candidate_xlayer,
                            &key_sequence,
                            &mut layer.output_effect_state,
                            &mut layer.film_grain_slots,
                        )?;
                        layer
                            .output_effect_state
                            .has_local_lcr_id(key_sequence.general.seq_lcr_id.get())
                    };
                    if starts_new_sequence || key_sequence_envelope.is_some() {
                        xlayers.validate_xlayer_lcr(
                            candidate_xlayer,
                            &key_sequence,
                            has_local_lcr,
                            key_sequence_offset,
                            false,
                        )?;
                    }
                }
                let first_picture_in_tu = if initializing_layer {
                    true
                } else {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let first = layer.begin_coded_picture();
                    if !first
                        && layer.decoding_initial_tu
                        && next_candidate.obu_type() == ObuType::OpenLoopKey
                    {
                        layer.discard_leading_frames = true;
                    }
                    first
                };
                if !starts_new_sequence && !initializing_layer {
                    let flushed = {
                        let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                            unsupported_at(
                                "missing_random_access_frame",
                                key_envelope.offset,
                                "decode runtime requires an active extended layer",
                            )
                        })?;
                        let flushed = layer
                            .scheduler
                            .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                        layer
                            .reference
                            .prepare_for_frame(next_candidate.obu_type(), first_picture_in_tu);
                        flushed
                    };
                    let total_emitted = xlayers.total_emitted_count();
                    charge_emitted_outputs(
                        options,
                        frames,
                        total_emitted,
                        &mut emission_queue,
                        &flushed,
                        emit,
                    )?;
                    if output_frame_limit_reached(options, total_emitted) {
                        *obu_storage = recycle(obu_buffer);
                        break;
                    }
                }
                let (key_core, key_user_qm, key_display_grain) = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let key_core = if starts_new_sequence {
                        parse_key_core_with_effects(
                            key_envelope,
                            &key_sequence,
                            &layer.output_effect_state,
                        )?
                    } else {
                        with_sample!(key_sequence, T, {
                            let state = layer.reference.state::<T>(frames)?;
                            parse_olk_core_with_effects(
                                key_envelope,
                                &key_sequence,
                                &state,
                                &layer.output_effect_state,
                                first_picture_in_tu,
                            )?
                        })
                    };
                    if layer.decoding_initial_tu {
                        layer.in_band_long_term_prelude.validate_required(
                            &key_core,
                            &layer.reference,
                            key_envelope.offset,
                        )?;
                    }
                    ensure_intra_header_complete(&key_core, key_envelope.offset)?;
                    let key_user_qm = layer.output_effect_state.prepare_frame(
                        key_envelope,
                        &key_core,
                        &key_sequence,
                        first_picture_in_tu,
                        next_candidate.ivf_frame().map(IvfFrameContext::frame_index),
                    )?;
                    let key_display_grain = layer.film_grain_slots.active_for_core(
                        key_envelope,
                        &key_core,
                        &key_sequence,
                        key_envelope.offset,
                    )?;
                    (key_core, key_user_qm, key_display_grain)
                };
                xlayers.check_tu_doh(
                    candidate_xlayer,
                    key_sequence
                        .inter
                        .as_ref()
                        .map_or(0, |inter| inter.order_hint_bits),
                    key_core.display_order_hint(),
                    frame_is_output(&key_core),
                    key_envelope.offset,
                )?;
                if starts_new_sequence && !initializing_layer {
                    let (key_reference, flushed) = {
                        let key_reference =
                            reference_buffer::RuntimeReferenceBuffer::new(key_num_ref_frames)?;
                        let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                            unsupported_at(
                                "missing_random_access_frame",
                                key_envelope.offset,
                                "decode runtime requires an active extended layer",
                            )
                        })?;
                        let flushed = layer.scheduler.start_new_sequence(key_num_ref_frames);
                        (key_reference, flushed)
                    };
                    charge_emitted_outputs(
                        options,
                        frames,
                        xlayers.total_emitted_count(),
                        &mut emission_queue,
                        &flushed,
                        emit,
                    )?;
                    if let Some(layer) = xlayers.get_mut(candidate_xlayer) {
                        layer.reference = key_reference;
                    }
                    reclaim_unowned_frames(
                        frames,
                        &mut xlayers,
                        &emission_queue,
                        ring,
                        &mut retained_frame_bytes,
                    )?;
                    if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                        *obu_storage = recycle(obu_buffer);
                        break;
                    }
                }
                if !initializing_layer && let Some(layer) = xlayers.get_mut(candidate_xlayer) {
                    layer.sequence = key_sequence;
                    layer.shared_sequence = None;
                }
                xlayers.observe_suffix_for_xlayer(
                    suffix_after_candidate(obus, next_candidate)?,
                    candidate_xlayer,
                )?;
                let key_output_effects = xlayers
                    .get_mut(candidate_xlayer)
                    .ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?
                    .output_effect_state
                    .finish_frame(key_envelope.header.embedded_layer_id);
                admit_within_reference_store(
                    |xlayers, retained| {
                        let layer = xlayers.get(candidate_xlayer).ok_or_else(|| {
                            unsupported_at(
                                "missing_random_access_frame",
                                key_envelope.offset,
                                "decode runtime requires an active extended layer",
                            )
                        })?;
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained,
                            &key_core,
                            &layer.sequence,
                            key_envelope.offset,
                        )
                    },
                    options,
                    rows,
                    pools,
                    ring,
                    frames,
                    &mut xlayers,
                    &mut emission_queue,
                    emit,
                    &mut retained_frame_bytes,
                )?;
                let frame_index = frames.len();
                let key_frame = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    decode_key_frame_with_effects(
                        rows,
                        backend,
                        pools,
                        ring,
                        frames,
                        Some((&unit, &mut layer.shared_sequence)),
                        frame_index,
                        unit.view(),
                        options,
                        next_candidate,
                        key_envelope,
                        key_core.clone(),
                        &layer.sequence,
                        frame_rate,
                        key_display_grain,
                        key_user_qm,
                        key_output_effects,
                        stream.props(),
                    )?
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &key_frame,
                )?;
                let key_update = frame_ref_update_from_core(
                    &key_core,
                    key_envelope.offset,
                    key_envelope.header.embedded_layer_id,
                    key_envelope.header.temporal_layer_id,
                )?;
                frames.push(key_frame)?;
                retained_frame_bytes = next_retained_frame_bytes;
                let key_hint = key_update.order_hint;
                let key_implicit = key_core.implicit_output_frame == Some(true);
                let key_immediate = key_core.immediate_output_frame == Some(true);
                let key_emit_allowed = stream.layer_filter().is_layer_output_for(
                    key_envelope.header.extended_layer_id,
                    key_envelope.header.embedded_layer_id,
                    key_envelope.header.temporal_layer_id,
                );
                let (key_ordering, key_tu_limit) = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    let key_ordering = output_ordering(
                        key_hint,
                        key_envelope.header.embedded_layer_id,
                        layer.sequence.general.max_mlayer_id,
                    );
                    let key_tu_limit = output_ordering(
                        key_hint,
                        EmbeddedLayerId::BASE,
                        layer.sequence.general.max_mlayer_id,
                    );
                    if !key_emit_allowed {
                        layer.scheduler.suppress(frame_index);
                    }
                    layer.scheduler.refresh(
                        key_update.refresh_frame_flags,
                        frame_index,
                        key_ordering,
                        key_implicit,
                        true,
                    );
                    (key_ordering, key_tu_limit)
                };
                let (newly_refresh, total_emitted) = xlayers.newly_and_total(candidate_xlayer);
                charge_emitted_outputs(
                    options,
                    frames,
                    total_emitted,
                    &mut emission_queue,
                    newly_refresh,
                    emit,
                )?;
                let trigger_immediate = {
                    let layer = xlayers.get_mut(candidate_xlayer).ok_or_else(|| {
                        unsupported_at(
                            "missing_random_access_frame",
                            key_envelope.offset,
                            "decode runtime requires an active extended layer",
                        )
                    })?;
                    layer
                        .reference
                        .update(frame_index, &key_update, is_key_or_switch(&key_core));
                    layer.reference.note_frame(
                        next_candidate.obu_type(),
                        first_picture_in_tu,
                        &key_update,
                    );
                    layer
                        .in_band_long_term_prelude
                        .note_frame(&key_core, frame_index);
                    layer.scheduler.note_frame(
                        next_candidate.obu_type(),
                        first_picture_in_tu,
                        key_tu_limit,
                        key_immediate,
                        key_implicit,
                    );
                    if key_immediate && !layer.scheduler.already_emitted(frame_index) {
                        layer.scheduler.on_immediate(frame_index, key_ordering);
                        true
                    } else {
                        false
                    }
                };
                if trigger_immediate {
                    let (newly_immediate, total_emitted) =
                        xlayers.newly_and_total(candidate_xlayer);
                    charge_emitted_outputs(
                        options,
                        frames,
                        total_emitted,
                        &mut emission_queue,
                        newly_immediate,
                        emit,
                    )?;
                }
                reclaim_unowned_frames(
                    frames,
                    &mut xlayers,
                    &emission_queue,
                    ring,
                    &mut retained_frame_bytes,
                )?;
                *obu_storage = recycle(obu_buffer);
                if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
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

    if !output_frame_limit_reached(options, xlayers.total_emitted_count()) {
        for layer_pos in 0..xlayers.len() {
            if output_frame_limit_reached(options, xlayers.total_emitted_count()) {
                break;
            }
            let flushed = xlayers.flush_layer(layer_pos);
            charge_emitted_outputs(
                options,
                frames,
                xlayers.total_emitted_count(),
                &mut emission_queue,
                &flushed,
                emit,
            )?;
        }
        emission_queue.flush(frames, emit)?;
        ring.harvest_all(pools);
        reclaim_unowned_frames(
            frames,
            &mut xlayers,
            &emission_queue,
            ring,
            &mut retained_frame_bytes,
        )?;
    } else {
        emission_queue.flush(frames, emit)?;
    }
    Ok(())
}

pub(crate) mod frame_engine;
pub(crate) mod general_intra;
pub(crate) mod reconstruct;

#[cfg(test)]
mod general_intra_d135_tests;
#[cfg(test)]
mod general_intra_d157_tests;
#[cfg(test)]
mod general_intra_horizontal_tests;
#[cfg(test)]
mod general_intra_lossless_d113_tests;
#[cfg(test)]
mod general_intra_lossless_d157_tests;
#[cfg(test)]
mod general_intra_lossless_hfollow_tests;
#[cfg(test)]
mod general_intra_lossless_sdp_d113_tests;
#[cfg(test)]
mod general_intra_lossless_smooth_tests;
#[cfg(test)]
mod general_intra_tests;

#[cfg(test)]
use tile_plan::continuation_structure_start_bits;
pub(crate) use tile_plan::{TileFactsKind, derive_tile_plan, derive_tile_plan_with};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
