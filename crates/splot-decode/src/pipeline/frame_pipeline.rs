// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Admission-scheduled parse, reconstruction, and filter overlap.
//!
//! The entropy pass reads no reference samples, so the driver can submit the
//! previous frame's reconstruction and continue parsing. Every cross-frame
//! dependency is an admission condition: pool tasks never wait for motion or
//! pixel publication. Ordered continuations preserve the commit spine, while
//! the driver alone waits when later bookkeeping needs reconstruction complete.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_parallel::{AdmissionScheduler, CompletionCell, Condition};
use splot_recon::{BitDepth, ReconSample};

use crate::Result;
use crate::error::DecodeError;
use crate::prediction::inter;

use super::inflight::{
    FinishSpawner, InflightRing, PendingFinish, PipelineFrameSlot, RefFrameSlot,
};
use super::unsupported;

type EntropyResult<T> = Arc<CompletionCell<Mutex<Option<Result<inter::DeferredInterWalk<T>>>>>>;
type EntropyEarly<T> = Arc<CompletionCell<Mutex<Option<inter::InterWalkEarly<T>>>>>;

/// The two halves one scheduled entropy pass publishes: the pre-parse half the
/// admission scheduler is built from, and the pass's own products.
pub(super) struct EntropyHandles<T: splot_recon::ReconSample> {
    early: EntropyEarly<T>,
    tail: EntropyResult<T>,
}

/// One scheduler-owned entropy pass whose reconstruction has not been promoted.
pub(super) enum PendingEntropy {
    Eight {
        frame_index: usize,
        result: EntropyHandles<u8>,
        finish: PendingFinish<u8>,
    },
    Ten {
        frame_index: usize,
        result: EntropyHandles<u16>,
        finish: PendingFinish<u16>,
    },
}

impl PendingEntropy {
    fn is_settled(&self) -> bool {
        match self {
            Self::Eight { result, .. } => result.early.get().is_some(),
            Self::Ten { result, .. } => result.early.get().is_some(),
        }
    }
}

/// Ordered, bounded entropy contexts awaiting reconstruction admission.
#[derive(Default)]
pub(super) struct PendingEntropyQueue {
    entries: VecDeque<PendingEntropy>,
}

impl PendingEntropyQueue {
    pub(super) fn push(&mut self, pending: PendingEntropy) {
        self.entries.push_back(pending);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_entropy<'scope, 'job, T, P>(
    parse: P,
    frame_index: usize,
    frame_cdfs: inter::FrameCdfHandle,
    ccso_grid: inter::CcsoGridHandle,
    segment_ids: inter::SegmentIdMapHandle,
    motion: inter::MotionFieldHandle,
    dependencies: &inter::EntropyDependencies,
    scheduler: &'scope AdmissionScheduler<'job>,
    spawner: &FinishSpawner<'_, 'scope>,
) -> Result<EntropyHandles<T>>
where
    T: splot_recon::ReconSample + Send + 'static,
    P: FnOnce(&dyn Fn(inter::InterWalkEarly<T>)) -> Result<inter::DeferredInterWalk<T>>
        + Send
        + 'job,
    'job: 'scope,
{
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    let result = Arc::new(CompletionCell::new());
    let result_for_job = Arc::clone(&result);
    let early: EntropyEarly<T> = Arc::new(CompletionCell::new());
    let early_for_job = Arc::clone(&early);
    let failed_cdfs = frame_cdfs.clone();
    let failed_ccso = ccso_grid.clone();
    let failed_segment_ids = segment_ids.clone();
    let failed_motion = motion;
    let conditions = dependencies.conditions();
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_key,
        &conditions,
        Box::new(move |_| {
            let parsed = parse(&|early| {
                let _ = early_for_job.set(Mutex::new(Some(early)));
            });
            if let Ok(deferred) = &parsed {
                frame_cdfs.publish(Arc::clone(deferred.frame_cdfs()));
                ccso_grid.publish(deferred.ccso_grid().cloned().map(Arc::new));
                segment_ids.publish(Arc::clone(deferred.segment_ids()));
            } else {
                failed_cdfs.fail();
                failed_ccso.fail();
                failed_segment_ids.fail();
                failed_motion.fail();
            }
            let _ = result_for_job.set(Mutex::new(Some(parsed)));
            let _ = early_for_job.set(Mutex::new(None));
        }),
    );
    Ok(EntropyHandles {
        early,
        tail: result,
    })
}

const ORDER_KEY_FRAME_STRIDE: u64 = 1 << 32;
type TemporalScratchSlot = Arc<CompletionCell<Mutex<Option<inter::TemporalMvScratch>>>>;
type ReconScratchSlot = Arc<CompletionCell<Mutex<Option<ScheduledReconScratch>>>>;

pub(super) enum ScheduledReconScratch {
    Eight(inter::InterDecodeScratch<u8>),
    Ten(inter::InterDecodeScratch<u16>),
}

pub(super) trait ScheduledScratchSample: splot_recon::ReconSample {
    fn take_scheduled_scratch(
        scratch: &mut Option<ScheduledReconScratch>,
    ) -> Option<inter::InterDecodeScratch<Self>>;

