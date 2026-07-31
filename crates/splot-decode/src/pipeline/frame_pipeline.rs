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

use splot_parallel::{AdmissionScheduler, CompletionCell, Condition};

use crate::Result;
use crate::error::DecodeError;
use crate::prediction::inter;

use super::inflight::{FinishSpawner, PendingFinish};
use super::unsupported;

/// One frame whose entropy pass is done and whose reconstruction the driver
/// still owes, in the pipeline's active sample storage bit depth.
pub(super) enum PendingWalk {
    /// Eight-bit sample storage.
    Eight {
        frame_index: usize,
        deferred: inter::DeferredInterWalk<u8>,
        finish: PendingFinish<u8>,
    },
    /// Ten-bit sample storage.
    Ten {
        frame_index: usize,
        deferred: inter::DeferredInterWalk<u16>,
        finish: PendingFinish<u16>,
    },
}

type EntropyResult<T> = Arc<CompletionCell<Mutex<Option<Result<inter::DeferredInterWalk<T>>>>>>;

/// One scheduler-owned entropy pass whose reconstruction has not been promoted.
pub(super) enum PendingEntropy {
    Eight {
        frame_index: usize,
        result: EntropyResult<u8>,
        finish: PendingFinish<u8>,
    },
    Ten {
        frame_index: usize,
        result: EntropyResult<u16>,
        finish: PendingFinish<u16>,
    },
}

impl PendingEntropy {
    fn is_settled(&self) -> bool {
        match self {
            Self::Eight { result, .. } => result.get().is_some(),
            Self::Ten { result, .. } => result.get().is_some(),
        }
    }

