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

impl PendingWalk {
    /// Runs the owed reconstruction and hands the frame's § 7.2 filter phase
    /// on, publishing the frame's motion field on the way.
    ///
    /// # Errors
    ///
    /// Returns the reconstruction's or an inline filter phase's diagnostic.
    pub(super) fn resume(
        self,
        spawner: &FinishSpawner<'_, '_>,
        scratch_eight: &mut inter::InterDecodeScratch<u8>,
        scratch_ten: &mut inter::InterDecodeScratch<u16>,
    ) -> Result<()> {
        let started = crate::timing::start();
        match self {
            Self::Eight {
                deferred, finish, ..
            } => {
                let walked = deferred.reconstruct(scratch_eight)?;
                match spawner {
                    FinishSpawner::Deferred(scope) => finish.spawn_finish(walked, scope),
                    FinishSpawner::Inline => {
                        scratch_eight.recycle_frame_filter_records(finish.finish_inline(walked)?);
                    }
                }
            }
            Self::Ten {
                deferred, finish, ..
            } => {
                let walked = deferred.reconstruct(scratch_ten)?;
                match spawner {
                    FinishSpawner::Deferred(scope) => finish.spawn_finish(walked, scope),
                    FinishSpawner::Inline => {
                        scratch_ten.recycle_frame_filter_records(finish.finish_inline(walked)?);
                    }
                }
            }
        }
        crate::timing::report("pass2_span", started);
        Ok(())
    }
}

const ORDER_KEY_FRAME_STRIDE: u64 = 1 << 32;
type TemporalScratchSlot = Arc<CompletionCell<Mutex<Option<inter::TemporalMvScratch>>>>;

/// The three-frame admission bound for scheduled reconstruction.
#[derive(Default)]
pub(super) struct ReconAdmissionLane {
    recon: VecDeque<Arc<CompletionCell<()>>>,
    filters: VecDeque<Arc<CompletionCell<()>>>,
    temporal: Option<TemporalScratchSlot>,
}

impl ReconAdmissionLane {
    fn reserve(
        lane: &mut VecDeque<Arc<CompletionCell<()>>>,
    ) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        let gate = (lane.len() >= 3).then(|| Arc::clone(&lane[0]));
        let done = Arc::new(CompletionCell::new());
        lane.push_back(Arc::clone(&done));
        while lane.len() > 3 {
            lane.pop_front();
        }
        (gate, done)
    }

    fn reserve_recon(&mut self) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        Self::reserve(&mut self.recon)
    }

    fn reserve_filter(&mut self) -> (Option<Arc<CompletionCell<()>>>, Arc<CompletionCell<()>>) {
        Self::reserve(&mut self.filters)
    }

    fn reserve_temporal(&mut self) -> (Option<TemporalScratchSlot>, TemporalScratchSlot) {
        let prior = self.temporal.take();
        let done = Arc::new(CompletionCell::new());
        self.temporal = Some(Arc::clone(&done));
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
    failed: AtomicBool,
    order_base: u64,
}

impl<T: splot_recon::ReconSample + Send + 'static> ScheduledFrame<T> {
    fn precompute(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'static>) {
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

    fn commit(self: &Arc<Self>, index: usize, admit: &dyn splot_parallel::Admit<'static>) {
        if self.failed.load(Ordering::Acquire) {
            if let Some(committed) = self.committed.get(index) {
                let _ = committed.set(());
            }
            admit.admit_ready();
            return;
        }
        match self.walk.commit(index) {
            Ok(Some(walked)) => {
                let _ = self.recon_done.set(());
                let finish = self
                    .finish
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(finish) = finish {
                    let conditions = self
                        .filter_gate
                        .as_deref()
                        .map(|gate| vec![Condition::Completion(gate)])
                        .unwrap_or_default();
                    let filter_done = Arc::clone(&self.filter_done);
                    admit.submit(
                        self.order_base + u64::from(u32::MAX),
                        &conditions,
                        Box::new(move |admit| {
                            finish.run_finish(walked, Some(admit));
                            let _ = filter_done.set(());
                            admit.admit_ready();
                        }),
                    );
                }
            }
            Ok(None) => {}
            Err(error) => self.fail(error, admit),
        }
        if let Some(committed) = self.committed.get(index) {
            let _ = committed.set(());
        }
        admit.admit_ready();
    }

    fn fail(&self, error: DecodeError, admit: &dyn splot_parallel::Admit<'static>) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.motion.fail();
        let _ = self.recon_done.set(());
        let _ = self.filter_done.set(());
        for completion in self.prepared.iter().chain(&self.committed) {
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

fn schedule_typed<'scope, T: splot_recon::ReconSample + Send + 'static>(
    deferred: inter::DeferredInterWalk<T>,
    finish: PendingFinish<T>,
    frame_index: usize,
    scheduler: &'scope AdmissionScheduler<'static>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) -> Arc<CompletionCell<()>> {
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
        .map(inter::MotionFieldHandle::condition)
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
            let scheduled = match deferred.prepare_scheduled(temporal_scratch) {
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
                walk: scheduled,
                finish: Mutex::new(Some(finish)),
                motion,
                recon_done: done_for_job,
                filter_gate,
                filter_done: filter_done_for_job,
                failed: AtomicBool::new(false),
                order_base,
            });
            for index in 0..frame.walk.len() {
                let precompute_conditions = frame.walk.conditions(index);
                let precompute = Arc::clone(&frame);
                let index_key = u64::try_from(index).unwrap_or(u64::MAX / 2);
                admit.submit(
                    order_base.saturating_add(index_key.saturating_mul(2).saturating_add(1)),
                    &precompute_conditions,
                    Box::new(move |admit| precompute.precompute(index, admit)),
                );
                let mut commit_conditions =
                    vec![Condition::Completion(frame.prepared[index].as_ref())];
                if index > 0 {
                    commit_conditions
                        .push(Condition::Completion(frame.committed[index - 1].as_ref()));
                }
                let commit = Arc::clone(&frame);
                admit.submit(
                    order_base.saturating_add(index_key.saturating_mul(2).saturating_add(2)),
                    &commit_conditions,
                    Box::new(move |admit| commit.commit(index, admit)),
                );
            }
        }),
    );
    recon_done
}