    fn wrap_scheduled_scratch(scratch: inter::InterDecodeScratch<Self>) -> ScheduledReconScratch;
}

macro_rules! impl_scheduled_scratch_sample {
    ($sample:ty, $variant:ident) => {
        impl ScheduledScratchSample for $sample {
            fn take_scheduled_scratch(
                scratch: &mut Option<ScheduledReconScratch>,
            ) -> Option<inter::InterDecodeScratch<Self>> {
                match scratch.take() {
                    Some(ScheduledReconScratch::$variant(scratch)) => Some(scratch),
                    other => {
                        *scratch = other;
                        None
                    }
                }
            }

            fn wrap_scheduled_scratch(
                scratch: inter::InterDecodeScratch<Self>,
            ) -> ScheduledReconScratch {
                ScheduledReconScratch::$variant(scratch)
            }
        }
    };
}

impl_scheduled_scratch_sample!(u8, Eight);
impl_scheduled_scratch_sample!(u16, Ten);

type PendingTipProducts<T> = (
    PipelineFrameSlot,
    PendingFinish<T>,
    inter::FrameDecodeGeometry,
    inter::FrameCdfHandle,
    inter::CcsoGridHandle,
    inter::SegmentIdMapHandle,
    inter::MotionFieldHandle,
);

/// Reserves the pending frame and product handles published by one TIP job.
pub(super) fn reserve_tip_output<T: ReconSample>(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    bit_depth: BitDepth,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<PendingTipProducts<T>> {
    let geometry = inter::FrameDecodeGeometry::new(core, sequence, bit_depth, false)?;
    let (slot, finish) =
        super::inflight::reserve_pending_slot(geometry.info(), erase, ring, frame_index)?;
    let frame_cdfs = inter::FrameCdfHandle::pending();
    let ccso_grid = inter::CcsoGridHandle::pending();
    let segment_ids = inter::SegmentIdMapHandle::pending();
    let motion = inter::MotionFieldHandle::pending_with_layout(geometry.motion_layout());
    Ok((
        slot,
        finish,
        geometry,
        frame_cdfs,
        ccso_grid,
        segment_ids,
        motion,
    ))
}

/// Admits one reference-gated TIP output reconstruction without stopping the
/// frame driver at its pixel-reference barrier.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_tip_output<'job, 'scope, T, P>(
    reconstruct: P,
    frame_index: usize,
    dependencies: &[Condition<'_>],
    frame_cdfs: inter::FrameCdfHandle,
    ccso_grid: inter::CcsoGridHandle,
    segment_ids: inter::SegmentIdMapHandle,
    motion: inter::MotionFieldHandle,
    finish: PendingFinish<T>,
    scheduler: &'scope AdmissionScheduler<'job>,
    spawner: &FinishSpawner<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    T: ScheduledScratchSample + Send + 'static,
    P: FnOnce(&mut inter::InterDecodeScratch<T>) -> Result<inter::InterDecodeOutput<T>>
        + Send
        + 'job,
    'job: 'scope,
{
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    let (scratch_source, scratch_done) = lane.reserve_recon();
    let mut conditions = dependencies.to_vec();
    if let Some(gate) = scratch_source.as_deref() {
        conditions.push(Condition::completion(gate));
    }
    let scratch_for_job = scratch_source.clone();
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_key,
        &conditions,
        Box::new(move |_| {
            let mut scratch = scratch_for_job
                .as_deref()
                .and_then(CompletionCell::get)
                .and_then(|scratch| {
                    T::take_scheduled_scratch(
                        &mut scratch.lock().unwrap_or_else(PoisonError::into_inner),
                    )
                })
                .unwrap_or_default();
            match reconstruct(&mut scratch) {
                Ok((frame, _, cdfs, ccso, field, segments)) => {
                    frame_cdfs.publish(cdfs);
                    ccso_grid.publish(ccso.map(Arc::new));
                    segment_ids.publish(Arc::new(segments));
                    motion.publish(field);
                    finish.complete_frame(frame);
                }
                Err(error) => {
                    frame_cdfs.fail();
                    ccso_grid.fail();
                    segment_ids.fail();
                    motion.fail();
                    finish.fail(error);
                }
            }
            let _ = scratch_done.set(Mutex::new(Some(T::wrap_scheduled_scratch(scratch))));
        }),
    );
    Ok(())
}

/// The frame-context admission bound for scheduled reconstruction.
pub(super) struct ReconAdmissionLane {
    depth: usize,
    recon: VecDeque<ReconScratchSlot>,
    filters: VecDeque<Arc<CompletionCell<()>>>,
    temporal: VecDeque<TemporalScratchSlot>,
}

impl ReconAdmissionLane {
    pub(super) fn new(depth: usize) -> Self {
        Self {
            depth: depth.max(1),
            recon: VecDeque::new(),
            filters: VecDeque::new(),
            temporal: VecDeque::new(),
        }
    }

    fn reserve<T>(
        depth: usize,
        lane: &mut VecDeque<Arc<CompletionCell<T>>>,
    ) -> (Option<Arc<CompletionCell<T>>>, Arc<CompletionCell<T>>) {
        let gate = (lane.len() >= depth).then(|| Arc::clone(&lane[0]));
        let done = Arc::new(CompletionCell::new());
        lane.push_back(Arc::clone(&done));
        while lane.len() > depth {
            lane.pop_front();
        }
        (gate, done)
    }

    fn reserve_recon(&mut self) -> (Option<ReconScratchSlot>, ReconScratchSlot) {
        Self::reserve(self.depth, &mut self.recon)
    }

    fn reserve_filter(&mut self) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        Self::reserve(self.depth, &mut self.filters)
    }

    fn reserve_temporal(&mut self) -> (Option<TemporalScratchSlot>, TemporalScratchSlot) {
        Self::reserve(self.depth, &mut self.temporal)
    }
}

struct ScheduledFrame<T: splot_recon::ReconSample> {
    reconstruction: inter::ScheduledTileRecon<T>,
    finish: Mutex<Option<PendingFinish<T>>>,
    motion: inter::MotionFieldHandle,
    scratch_done: ReconScratchSlot,
    filter_gate: Option<Arc<CompletionCell<()>>>,
    filter_done: Arc<CompletionCell<()>>,
    prepared: Vec<CompletionCell<()>>,
    frontier_done: Vec<CompletionCell<()>>,
    filtered: Vec<CompletionCell<()>>,
    filter_error: Mutex<Option<DecodeError>>,
    filters_ready: Arc<CompletionCell<()>>,
    failed: AtomicBool,
    order_base: u64,
}

impl<T: ScheduledScratchSample + Send + 'static> ScheduledFrame<T> {
    fn submit_batches(
        self: &Arc<Self>,
        batches: core::ops::Range<usize>,
        admit: &dyn splot_parallel::Admit<'_>,
    ) {
        let starts_commit = batches.start == 0 && !batches.is_empty();
        let mut ready_key = None;
        let mut ready = Vec::<splot_parallel::Job<'_>>::new();
        for index in batches {
            let conditions = self.reconstruction.conditions(index);
            let row = Arc::clone(self);
            let job: splot_parallel::Job<'_> = Box::new(move |admit| row.precompute(index, admit));
            if conditions.is_empty() {
                ready_key.get_or_insert_with(|| self.batch_key(index, 1));
                ready.push(job);
            } else {
                if let Some(order_key) = ready_key.take() {
                    admit.submit_ready_batch(order_key, core::mem::take(&mut ready));
                }
                admit.submit(self.batch_key(index, 1), &conditions, job);
            }
        }
        if let Some(order_key) = ready_key {
            admit.submit_ready_batch(order_key, ready);
        }
        if starts_commit {
            let commit = Arc::clone(self);
            admit.submit(
                self.batch_key(0, 2),
                &[
                    Condition::completion(&self.prepared[0]),
                    Condition::completion(self.filters_ready.as_ref()),
                ],
                Box::new(move |admit| commit.commit(0, admit)),
            );
        }
    }