    fn promote(self) -> Result<Option<PendingWalk>> {
        match self {
            Self::Eight {
                frame_index,
                result,
                finish,
            } => promote_typed(
                frame_index,
                &result,
                finish,
                |frame_index, deferred, finish| PendingWalk::Eight {
                    frame_index,
                    deferred,
                    finish,
                },
            ),
            Self::Ten {
                frame_index,
                result,
                finish,
            } => promote_typed(
                frame_index,
                &result,
                finish,
                |frame_index, deferred, finish| PendingWalk::Ten {
                    frame_index,
                    deferred,
                    finish,
                },
            ),
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

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn promote_typed<T: splot_recon::ReconSample + Send + 'static>(
    frame_index: usize,
    result: &EntropyResult<T>,
    finish: PendingFinish<T>,
    wrap: impl FnOnce(usize, inter::DeferredInterWalk<T>, PendingFinish<T>) -> PendingWalk,
) -> Result<Option<PendingWalk>> {
    let settled = result.wait_with_pool_assist();
    let parsed = settled
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or_else(frame_task_scope)?;
    match parsed {
        Ok(deferred) => Ok(Some(wrap(frame_index, deferred, finish))),
        Err(error) => {
            finish.fail(error);
            Ok(None)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_entropy<'scope, 'job, T, P>(
    parse: P,
    frame_index: usize,
    frame_cdfs: inter::FrameCdfHandle,
    ccso_grid: inter::CcsoGridHandle,
    motion: inter::MotionFieldHandle,
    dependencies: &inter::EntropyDependencies,
    scheduler: &'scope AdmissionScheduler<'job>,
    spawner: &FinishSpawner<'_, 'scope>,
) -> Result<EntropyResult<T>>
where
    T: splot_recon::ReconSample + Send + 'static,
    P: FnOnce() -> Result<inter::DeferredInterWalk<T>> + Send + 'job,
    'job: 'scope,
{
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    let result = Arc::new(CompletionCell::new());
    let result_for_job = Arc::clone(&result);
    let failed_cdfs = frame_cdfs.clone();
    let failed_ccso = ccso_grid.clone();
    let failed_motion = motion;
    let conditions = dependencies.conditions();
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_key,
        &conditions,
        Box::new(move |admit| {
            let parsed = parse();
            if let Ok(deferred) = &parsed {
                frame_cdfs.publish(Arc::clone(&deferred.frame_cdfs));
                ccso_grid.publish(deferred.ccso_grid.clone().map(Arc::new));
            } else {
                failed_cdfs.fail();
                failed_ccso.fail();
                failed_motion.fail();
            }
            let _ = result_for_job.set(Mutex::new(Some(parsed)));
            admit.admit_ready();
        }),
    );
    Ok(result)
}

const ORDER_KEY_FRAME_STRIDE: u64 = 1 << 32;
type TemporalScratchSlot = Arc<CompletionCell<Mutex<Option<inter::TemporalMvScratch>>>>;

/// The frame-context admission bound for scheduled reconstruction.
pub(super) struct ReconAdmissionLane {
    depth: usize,
    recon: VecDeque<Arc<CompletionCell<()>>>,
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

    fn reserve(
        depth: usize,
        lane: &mut VecDeque<Arc<CompletionCell<()>>>,
    ) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        let gate = (lane.len() >= depth).then(|| Arc::clone(&lane[0]));
        let done = Arc::new(CompletionCell::new());
        lane.push_back(Arc::clone(&done));
        while lane.len() > depth {
            lane.pop_front();
        }
        (gate, done)
    }

    fn reserve_recon(&mut self) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        Self::reserve(self.depth, &mut self.recon)
    }

    fn reserve_filter(&mut self) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        Self::reserve(self.depth, &mut self.filters)
    }

    fn reserve_temporal(&mut self) -> (Option<TemporalScratchSlot>, TemporalScratchSlot) {
        let prior = (self.temporal.len() >= self.depth).then(|| Arc::clone(&self.temporal[0]));
        let done = Arc::new(CompletionCell::new());
        self.temporal.push_back(Arc::clone(&done));
        while self.temporal.len() > self.depth {
            self.temporal.pop_front();
        }
        (prior, done)
    }
}

struct ScheduledFrame<T: splot_recon::ReconSample> {
    walk: inter::ScheduledInterWalk<T>,
    finish: Mutex<Option<PendingFinish<T>>>,
    motion: inter::MotionFieldHandle,
    recon_done: Arc<CompletionCell<()>>,
    filter_gate: Option<Arc<CompletionCell<()>>>,
    filter_done: Arc<CompletionCell<()>>,
    prepared: Vec<Arc<CompletionCell<()>>>,
    committed: Vec<Arc<CompletionCell<()>>>,
    frontier_done: Vec<Arc<CompletionCell<()>>>,
    filtered: Vec<Arc<CompletionCell<()>>>,
    filter_error: Mutex<Option<DecodeError>>,
    failed: AtomicBool,
    order_base: u64,
}

impl<T: splot_recon::ReconSample + Send + 'static> ScheduledFrame<T> {
    fn submit_batch(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        let mut precompute_conditions = self.walk.conditions(index);
        if self.walk.owns_canonical_bands() && index > 0 {
            precompute_conditions.push(Condition::Completion(self.committed[index - 1].as_ref()));
        }
        let row = Arc::clone(self);
        admit.submit(
            self.batch_key(index, 1),
            &precompute_conditions,
            if self.walk.owns_canonical_bands() {
                Box::new(move |admit| row.run_batch(index, admit))
            } else {
                Box::new(move |admit| row.precompute(index, admit))
            },
        );
        if self.walk.owns_canonical_bands() {
            return;
        }
        let mut commit_conditions = vec![Condition::Completion(self.prepared[index].as_ref())];
        if index > 0 {
            commit_conditions.push(Condition::Completion(self.committed[index - 1].as_ref()));
        }
        let commit = Arc::clone(self);
        admit.submit(
            self.batch_key(index, 2),
            &commit_conditions,
            Box::new(move |admit| commit.commit(index, admit)),
        );
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
            .map(|previous| vec![Condition::Completion(previous.as_ref())])
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
            match self.walk.frontier(row) {
                Ok(progress) => self.publish_filters(progress, admit),
                Err(error) => self.fail(error, admit),
            }
        }
        if let Some(done) = self.frontier_done.get(row) {
            let _ = done.set(());
        }
        admit.admit_ready();
    }

    fn submit_resolve(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        let conditions = self.walk.resolve_conditions(index);
        let resolve = Arc::clone(self);
        let index_key = u64::try_from(index).unwrap_or(u64::MAX / 2);
        admit.submit(
            self.order_base.saturating_add(index_key),
            &conditions,
            Box::new(move |admit| resolve.resolve(index, admit)),
        );
    }

    fn resolve(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.load(Ordering::Acquire) {
            admit.admit_ready();
            return;
        }
        let batches = match self.walk.resolve(index) {
            Ok(batches) => batches,
            Err(error) => {
                self.fail(error, admit);
                return;
            }
        };
        for batch in batches {
            self.submit_batch(batch, admit);
        }
        let next = index.saturating_add(1);
        if next < self.walk.resolve_len() {
            self.submit_resolve(next, admit);
        }
        admit.admit_ready();
    }

    fn run_batch(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if !self.failed.load(Ordering::Acquire)
            && let Err(error) = self.walk.precompute(index)
        {
            self.fail(error, admit);
            return;
        }
        if let Some(prepared) = self.prepared.get(index) {
            let _ = prepared.set(());
        }
        self.commit(index, admit);
    }

    fn precompute(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if !self.failed.load(Ordering::Acquire)
            && let Err(error) = self.walk.precompute(index)
        {
            self.fail(error, admit);
        }
        if let Some(prepared) = self.prepared.get(index) {
            let _ = prepared.set(());
        }
        admit.admit_ready();
    }

    fn commit(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.load(Ordering::Acquire) {
            if let Some(committed) = self.committed.get(index) {
                let _ = committed.set(());
            }
            admit.admit_ready();
            return;
        }
        match self.walk.commit(index) {
            Ok(progress) => {
                if progress.recon_complete {
                    let _ = self.recon_done.set(());
                }
                for row in progress.frontier_rows {
                    self.submit_frontier(index, row, admit);
                }
            }
            Err(error) => self.fail(error, admit),
        }
        if let Some(committed) = self.committed.get(index) {
            let _ = committed.set(());
        }
        admit.admit_ready();
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
            let Some(done) = self.filtered.get(stripe).cloned() else {
                self.fail(
                    unsupported(
                        "inter_admission_filter_index",
                        None,
                        "scheduled filter stripe index is out of range",
                    ),
                    admit,
                );
                break;
            };
            let frame = Arc::clone(self);
            admit.submit(
                self.order_base
                    .saturating_add(u64::try_from(stripe).unwrap_or(u64::MAX / 2) * 2 + 2),
                &[],
                Box::new(move |admit| {
                    if let Err(error) = filter.run() {
                        let mut owed = frame
                            .filter_error
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        if owed.is_none() {
                            *owed = Some(error);
                        }
                    }
                    let _ = done.set(());
                    admit.admit_ready();
                }),
            );
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
                .map(|gate| vec![Condition::Completion(gate)])
                .unwrap_or_default();
            conditions.extend(
                self.filtered
                    .iter()
                    .map(|done| Condition::Completion(done.as_ref())),
            );
            let filter_done = Arc::clone(&self.filter_done);
            let frame = Arc::clone(self);
            admit.submit(
                self.order_base + u64::from(u32::MAX),
                &conditions,
                Box::new(move |admit| {
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
                    admit.admit_ready();
                }),
            );
        }
    }

    fn fail(&self, error: DecodeError, admit: &dyn splot_parallel::Admit<'_>) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        let failure_started = crate::timing::start();
        if failure_started.is_some() {
            crate::timing::report_detail(
                "inter_admission_failure",
                failure_started,
                &format!("error={error}"),
            );
        }
        self.walk.fail_temporal();
        self.motion.fail();
        let _ = self.recon_done.set(());
        let _ = self.filter_done.set(());
        for completion in self
            .prepared
            .iter()
            .chain(&self.committed)
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

fn schedule_typed<'job, 'scope, T: splot_recon::ReconSample + Send + 'static>(
    deferred: inter::DeferredInterWalk<T>,
    finish: PendingFinish<T>,
    frame_index: usize,
    scheduler: &'scope AdmissionScheduler<'job>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) -> Arc<CompletionCell<()>>