pub(super) fn schedule_finish<'scope, T: splot_recon::ReconSample + Send + 'static>(
    finish: PendingFinish<T>,
    walked: super::frame_engine::finish::WalkedFrame<T>,
    frame_index: usize,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'static>,
    lane: &mut ReconAdmissionLane,
) -> Result<()> {
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

fn schedule_pending<'scope>(
    pending: PendingWalk,
    spawner: &FinishSpawner<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'static>,
    lane: &mut ReconAdmissionLane,
) -> Result<Arc<CompletionCell<()>>> {
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

/// Runs any owed reconstruction now, so the driver reaches a program point that
/// reads decoded samples with every frame before it complete.
///
/// # Errors
///
/// Returns the owed reconstruction's diagnostic.
pub(super) fn flush_pending<'scope>(
    pending: &mut Option<PendingWalk>,
    spawner: &FinishSpawner<'_, 'scope>,
    admission: Option<(&'scope AdmissionScheduler<'static>, &mut ReconAdmissionLane)>,
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<()> {
    let Some(owed) = pending.take() else {
        return Ok(());
    };
    match admission {
        Some((scheduler, lane)) => {
            let done = schedule_pending(owed, spawner, scheduler, lane)?;
            let () = done.wait_with_pool_assist();
            Ok(())
        }
        None => owed.resume(spawner, scratch_eight, scratch_ten),
    }
}

/// Runs one frame's entropy pass beside the previous frame's reconstruction.
///
/// The entropy pass is spawned as a single ready task and the driver runs the
/// owed reconstruction itself, so the driver may steal the entropy task while
/// its own assisted waits donate to the pool. That is safe in one direction
/// only, and it is the direction this takes: the entropy pass waits on nothing,
/// while the reconstruction waits on reference rows the frames below it publish.
///
/// A reconstruction failure outranks an entropy-pass failure, since the frame it
/// belongs to decodes first.
///
/// # Errors
///
/// Returns the lower-indexed frame's diagnostic, or the scope's own when the
/// caller is not on a worker pool.
pub(super) fn parse_beside_pending<'scope, P: Send>(
    parse: impl FnOnce() -> P + Send,
    pending: Option<PendingWalk>,
    spawner: &FinishSpawner<'_, 'scope>,
    admission: Option<(&'scope AdmissionScheduler<'static>, &mut ReconAdmissionLane)>,
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<P> {
    let Some(pending) = pending else {
        return Ok(parse());
    };
    if let Some((scheduler, lane)) = admission {
        schedule_pending(pending, spawner, scheduler, lane)?;
        return Ok(parse());
    }
    let mut parsed = None;
    let mut resumed = None;
    let mut joined = None;
    let driver = std::thread::current().id();
    let stolen = std::sync::atomic::AtomicBool::new(false);
    let parsed_slot = &mut parsed;
    let resumed_slot = &mut resumed;
    let joined_slot = &mut joined;
    let stolen_flag = &stolen;
    splot_parallel::ready_task_scope(|scope| {
        scope.spawn(move |_| {
            if std::thread::current().id() == driver {
                stolen_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            *parsed_slot = Some(parse());
        });
        *resumed_slot = Some(pending.resume(spawner, scratch_eight, scratch_ten));
        *joined_slot = crate::timing::start();
    })
    .map_err(|_| frame_task_scope())?;
    if joined.is_some() {
        crate::timing::report_detail(
            "parse_join_wait",
            joined,
            if stolen.load(std::sync::atomic::Ordering::Relaxed) {
                "on=driver"
            } else {
                "on=worker"
            },
        );
    }
    resumed.ok_or_else(frame_task_scope)??;
    parsed.ok_or_else(frame_task_scope)
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