    fn continue_commit(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        let commit = Arc::clone(self);
        let job = Box::new(move |admit: &dyn splot_parallel::Admit<'_>| {
            commit.commit(index, admit);
        });
        if self.prepared[index].is_set() {
            admit.continue_ready(self.batch_key(index, 2), job);
        } else {
            admit.submit(
                self.batch_key(index, 2),
                &[Condition::completion(&self.prepared[index])],
                job,
            );
        }
    }

    /// Order key for one batch's `slot`-th link, in submission order.
    fn batch_key(&self, index: usize, slot: u64) -> u64 {
        let index_key = u64::try_from(index).unwrap_or(u64::MAX / 4);
        self.order_base
            .saturating_add(1 << 20)
            .saturating_add(index_key.saturating_mul(4).saturating_add(slot))
    }

    /// Submits the § 7.17 frontier link for one sealed superblock row.
    ///
    /// The chain is ordered by the previous link alone: a link is submitted
    /// exactly when the commit spine has sealed its rows, so its own source is
    /// final before it exists.
    fn submit_frontier(
        self: &Arc<Self>,
        batch: usize,
        row: usize,
        admit: &dyn splot_parallel::Admit<'_>,
    ) {
        let conditions = row
            .checked_sub(1)
            .and_then(|previous| self.frontier_done.get(previous))
            .map(|previous| vec![Condition::completion(previous)])
            .unwrap_or_default();
        let frame = Arc::clone(self);
        admit.submit(
            self.batch_key(batch, 3),
            &conditions,
            Box::new(move |admit| frame.frontier(row, admit)),
        );
    }

    fn frontier(self: &Arc<Self>, row: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if !self.failed.load(Ordering::Acquire) {
            match self.reconstruction.frontier(row) {
                Ok(progress) => self.publish_filters(progress, admit),
                Err(error) => self.fail(error, admit),
            }
        }
        if let Some(done) = self.frontier_done.get(row) {
            let _ = done.set(());
        }
    }

    fn submit_resolve(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        let conditions = self.reconstruction.resolve_conditions(index);
        let resolve = Arc::clone(self);
        let index_key = u64::try_from(index).unwrap_or(u64::MAX / 2);
        admit.submit(
            self.order_base.saturating_add(index_key),
            &conditions,
            Box::new(move |admit| resolve.resolve(index, admit)),
        );
    }

    /// Resolves one § 7.12 band, or resumes on the § 8.2 watermark when the
    /// pass has not yet published the units the band still owes.
    fn resolve(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        let (batches, awaiting) = match self.reconstruction.resolve(index) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.fail(error, admit);
                return;
            }
        };
        self.submit_batches(batches, admit);
        if let Some(units) = awaiting {
            let resume = Arc::clone(self);
            let index_key = u64::try_from(index).unwrap_or(u64::MAX / 2);
            admit.submit(
                self.order_base.saturating_add(index_key),
                &[Condition::watermark(
                    self.reconstruction.parse_watermark(),
                    units,
                )],
                Box::new(move |admit| resume.resolve(index, admit)),
            );
            return;
        }
        let next = index.saturating_add(1);
        if next < self.reconstruction.resolve_len() {
            self.submit_resolve(next, admit);
        }
    }

    fn precompute(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if !self.failed.load(Ordering::Acquire)
            && let Err(error) = self.reconstruction.precompute(index)
        {
            self.fail(error, admit);
        }
        if let Some(prepared) = self.prepared.get(index) {
            let _ = prepared.set(());
        }
    }

    fn commit(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        match self.reconstruction.commit(index) {
            Ok(progress) => {
                if progress.recon_complete {
                    let scratch = match self.reconstruction.take_scheduled_scratch() {
                        Ok(scratch) => scratch,
                        Err(error) => {
                            self.fail(error, admit);
                            return;
                        }
                    };
                    let _ = self
                        .scratch_done
                        .set(Mutex::new(Some(T::wrap_scheduled_scratch(scratch))));
                }
                for row in progress.frontier_rows {
                    self.submit_frontier(index, row, admit);
                }
            }
            Err(error) => self.fail(error, admit),
        }
        let next = index.saturating_add(1);
        if !self.failed.load(Ordering::Acquire) && next < self.reconstruction.len() {
            self.continue_commit(next, admit);
        }
    }

    /// Schedules the filter stripes one frontier link released, and the
    /// frame's finish once the final link has released them all.
    fn publish_filters(
        self: &Arc<Self>,
        progress: inter::ScheduledFrameProgress<T>,
        admit: &dyn splot_parallel::Admit<'_>,
    ) {
        for filter in progress.filters {
            let stripe = filter.stripe();
            if self.filtered.get(stripe).is_none() {
                self.fail(
                    unsupported(
                        "inter_admission_filter_index",
                        None,
                        "scheduled filter stripe index is out of range",
                    ),
                    admit,
                );
                break;
            }
            let frame = Arc::clone(self);
            admit.spawn_ready(Box::new(move |_| {
                if let Err(error) = filter.run() {
                    let mut owed = frame
                        .filter_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    if owed.is_none() {
                        *owed = Some(error);
                    }
                }
                let _ = frame.filtered[stripe].set(());
            }));
        }
        let Some(filter) = progress.output else {
            return;
        };
        let finish = self
            .finish
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(finish) = finish {
            let mut conditions = self
                .filter_gate
                .as_deref()
                .map(|gate| vec![Condition::completion(gate)])
                .unwrap_or_default();
            conditions.extend(self.filtered.iter().map(Condition::completion));
            let filter_done = Arc::clone(&self.filter_done);
            let frame = Arc::clone(self);
            admit.submit(
                self.order_base + u64::from(u32::MAX),
                &conditions,
                Box::new(move |_| {
                    let error = frame
                        .filter_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take();
                    if let Some(error) = error {
                        finish.fail(error);
                    } else {
                        finish.run_owned_finish(filter);
                    }
                    let _ = filter_done.set(());
                }),
            );
        }
    }

    fn fail(&self, error: DecodeError, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.reconstruction.fail_temporal();
        self.motion.fail();
        let _ = self.scratch_done.set(Mutex::new(None));
        let _ = self.filter_done.set(());
        for completion in self
            .prepared
            .iter()
            .chain(&self.frontier_done)
            .chain(&self.filtered)
        {
            let _ = completion.set(());
        }
        admit.admit_ready();
        if let Some(finish) = self
            .finish
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            finish.fail(error);
        }
    }
}