where
    'job: 'scope,
{
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    let motion = deferred.motion.clone();
    let dependencies = deferred.motion_dependencies();
    let (gate, recon_done) = lane.reserve_recon();
    let (filter_gate, filter_done) = lane.reserve_filter();
    let (temporal_gate, temporal_done) = lane.reserve_temporal();
    let mut conditions = dependencies
        .iter()
        .map(inter::MotionFieldHandle::metadata_condition)
        .collect::<Vec<_>>();
    if let Some(gate) = gate.as_deref() {
        conditions.push(Condition::Completion(gate));
    }
    if let Some(gate) = temporal_gate.as_deref() {
        conditions.push(Condition::Completion(gate));
    }
    let temporal_source = temporal_gate.clone();
    let done_for_job = Arc::clone(&recon_done);
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
            let Some(progress) = finish.progress_handle() else {
                let _ = temporal_done.set(Mutex::new(Some(inter::TemporalMvScratch::default())));
                motion.fail();
                let _ = done_for_job.set(());
                let _ = filter_done_for_job.set(());
                finish.fail(unsupported(
                    "inter_admission_filter_progress",
                    None,
                    "scheduled filter progress handle is missing",
                ));
                admit.admit_ready();
                return;
            };
            let scheduled = match deferred.prepare_scheduled(temporal_scratch, progress) {
                Ok((scheduled, temporal_scratch)) => {
                    let _ = temporal_done.set(Mutex::new(Some(temporal_scratch)));
                    admit.admit_ready();
                    scheduled
                }
                Err(error) => {
                    let _ =
                        temporal_done.set(Mutex::new(Some(inter::TemporalMvScratch::default())));
                    motion.fail();
                    let _ = done_for_job.set(());
                    let _ = filter_done_for_job.set(());
                    finish.fail(error);
                    admit.admit_ready();
                    return;
                }
            };
            let frame = Arc::new(ScheduledFrame {
                prepared: (0..scheduled.len())
                    .map(|_| Arc::new(CompletionCell::new()))
                    .collect(),
                committed: (0..scheduled.len())
                    .map(|_| Arc::new(CompletionCell::new()))
                    .collect(),
                frontier_done: (0..scheduled.frontier_len())
                    .map(|_| Arc::new(CompletionCell::new()))
                    .collect(),
                filtered: (0..scheduled.filter_count())
                    .map(|_| Arc::new(CompletionCell::new()))
                    .collect(),
                filter_error: Mutex::new(None),
                walk: scheduled,
                finish: Mutex::new(Some(finish)),
                motion,
                recon_done: done_for_job,
                filter_gate,
                filter_done: filter_done_for_job,
                failed: AtomicBool::new(false),
                order_base,
            });
            if frame.walk.resolve_len() == 0 {
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
    recon_done
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
        .map(|gate| vec![Condition::Completion(gate)])
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
            admit.admit_ready();
        }),
    );
    Ok(())
}