fn schedule_typed<'job, 'scope, T: ScheduledScratchSample + Send + 'static>(
    early: inter::InterWalkEarly<T>,
    tail: EntropyResult<T>,
    finish: PendingFinish<T>,
    frame_index: usize,
    scheduler: &'scope AdmissionScheduler<'job>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    let motion = early.motion.clone();
    let dependencies = early.motion_dependencies();
    let (scratch_source, scratch_done) = lane.reserve_recon();
    let (filter_gate, filter_done) = lane.reserve_filter();
    let (temporal_gate, temporal_done) = lane.reserve_temporal();
    let mut conditions = dependencies
        .iter()
        .map(inter::MotionFieldHandle::metadata_condition)
        .collect::<Vec<_>>();
    if let Some(gate) = scratch_source.as_deref() {
        conditions.push(Condition::completion(gate));
    }
    if let Some(gate) = temporal_gate.as_deref() {
        conditions.push(Condition::completion(gate));
    }
    let temporal_source = temporal_gate.clone();
    let scheduled_scratch_source = scratch_source.clone();
    let scratch_done_for_job = Arc::clone(&scratch_done);
    let filter_done_for_job = Arc::clone(&filter_done);
    scheduler.submit(
        scope,
        order_base,
        &conditions,
        Box::new(move |admit| {
            let temporal_scratch = temporal_source
                .as_deref()
                .and_then(CompletionCell::get)
                .and_then(|scratch| {
                    scratch
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                })
                .unwrap_or_default();
            let decode_scratch = scheduled_scratch_source
                .as_deref()
                .and_then(CompletionCell::get)
                .and_then(|scratch| {
                    T::take_scheduled_scratch(
                        &mut scratch.lock().unwrap_or_else(PoisonError::into_inner),
                    )
                })
                .unwrap_or_default();
            let settle_prepare_failure = |finish: PendingFinish<T>, error| {
                let _ = temporal_done.set(Mutex::new(Some(inter::TemporalMvScratch::default())));
                let _ = scratch_done_for_job.set(Mutex::new(None));
                motion.fail();
                let _ = filter_done_for_job.set(());
                finish.fail(error);
            };
            let progress = finish.progress_handle();
            let filters_ready = Arc::new(CompletionCell::new());
            let (scheduled, pending_filters) =
                match early.prepare_scheduled(decode_scratch, temporal_scratch, progress) {
                    Ok((scheduled, temporal_scratch, pending)) => {
                        let _ = temporal_done.set(Mutex::new(Some(temporal_scratch)));
                        admit.admit_ready();
                        (scheduled, pending)
                    }
                    Err(error) => {
                        settle_prepare_failure(finish, error);
                        return;
                    }
                };
            let frame = Arc::new(ScheduledFrame {
                prepared: (0..scheduled.len())
                    .map(|_| CompletionCell::new())
                    .collect(),
                frontier_done: (0..scheduled.frontier_len())
                    .map(|_| CompletionCell::new())
                    .collect(),
                filtered: (0..scheduled.filter_count())
                    .map(|_| CompletionCell::new())
                    .collect(),
                filter_error: Mutex::new(None),
                filters_ready: Arc::clone(&filters_ready),
                reconstruction: scheduled,
                finish: Mutex::new(Some(finish)),
                motion,
                scratch_done: scratch_done_for_job,
                filter_gate,
                filter_done: filter_done_for_job,
                failed: AtomicBool::new(false),
                order_base,
            });
            {
                let attach_frame = Arc::clone(&frame);
                let attach_tail = Arc::clone(&tail);
                let attach_ready = Arc::clone(&filters_ready);
                admit.submit(
                    order_base.saturating_add(1 << 19),
                    &[Condition::completion(tail.as_ref())],
                    Box::new(move |admit| {
                        let parsed = attach_tail.get().and_then(|slot| {
                            slot.lock().unwrap_or_else(PoisonError::into_inner).take()
                        });
                        let outcome = match parsed {
                            Some(Ok(deferred)) => deferred
                                .attach_filters(pending_filters, &attach_frame.reconstruction),
                            Some(Err(error)) => Err(error),
                            None => Err(frame_task_scope()),
                        };
                        match outcome {
                            Ok(()) => {
                                let _ = attach_ready.set(());
                            }
                            Err(error) => {
                                attach_frame.fail(error, admit);
                                let _ = attach_ready.set(());
                            }
                        }
                    }),
                );
            }
            if frame.reconstruction.resolve_len() == 0 {
                frame.fail(
                    unsupported(
                        "inter_admission_temporal_band_count",
                        None,
                        "scheduled temporal projection has no row bands",
                    ),
                    admit,
                );
            } else {
                frame.submit_resolve(0, admit);
            }
        }),
    );
}

pub(super) fn schedule_finish<'job, 'scope, T: splot_recon::ReconSample + Send + 'static>(
    finish: PendingFinish<T>,
    walked: super::frame_engine::finish::WalkedFrame<T>,
    frame_index: usize,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    'job: 'scope,
{
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    let (gate, done) = lane.reserve_filter();
    let conditions = gate
        .as_deref()
        .map(|gate| vec![Condition::completion(gate)])
        .unwrap_or_default();
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_base + u64::from(u32::MAX),
        &conditions,
        Box::new(move |admit| {
            finish.run_finish(walked, Some(admit));
            let _ = done.set(());
        }),
    );
    Ok(())
}

fn promote_front<'scope, 'job>(
    entropy: &mut PendingEntropyQueue,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    'job: 'scope,
{
    let Some(pending) = entropy.entries.pop_front() else {
        return Ok(());
    };
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    match pending {
        PendingEntropy::Eight {
            frame_index,
            result: EntropyHandles { early, tail },
            finish,
        } => {
            let settled = early.wait_with_pool_assist();
            let early = settled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(early) = early {
                schedule_typed(early, tail, finish, frame_index, scheduler, scope, lane);
            } else {
                let error = tail
                    .get()
                    .and_then(|slot| {
                        slot.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .take()
                            .and_then(Result::err)
                    })
                    .unwrap_or_else(frame_task_scope);
                finish.fail(error);
            }
        }
        PendingEntropy::Ten {
            frame_index,
            result: EntropyHandles { early, tail },
            finish,
        } => {
            let settled = early.wait_with_pool_assist();
            let early = settled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(early) = early {
                schedule_typed(early, tail, finish, frame_index, scheduler, scope, lane);
            } else {
                let error = tail
                    .get()
                    .and_then(|slot| {
                        slot.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .take()
                            .and_then(Result::err)
                    })
                    .unwrap_or_else(frame_task_scope);
                finish.fail(error);
            }
        }
    }
    Ok(())
}

fn drain_ready_entropy<'scope, 'job>(
    entropy: &mut PendingEntropyQueue,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    'job: 'scope,
{
    loop {
        while entropy
            .entries
            .front()
            .is_some_and(PendingEntropy::is_settled)
        {
            promote_front(entropy, spawner, scheduler, lane)?;
        }
        if entropy.entries.is_empty() || !splot_parallel::assist_pool_once() {
            return Ok(());
        }
    }
}

/// Opens one bounded entropy-context slot before the caller reserves frame storage.
pub(super) fn prepare_entropy_submission<'scope, 'job>(
    entropy: &mut PendingEntropyQueue,
    limit: usize,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    'job: 'scope,
{
    let limit = limit.max(1);
    drain_ready_entropy(entropy, spawner, scheduler, lane)?;
    while entropy.entries.len() >= limit {
        promote_front(entropy, spawner, scheduler, lane)?;
    }
    Ok(())
}

/// Promotes every pending entropy context before a non-inter frame or output barrier.
pub(super) fn drain_entropy_before_barrier<'scope, 'job>(
    entropy: &mut PendingEntropyQueue,
    spawner: &FinishSpawner<'_, 'scope>,
    admission: Option<&'scope AdmissionScheduler<'job>>,
    lane: &mut ReconAdmissionLane,
) -> Result<()>
where
    'job: 'scope,
{
    if entropy.entries.is_empty() {
        return Ok(());
    }
    let scheduler = admission.ok_or_else(frame_task_scope)?;
    while !entropy.entries.is_empty() {
        promote_front(entropy, spawner, scheduler, lane)?;
    }
    Ok(())
}

/// Shares the driver's active sequence header with the frames it defers.
///
/// A deferred frame's reconstruction outlives the driver's borrow of the
/// header, and the header only ever changes at a frame the driver flushes
/// before, so the shared copy is made once per activation.
pub(super) fn shared_sequence(
    cached: &mut Option<Arc<splot_core::headers::sequence::SequenceHeader>>,
    sequence: &splot_core::headers::sequence::SequenceHeader,
) -> Arc<splot_core::headers::sequence::SequenceHeader> {
    Arc::clone(cached.get_or_insert_with(|| Arc::new(sequence.clone())))
}

fn frame_task_scope() -> DecodeError {
    unsupported(
        "frame_parse_task_scope",
        None,
        "internal invariant violation: a frame entropy pass task did not report an outcome",
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn recon_lane_returns_the_same_typed_context_at_bounded_depth() {
        let mut lane = ReconAdmissionLane::new(1);
        let (prior, first) = lane.reserve_recon();
        assert!(prior.is_none());
        assert!(
            first
                .set(Mutex::new(Some(ScheduledReconScratch::Ten(
                    inter::InterDecodeScratch::default(),
                ))))
                .is_ok()
        );

        let (prior, _) = lane.reserve_recon();
        assert_eq!(lane.recon.len(), 1);
        let prior = prior.expect("prior context");
        let stored = prior.get().expect("settled prior context");
        let mut stored = stored.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(u8::take_scheduled_scratch(&mut stored).is_none());
        assert!(u16::take_scheduled_scratch(&mut stored).is_some());
    }

    #[test]
    fn failed_recon_context_still_settles_the_lane() {
        let mut lane = ReconAdmissionLane::new(1);
        let (_, failed) = lane.reserve_recon();
        assert!(failed.set(Mutex::new(None)).is_ok());

        let (prior, _) = lane.reserve_recon();
        assert!(
            prior
                .expect("failed prior context")
                .get()
                .expect("settled failure")
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_none()
        );
    }
}