fn schedule_pending<'job, 'scope>(
    pending: PendingWalk,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<Arc<CompletionCell<()>>>
where
    'job: 'scope,
{
    let FinishSpawner::Deferred(scope) = spawner else {
        return Err(frame_task_scope());
    };
    Ok(match pending {
        PendingWalk::Eight {
            frame_index,
            deferred,
            finish,
        } => schedule_typed(deferred, finish, frame_index, scheduler, scope, lane),
        PendingWalk::Ten {
            frame_index,
            deferred,
            finish,
        } => schedule_typed(deferred, finish, frame_index, scheduler, scope, lane),
    })
}

fn promote_front<'scope, 'job>(
    entropy: &mut PendingEntropyQueue,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job>,
    lane: &mut ReconAdmissionLane,
) -> Result<Option<Arc<CompletionCell<()>>>>
where
    'job: 'scope,
{
    let Some(pending) = entropy.entries.pop_front() else {
        return Ok(None);
    };
    pending
        .promote()?
        .map(|walk| schedule_pending(walk, spawner, scheduler, lane))
        .transpose()
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
    while entropy
        .entries
        .front()
        .is_some_and(PendingEntropy::is_settled)
    {
        let _ = promote_front(entropy, spawner, scheduler, lane)?;
    }
    Ok(())
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
        let _ = promote_front(entropy, spawner, scheduler, lane)?;
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
    if entropy.is_empty() {
        return Ok(());
    }
    let scheduler = admission.ok_or_else(frame_task_scope)?;
    while !entropy.is_empty() {
        let _ = promote_front(entropy, spawner, scheduler, lane)?;
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
