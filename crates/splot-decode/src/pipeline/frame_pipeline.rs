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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_parallel::{AdmissionScheduler, CompletionCell, Condition};
use splot_recon::BitDepth;

use crate::Result;
use crate::error::DecodeError;
use crate::prediction::inter;

use super::inflight::{InflightRing, PendingFinish, PipelineFrameSlot, RefFrameSlot};
use super::unsupported;
use parking_lot::Mutex;

type EntropyResult<T> = Arc<CompletionCell<Mutex<Option<Result<inter::DeferredInterWalk<T>>>>>>;
type EntropyEarly<T> = Arc<CompletionCell<Mutex<Option<inter::InterWalkEarly<T>>>>>;

/// The two halves one scheduled entropy pass publishes: the pre-parse half the
/// admission scheduler is built from, and the pass's own products.
pub(super) struct EntropyHandles<'job, T: splot_recon::ReconSample> {
    early: EntropyEarly<T>,
    tail: EntropyResult<T>,
    context: EntropySlot<'job, T>,
}

/// One scheduler-owned entropy pass whose reconstruction has not been promoted.
pub(super) enum PendingEntropy<'job> {
    Eight {
        frame_index: usize,
        result: EntropyHandles<'job, u8>,
        finish: PendingFinish<u8>,
    },
    Ten {
        frame_index: usize,
        result: EntropyHandles<'job, u16>,
        finish: PendingFinish<u16>,
    },
}

impl PendingEntropy<'_> {
    fn is_settled(&self) -> bool {
        match self {
            Self::Eight { result, .. } => result.early.get().is_some(),
            Self::Ten { result, .. } => result.early.get().is_some(),
        }
    }
}

/// Ordered, bounded entropy contexts awaiting reconstruction admission.
pub(super) type PendingEntropyQueue<'job> = VecDeque<PendingEntropy<'job>>;

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_entropy<'scope, 'job, T: ScheduledScratchSample + Send + 'static>(
    start: inter::InterFrameStart<'job, T>,
    context: EntropySlot<'job, T>,
    frame_index: usize,
    motion: inter::MotionFieldHandle,
    dependencies: &inter::EntropyDependencies,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
) -> EntropyHandles<'job, T>
where
    'job: 'scope,
{
    let result = EntropyHandles {
        early: Arc::clone(&context.early),
        tail: Arc::clone(&context.tail),
        context: Arc::clone(&context),
    };
    *context.task.lock() = Some(EntropyTask { start, motion });
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit_iter(
        scope,
        order_key,
        &mut dependencies.condition_iter(),
        splot_parallel::Job::Inline(T::parse_task(context)),
    );
    result
}

pub(crate) type EntropySlot<'job, T> = Arc<EntropyContext<'job, T>>;

pub(crate) struct EntropyContext<'job, T: splot_recon::ReconSample> {
    task: Mutex<Option<EntropyTask<'job, T>>>,
    frame: Mutex<Option<Arc<ScheduledFrame<T>>>>,
    workspace: Mutex<inter::ScheduledTileWorkspace<T>>,
    temporal: Mutex<Arc<inter::TemporalMvContext>>,
    workers: Arc<inter::InterReconScratchPool<T>>,
    prepare: Mutex<Option<ScheduledPrepare<T>>>,
    attach: Mutex<Option<ScheduledAttach<T>>>,
    early: EntropyEarly<T>,
    tail: EntropyResult<T>,
}

pub(super) struct EntropyContexts<'job, T: splot_recon::ReconSample> {
    slots: Vec<EntropySlot<'job, T>>,
    workers: Option<Arc<inter::InterReconScratchPool<T>>>,
    next: usize,
    depth: usize,
}

impl<'job, T: splot_recon::ReconSample> EntropyContexts<'job, T> {
    pub(super) fn new(depth: usize) -> Self {
        Self {
            slots: Vec::new(),
            workers: None,
            next: 0,
            depth: depth.max(1),
        }
    }

    pub(super) fn claim(&mut self) -> EntropySlot<'job, T> {
        let index = self.next;
        self.next = (index + 1) % self.depth;
        if index == self.slots.len() {
            let workers = self.workers.get_or_insert_with(Arc::default);
            self.slots.push(Arc::new(EntropyContext {
                task: Mutex::new(None),
                frame: Mutex::new(None),
                workspace: Mutex::new(inter::ScheduledTileWorkspace::default()),
                temporal: Mutex::new(Arc::new(inter::TemporalMvContext::empty())),
                workers: Arc::clone(workers),
                prepare: Mutex::new(None),
                attach: Mutex::new(None),
                early: Arc::new(CompletionCell::new()),
                tail: Arc::new(CompletionCell::new()),
            }));
        }
        loop {
            if let Some(context) = Arc::get_mut(&mut self.slots[index])
                && context.reset()
            {
                return Arc::clone(&self.slots[index]);
            }
            if !splot_parallel::assist_pool_once() {
                std::thread::yield_now();
            }
        }
    }
}

impl<T: splot_recon::ReconSample> EntropyContext<'_, T> {
    fn reset(&mut self) -> bool {
        let (Some(early), Some(tail)) =
            (Arc::get_mut(&mut self.early), Arc::get_mut(&mut self.tail))
        else {
            return false;
        };
        if let Some(frame) = self.frame.get_mut() {
            let Some(frame) = Arc::get_mut(frame) else {
                return false;
            };
            if let Some(active) = frame.active.take() {
                *self.workspace.get_mut() = active.reconstruction.retire();
            }
            frame.reset();
        }
        if Arc::get_mut(self.temporal.get_mut()).is_none() {
            return false;
        }
        if !self.workspace.get_mut().producer_storage_reusable() {
            return false;
        }
        if !self.workspace.get_mut().retire_reference_handles() {
            return false;
        }
        early.reset();
        tail.reset();
        self.task.get_mut().take();
        self.prepare.get_mut().take();
        self.attach.get_mut().take();
        true
    }
}

impl<T: splot_recon::ReconSample> EntropyContexts<'_, T> {
    pub(super) fn retire_completed(&mut self) {
        for context in &mut self.slots {
            if let Some(context) = Arc::get_mut(context) {
                if let Some(frame) = context.frame.get_mut().as_mut().and_then(Arc::get_mut)
                    && let Some(active) = frame.active.take()
                {
                    *context.workspace.get_mut() = active.reconstruction.retire();
                }
                context.workspace.get_mut().retire_reference_handles();
            }
        }
    }
}

struct EntropyTask<'job, T: splot_recon::ReconSample> {
    start: inter::InterFrameStart<'job, T>,
    motion: inter::MotionFieldHandle,
}

impl<'job, T: ScheduledScratchSample + Send + 'static> EntropyTask<'job, T> {
    fn run(context: &EntropyContext<'job, T>) {
        let Some(task) = context.task.lock().take() else {
            return;
        };
        let started = {
            let mut workspace = context.workspace.lock();
            task.start.run(&mut workspace)
        };
        let parsed = started
            .and_then(|(early, pending)| {
                let _ = context.early.set(Mutex::new(Some(early)));
                pending.run()
            })
            .and_then(|mut deferred| {
                deferred.publish_products()?;
                Ok(deferred)
            });
        if parsed.is_err() {
            task.motion.fail();
        }
        let _ = context.tail.set(Mutex::new(Some(parsed)));
        let _ = context.early.set(Mutex::new(None));
    }
}

const ORDER_KEY_FRAME_STRIDE: u64 = 1 << 32;
type ReconScratchSlot = Arc<CompletionCell<Mutex<Option<ScheduledReconScratch>>>>;

pub(crate) enum ScheduledReconScratch {
    Eight(inter::InterDecodeScratch<u8>),
    Ten(inter::InterDecodeScratch<u16>),
}

/// A scheduled frame of either sample depth.
///
/// One scheduler serves both depths, so a task that names a frame has to hold
/// either kind; the pipeline already splits this way for its scratch.
pub(crate) enum ScheduledFrameRef {
    Eight(Arc<ScheduledFrame<u8>>),
    Ten(Arc<ScheduledFrame<u16>>),
}

/// One frame's filter stripe, named at its sample depth.
pub(crate) enum ScheduledFilterJob {
    Eight(
        Arc<ScheduledFrame<u8>>,
        crate::filters::wienerns_lr::recon::OwnedFilterJob<u8>,
    ),
    Ten(
        Arc<ScheduledFrame<u16>>,
        crate::filters::wienerns_lr::recon::OwnedFilterJob<u16>,
    ),
}

/// The job shapes the pipeline schedules for every unit of every frame.
///
/// Entropy passes and reconstruction stages occupy typed scheduler records.
/// Entropy inputs and publication cells belong to bounded frame contexts.
pub(crate) enum FrameTask<'job> {
    ParseEight(EntropySlot<'job, u8>),
    ParseTen(EntropySlot<'job, u16>),
    PrepareEight(EntropySlot<'job, u8>),
    PrepareTen(EntropySlot<'job, u16>),
    AttachEight(EntropySlot<'job, u8>),
    AttachTen(EntropySlot<'job, u16>),
    Precompute {
        frame: ScheduledFrameRef,
        index: usize,
    },
    Commit {
        frame: ScheduledFrameRef,
        index: usize,
    },
    Frontier {
        frame: ScheduledFrameRef,
        row: usize,
    },
    Resolve {
        frame: ScheduledFrameRef,
        index: usize,
    },
    Filter(ScheduledFilterJob),
    Output(ScheduledFrameRef),
}

impl<'job> splot_parallel::Task<'job> for FrameTask<'job> {
    fn run(self, admit: &dyn splot_parallel::Admit<'job, Self>) {
        match self {
            Self::Output(frame) => match frame {
                ScheduledFrameRef::Eight(frame) => frame.run_output(),
                ScheduledFrameRef::Ten(frame) => frame.run_output(),
            },
            Self::ParseEight(context) => EntropyTask::run(&context),
            Self::ParseTen(context) => EntropyTask::run(&context),
            Self::PrepareEight(context) => ScheduledPrepare::run(&context, admit),
            Self::PrepareTen(context) => ScheduledPrepare::run(&context, admit),
            Self::AttachEight(context) => ScheduledAttach::run(&context, admit),
            Self::AttachTen(context) => ScheduledAttach::run(&context, admit),
            Self::Precompute { frame, index } => match frame {
                ScheduledFrameRef::Eight(frame) => frame.precompute(index, admit),
                ScheduledFrameRef::Ten(frame) => frame.precompute(index, admit),
            },
            Self::Commit { frame, index } => match frame {
                ScheduledFrameRef::Eight(frame) => frame.commit(index, admit),
                ScheduledFrameRef::Ten(frame) => frame.commit(index, admit),
            },
            Self::Frontier { frame, row } => match frame {
                ScheduledFrameRef::Eight(frame) => frame.frontier(row, admit),
                ScheduledFrameRef::Ten(frame) => frame.frontier(row, admit),
            },
            Self::Resolve { frame, index } => match frame {
                ScheduledFrameRef::Eight(frame) => frame.resolve(index, admit),
                ScheduledFrameRef::Ten(frame) => frame.resolve(index, admit),
            },
            Self::Filter(job) => match job {
                ScheduledFilterJob::Eight(frame, filter) => frame.run_filter_stripe(filter),
                ScheduledFilterJob::Ten(frame, filter) => frame.run_filter_stripe(filter),
            },
        }
    }
}

/// Wraps a job the task enum cannot name.
pub(crate) fn boxed_task<'job>(
    job: impl for<'a> FnOnce(&'a dyn splot_parallel::Admit<'job, FrameTask<'job>>) + Send + 'job,
) -> splot_parallel::Job<'job, FrameTask<'job>> {
    splot_parallel::Job::Boxed(Box::new(job))
}

pub(crate) trait ScheduledScratchSample: splot_recon::ReconSample {
    fn parse_task(task: EntropySlot<'_, Self>) -> FrameTask<'_>;
    fn prepare_task(task: EntropySlot<'_, Self>) -> FrameTask<'_>;
    fn attach_task(task: EntropySlot<'_, Self>) -> FrameTask<'_>;
    /// Names this depth's frame for a scheduled task.
    fn scheduled_frame_ref(frame: Arc<ScheduledFrame<Self>>) -> ScheduledFrameRef;

    /// Names this depth's frame and one of its filter stripes.
    fn scheduled_filter_job(
        frame: Arc<ScheduledFrame<Self>>,
        filter: crate::filters::wienerns_lr::recon::OwnedFilterJob<Self>,
    ) -> ScheduledFilterJob;

    fn take_scheduled_scratch(
        scratch: &mut Option<ScheduledReconScratch>,
    ) -> Option<inter::InterDecodeScratch<Self>>;

    fn wrap_scheduled_scratch(scratch: inter::InterDecodeScratch<Self>) -> ScheduledReconScratch;
}

macro_rules! impl_scheduled_scratch_sample {
    ($sample:ty, $variant:ident, $parse:ident, $prepare:ident, $attach:ident) => {
        impl ScheduledScratchSample for $sample {
            fn parse_task(task: EntropySlot<'_, Self>) -> FrameTask<'_> {
                FrameTask::$parse(task)
            }
            fn prepare_task(task: EntropySlot<'_, Self>) -> FrameTask<'_> {
                FrameTask::$prepare(task)
            }
            fn attach_task(task: EntropySlot<'_, Self>) -> FrameTask<'_> {
                FrameTask::$attach(task)
            }
            fn scheduled_frame_ref(frame: Arc<ScheduledFrame<Self>>) -> ScheduledFrameRef {
                ScheduledFrameRef::$variant(frame)
            }

            fn scheduled_filter_job(
                frame: Arc<ScheduledFrame<Self>>,
                filter: crate::filters::wienerns_lr::recon::OwnedFilterJob<Self>,
            ) -> ScheduledFilterJob {
                ScheduledFilterJob::$variant(frame, filter)
            }

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

impl_scheduled_scratch_sample!(u8, Eight, ParseEight, PrepareEight, AttachEight);
impl_scheduled_scratch_sample!(u16, Ten, ParseTen, PrepareTen, AttachTen);

type PendingTipProducts<T> = (
    PipelineFrameSlot,
    PendingFinish<T>,
    inter::FrameDecodeGeometry,
    inter::FrameProductWriters,
    inter::MotionFieldHandle,
);

/// Reserves the pending frame and product handles published by one TIP job.
pub(super) fn reserve_tip_output<T: super::inflight::SpareFramePlanes>(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    bit_depth: BitDepth,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    frames: &mut super::FrameStore,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<PendingTipProducts<T>> {
    let geometry = inter::FrameDecodeGeometry::new(core, sequence, bit_depth, false)?;
    let motion = frames.reserve_motion(geometry.motion_layout())?;
    let (slot, finish) =
        super::inflight::reserve_pending_slot(geometry.info(), erase, ring, frames, frame_index)?;
    let products = frames.reserve_products()?;
    Ok((slot, finish, geometry, products, motion))
}

/// Admits one reference-gated TIP output reconstruction without stopping the
/// frame driver at its pixel-reference barrier.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_tip_output<'job, 'scope, T, P>(
    reconstruct: P,
    frame_index: usize,
    dependencies: &[Condition<'_>],
    mut products: inter::FrameProductWriters,
    motion: inter::MotionFieldHandle,
    finish: PendingFinish<T>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) where
    T: ScheduledScratchSample + Send + 'static,
    P: FnOnce(
            &mut inter::InterDecodeScratch<T>,
            &mut inter::FrameProductWriters,
        ) -> Result<inter::InterDecodeOutput<T>>
        + Send
        + 'job,
    'job: 'scope,
{
    let (scratch_source, scratch_done) = match lane.reserve_recon() {
        Ok(reservation) => reservation,
        Err(error) => {
            drop(products);
            motion.fail();
            finish.fail(error);
            return;
        }
    };
    let mut conditions = dependencies
        .iter()
        .copied()
        .chain(scratch_source.as_deref().map(Condition::completion));
    let scratch_for_job = scratch_source.clone();
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit_iter(
        scope,
        order_key,
        &mut conditions,
        boxed_task(move |_| {
            let mut scratch = scratch_for_job
                .as_deref()
                .and_then(CompletionCell::get)
                .and_then(|scratch| T::take_scheduled_scratch(&mut scratch.lock()))
                .unwrap_or_default();
            match reconstruct(&mut scratch, &mut products) {
                Ok((frame, _, cdfs, ccso, field, segments)) => {
                    products.settle(cdfs, ccso, segments);
                    motion.publish(field);
                    finish.complete_frame(frame);
                }
                Err(error) => {
                    drop(products);
                    motion.fail();
                    finish.fail(error);
                }
            }
            let _ = scratch_done.set(Mutex::new(Some(T::wrap_scheduled_scratch(scratch))));
        }),
    );
}

type LaneReservation<T> = (Option<Arc<CompletionCell<T>>>, Arc<CompletionCell<T>>);

/// The frame-context admission bound for scheduled reconstruction.
pub(super) struct ReconAdmissionLane {
    depth: usize,
    recon: VecDeque<ReconScratchSlot>,
    filters: VecDeque<Arc<CompletionCell<()>>>,
    recon_cells: Vec<ReconScratchSlot>,
    filter_cells: Vec<Arc<CompletionCell<()>>>,
}

impl ReconAdmissionLane {
    pub(super) fn new(depth: usize) -> Self {
        let depth = depth.max(1);
        // Two identities per context (2D), in-flight job (D), and worker (W),
        // plus D queued identities and the reservation in construction.

        let cells = depth
            .saturating_mul(7)
            .saturating_add(
                splot_parallel::current_pool_width()
                    .max(1)
                    .saturating_mul(2),
            )
            .saturating_add(1);
        Self {
            depth,
            recon: VecDeque::with_capacity(depth),
            filters: VecDeque::with_capacity(depth),
            recon_cells: (0..cells)
                .map(|_| Arc::new(CompletionCell::new()))
                .collect(),
            filter_cells: (0..cells)
                .map(|_| Arc::new(CompletionCell::new()))
                .collect(),
        }
    }

    fn reserve<T>(
        depth: usize,
        lane: &mut VecDeque<Arc<CompletionCell<T>>>,
        cells: &mut [Arc<CompletionCell<T>>],
    ) -> Result<LaneReservation<T>> {
        let done = cells
            .iter_mut()
            .find_map(|cell| {
                Arc::get_mut(cell)?.reset();
                Some(Arc::clone(cell))
            })
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let gate = if lane.len() == depth {
            lane.pop_front()
        } else {
            None
        };
        lane.push_back(Arc::clone(&done));
        Ok((gate, done))
    }

    fn reserve_recon(&mut self) -> Result<(Option<ReconScratchSlot>, ReconScratchSlot)> {
        Self::reserve(self.depth, &mut self.recon, &mut self.recon_cells)
    }

    fn reserve_filter(&mut self) -> Result<LaneReservation<()>> {
        Self::reserve(self.depth, &mut self.filters, &mut self.filter_cells)
    }
}

#[derive(Default)]
pub(crate) struct ScheduledFrame<T: splot_recon::ReconSample> {
    active: Option<ScheduledFrameState<T>>,
    filter_shell: Mutex<Option<crate::filters::wienerns_lr::recon::OwnedFilterShell<T>>>,
    prepared: Vec<CompletionCell<()>>,
    frontier_done: Vec<CompletionCell<()>>,
    filtered: Vec<CompletionCell<()>>,
    filter_error: Mutex<Option<DecodeError>>,
    filters_ready: CompletionCell<()>,
    failed: AtomicBool,
    order_base: u64,
}

struct ScheduledFrameState<T: splot_recon::ReconSample> {
    reconstruction: inter::ScheduledTileRecon<T>,
    finish: Mutex<Option<PendingFinish<T>>>,
    output: Mutex<Option<ScheduledOutput<T>>>,
    motion: inter::MotionFieldHandle,
    scratch_done: ReconScratchSlot,
    filter_gate: Option<Arc<CompletionCell<()>>>,
    filter_done: Arc<CompletionCell<()>>,
}

struct ScheduledOutput<T: splot_recon::ReconSample> {
    finish: PendingFinish<T>,
    filter: crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>,
}

impl<T: splot_recon::ReconSample> ScheduledFrame<T> {
    fn reset(&mut self) {
        self.active.take();
        for cell in self
            .prepared
            .iter_mut()
            .chain(&mut self.frontier_done)
            .chain(&mut self.filtered)
        {
            cell.reset();
        }
        self.filters_ready.reset();
        self.filter_error.get_mut().take();
        *self.failed.get_mut() = false;
    }
}

impl<'job, T: ScheduledScratchSample + Send + 'static> ScheduledFrame<T> {
    fn submit_batches(
        self: &Arc<Self>,
        batches: core::ops::Range<usize>,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let starts_commit = batches.start == 0 && !batches.is_empty();
        for index in batches {
            admit.submit_iter(
                self.batch_key(index, 1),
                &mut active.reconstruction.conditions(index),
                splot_parallel::Job::Inline(FrameTask::Precompute {
                    frame: T::scheduled_frame_ref(Arc::clone(self)),
                    index,
                }),
            );
        }
        if starts_commit {
            let commit = Arc::clone(self);
            admit.submit(
                self.batch_key(0, 2),
                &[Condition::completion(&self.prepared[0])],
                splot_parallel::Job::Inline(FrameTask::Commit {
                    frame: T::scheduled_frame_ref(commit),
                    index: 0,
                }),
            );
        }
    }

    fn continue_commit(
        self: &Arc<Self>,
        index: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let commit = Arc::clone(self);
        let job = splot_parallel::Job::Inline(FrameTask::Commit {
            frame: T::scheduled_frame_ref(commit),
            index,
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
    /// The first link waits for entropy's filter records; later links wait for
    /// their predecessor. The commit spine has already sealed each link's rows.
    fn submit_frontier(
        self: &Arc<Self>,
        batch: usize,
        row: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let ready = row
            .checked_sub(1)
            .and_then(|previous| self.frontier_done.get(previous))
            .unwrap_or(&self.filters_ready);
        let frame = Arc::clone(self);
        admit.submit(
            self.batch_key(batch, 3),
            &[Condition::completion(ready)],
            splot_parallel::Job::Inline(FrameTask::Frontier {
                frame: T::scheduled_frame_ref(frame),
                row,
            }),
        );
    }

    fn frontier(
        self: &Arc<Self>,
        row: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if !self.failed.load(Ordering::Acquire) {
            match active.reconstruction.frontier(row) {
                Ok(progress) => self.publish_filters(progress, admit),
                Err(error) => self.fail(error, admit),
            }
        }
        if let Some(done) = self.frontier_done.get(row) {
            let _ = done.set(());
        }
    }

    fn submit_resolve(
        self: &Arc<Self>,
        index: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let resolve = Arc::clone(self);
        let index_key = u64::try_from(index).unwrap_or(u64::MAX / 2);
        admit.submit_iter(
            self.order_base.saturating_add(index_key),
            &mut active.reconstruction.resolve_conditions(index),
            splot_parallel::Job::Inline(FrameTask::Resolve {
                frame: T::scheduled_frame_ref(resolve),
                index,
            }),
        );
    }

    /// Resolves one § 7.12 band, or resumes on the § 8.2 watermark when the
    /// pass has not yet published the units the band still owes.
    fn resolve(
        self: &Arc<Self>,
        index: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        let (batches, awaiting) = match active.reconstruction.resolve(index) {
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
                    active.reconstruction.parse_watermark(),
                    units,
                )],
                splot_parallel::Job::Inline(FrameTask::Resolve {
                    frame: T::scheduled_frame_ref(resume),
                    index,
                }),
            );
            return;
        }
        let next = index.saturating_add(1);
        if next < active.reconstruction.resolve_len() {
            self.submit_resolve(next, admit);
        }
    }

    fn precompute(
        self: &Arc<Self>,
        index: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if !self.failed.load(Ordering::Acquire)
            && let Err(error) = active.reconstruction.precompute(index)
        {
            self.fail(error, admit);
        }
        if let Some(prepared) = self.prepared.get(index) {
            let _ = prepared.set(());
        }
    }

    fn commit(
        self: &Arc<Self>,
        index: usize,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        match active.reconstruction.commit(index) {
            Ok(progress) => {
                if progress.recon_complete {
                    let scratch = match active.reconstruction.take_scheduled_scratch() {
                        Ok(scratch) => scratch,
                        Err(error) => {
                            self.fail(error, admit);
                            return;
                        }
                    };
                    let _ = active
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
        if !self.failed.load(Ordering::Acquire) && next < active.reconstruction.len() {
            self.continue_commit(next, admit);
        }
    }

    /// Schedules the filter stripes one frontier link released, and the
    /// frame's finish once the final link has released them all.
    fn publish_filters(
        self: &Arc<Self>,
        mut progress: inter::ScheduledFrameProgress<T>,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        for filter in progress.filters.drain(..) {
            let stripe = filter.stripe();
            if stripe >= active.reconstruction.filter_count() {
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
            admit.spawn_ready(splot_parallel::Job::Inline(FrameTask::Filter(
                T::scheduled_filter_job(Arc::clone(self), filter),
            )));
        }
        active
            .reconstruction
            .recycle_filter_jobs(core::mem::take(&mut progress.filters));
        let Some(filter) = progress.output else {
            return;
        };
        let finish = active.finish.lock().take();
        if let Some(finish) = finish {
            let mut conditions = active
                .filter_gate
                .iter()
                .map(|gate| Condition::completion(gate.as_ref()))
                .chain(
                    self.filtered[..active.reconstruction.filter_count()]
                        .iter()
                        .map(Condition::completion),
                );
            *active.output.lock() = Some(ScheduledOutput { finish, filter });
            admit.submit_iter(
                self.order_base + u64::from(u32::MAX),
                &mut conditions,
                splot_parallel::Job::Inline(FrameTask::Output(T::scheduled_frame_ref(Arc::clone(
                    self,
                )))),
            );
        }
    }

    fn run_output(&self) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(ScheduledOutput { finish, filter }) = active.output.lock().take() else {
            return;
        };
        if let Some(error) = self.filter_error.lock().take() {
            finish.fail(error);
        } else {
            *self.filter_shell.lock() = Some(finish.run_owned_finish(filter));
        }
        let _ = active.filter_done.set(());
    }

    /// Runs one filter stripe and publishes it, recording the frame's first
    /// filter error.
    fn run_filter_stripe(&self, filter: crate::filters::wienerns_lr::recon::OwnedFilterJob<T>) {
        let stripe = filter.stripe();
        if let Err(error) = filter.run() {
            let mut owed = self.filter_error.lock();
            if owed.is_none() {
                *owed = Some(error);
            }
        }
        if let Some(filtered) = self.filtered.get(stripe) {
            let _ = filtered.set(());
        }
    }

    fn fail(
        &self,
        error: DecodeError,
        admit: &dyn splot_parallel::Admit<'job, crate::pipeline::frame_pipeline::FrameTask<'job>>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        active.reconstruction.fail_temporal();
        active.motion.fail();
        let _ = active.scratch_done.set(Mutex::new(None));
        let _ = active.filter_done.set(());
        for completion in self
            .prepared
            .iter()
            .chain(&self.frontier_done)
            .chain(&self.filtered)
        {
            let _ = completion.set(());
        }
        admit.admit_ready();
        if let Some(finish) = active.finish.lock().take() {
            finish.fail(error);
        }
    }
}

fn schedule_typed<'job, 'scope, T: ScheduledScratchSample + Send + 'static>(
    early: inter::InterWalkEarly<T>,
    context: EntropySlot<'job, T>,
    finish: PendingFinish<T>,
    frame_index: usize,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    let tail = Arc::clone(&context.tail);
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    let motion = early.motion.clone();
    let dependencies = early.motion_dependencies();
    let (scratch_source, scratch_done) = match lane.reserve_recon() {
        Ok(reservation) => reservation,
        Err(error) => {
            motion.fail();
            finish.fail(error);
            return;
        }
    };
    let (filter_gate, filter_done) = match lane.reserve_filter() {
        Ok(reservation) => reservation,
        Err(error) => {
            let _ = scratch_done.set(Mutex::new(None));
            motion.fail();
            finish.fail(error);
            return;
        }
    };
    let mut conditions = dependencies
        .iter()
        .flatten()
        .map(inter::MotionFieldHandle::metadata_condition)
        .chain(scratch_source.as_deref().map(Condition::completion));
    let scheduled_scratch_source = scratch_source.clone();
    let scratch_done_for_job = Arc::clone(&scratch_done);
    let filter_done_for_job = Arc::clone(&filter_done);
    *context.prepare.lock() = Some(ScheduledPrepare {
        early,
        tail,
        finish,
        motion,
        scheduled_scratch_source,
        scratch_done_for_job,
        filter_gate,
        filter_done_for_job,
        order_base,
    });
    scheduler.submit_iter(
        scope,
        order_base,
        &mut conditions,
        splot_parallel::Job::Inline(T::prepare_task(context)),
    );
}

struct ScheduledPrepare<T: splot_recon::ReconSample> {
    early: inter::InterWalkEarly<T>,
    tail: EntropyResult<T>,
    finish: PendingFinish<T>,
    motion: inter::MotionFieldHandle,
    scheduled_scratch_source: Option<ReconScratchSlot>,
    scratch_done_for_job: ReconScratchSlot,
    filter_gate: Option<Arc<CompletionCell<()>>>,
    filter_done_for_job: Arc<CompletionCell<()>>,
    order_base: u64,
}

impl<T: ScheduledScratchSample + Send + 'static> ScheduledPrepare<T> {
    fn run<'job>(
        context: &EntropySlot<'job, T>,
        admit: &dyn splot_parallel::Admit<'job, FrameTask<'job>>,
    ) {
        let Some(Self {
            early,
            tail,
            finish,
            motion,
            scheduled_scratch_source,
            scratch_done_for_job,
            filter_gate,
            filter_done_for_job,
            order_base,
        }) = context.prepare.lock().take()
        else {
            return;
        };

        let decode_scratch = scheduled_scratch_source
            .as_deref()
            .and_then(CompletionCell::get)
            .and_then(|scratch| T::take_scheduled_scratch(&mut scratch.lock()))
            .unwrap_or_default();
        let settle_prepare_failure = |finish: PendingFinish<T>, error| {
            let _ = scratch_done_for_job.set(Mutex::new(None));
            motion.fail();
            let _ = filter_done_for_job.set(());
            finish.fail(error);
        };
        let progress = finish.progress_handle();
        let mut decode_scratch = decode_scratch;
        if let Some(buffers) = progress.buffers() {
            decode_scratch.set_decode_buffers(buffers);
        }
        let (scheduled, pending_filters) = match early.prepare_scheduled(
            decode_scratch,
            &mut context.workspace.lock(),
            &mut context.temporal.lock(),
            Arc::clone(&context.workers),
            progress,
        ) {
            Ok((scheduled, pending)) => {
                admit.admit_ready();
                (scheduled, pending)
            }
            Err(error) => {
                settle_prepare_failure(finish, error);
                return;
            }
        };
        let frame = {
            let mut slot = context.frame.lock();
            let frame = slot.get_or_insert_with(|| Arc::new(ScheduledFrame::default()));
            let Some(reused) = Arc::get_mut(frame) else {
                settle_prepare_failure(finish, frame_task_scope());
                return;
            };
            reused.prepared.resize_with(
                reused.prepared.len().max(scheduled.len()),
                CompletionCell::new,
            );
            reused.frontier_done.resize_with(
                reused.frontier_done.len().max(scheduled.frontier_len()),
                CompletionCell::new,
            );
            reused.filtered.resize_with(
                reused.filtered.len().max(scheduled.filter_count()),
                CompletionCell::new,
            );
            reused.order_base = order_base;
            reused.active = Some(ScheduledFrameState {
                reconstruction: scheduled,
                finish: Mutex::new(Some(finish)),
                output: Mutex::new(None),
                motion,
                scratch_done: scratch_done_for_job,
                filter_gate,
                filter_done: filter_done_for_job,
            });
            Arc::clone(frame)
        };
        *context.attach.lock() = Some(ScheduledAttach {
            attach_frame: Arc::clone(&frame),
            attach_tail: Arc::clone(&tail),
            pending_filters,
        });
        admit.submit(
            order_base.saturating_add(1 << 19),
            &[Condition::completion(tail.as_ref())],
            splot_parallel::Job::Inline(T::attach_task(Arc::clone(context))),
        );
        if frame
            .active
            .as_ref()
            .is_none_or(|active| active.reconstruction.resolve_len() == 0)
        {
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
    }
}

struct ScheduledAttach<T: splot_recon::ReconSample> {
    attach_frame: Arc<ScheduledFrame<T>>,
    attach_tail: EntropyResult<T>,
    pending_filters: inter::PendingFilterAttach<T>,
}

impl<T: ScheduledScratchSample + Send + 'static> ScheduledAttach<T> {
    fn run<'job>(
        context: &EntropySlot<'job, T>,
        admit: &dyn splot_parallel::Admit<'job, FrameTask<'job>>,
    ) {
        let Some(Self {
            attach_frame,
            attach_tail,
            pending_filters,
        }) = context.attach.lock().take()
        else {
            return;
        };

        let parsed = attach_tail.get().and_then(|slot| slot.lock().take());
        let outcome = match parsed {
            Some(Ok(deferred)) => match attach_frame.active.as_ref() {
                Some(active) => {
                    let filter_shell = attach_frame
                        .filter_shell
                        .lock()
                        .take()
                        .unwrap_or_else(|| Arc::new(None));
                    deferred.attach_filters(pending_filters, &active.reconstruction, filter_shell)
                }
                None => Err(frame_task_scope()),
            },
            Some(Err(error)) => Err(error),
            None => Err(frame_task_scope()),
        };
        match outcome {
            Ok(()) => {
                let _ = attach_frame.filters_ready.set(());
            }
            Err(error) => {
                attach_frame.fail(error, admit);
                let _ = attach_frame.filters_ready.set(());
            }
        }
    }
}

pub(super) fn schedule_finish<'job, 'scope, T: splot_recon::ReconSample + Send + 'static>(
    finish: PendingFinish<T>,
    walked: super::frame_engine::finish::WalkedFrame<T>,
    frame_index: usize,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    let (gate, done) = match lane.reserve_filter() {
        Ok(reservation) => reservation,
        Err(error) => {
            finish.fail(error);
            return;
        }
    };
    let conditions = gate.as_deref().map(Condition::completion);
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_base + u64::from(u32::MAX),
        conditions.as_slice(),
        boxed_task(move |admit| {
            finish.run_finish(walked, Some(admit));
            let _ = done.set(());
        }),
    );
}

fn promote_front<'scope, 'job>(
    entropy: &mut PendingEntropyQueue<'job>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    let Some(pending) = entropy.pop_front() else {
        return;
    };
    match pending {
        PendingEntropy::Eight {
            frame_index,
            result:
                EntropyHandles {
                    early,
                    tail,
                    context,
                },
            finish,
        } => {
            let settled = early.wait_with_assist(|| scheduler.assist_ready(scope));
            let early = settled.lock().take();
            if let Some(early) = early {
                schedule_typed(early, context, finish, frame_index, scheduler, scope, lane);
            } else {
                let error = tail
                    .get()
                    .and_then(|slot| slot.lock().take().and_then(Result::err))
                    .unwrap_or_else(frame_task_scope);
                finish.fail(error);
            }
        }
        PendingEntropy::Ten {
            frame_index,
            result:
                EntropyHandles {
                    early,
                    tail,
                    context,
                },
            finish,
        } => {
            let settled = early.wait_with_assist(|| scheduler.assist_ready(scope));
            let early = settled.lock().take();
            if let Some(early) = early {
                schedule_typed(early, context, finish, frame_index, scheduler, scope, lane);
            } else {
                let error = tail
                    .get()
                    .and_then(|slot| slot.lock().take().and_then(Result::err))
                    .unwrap_or_else(frame_task_scope);
                finish.fail(error);
            }
        }
    }
}

fn drain_ready_entropy<'scope, 'job>(
    entropy: &mut PendingEntropyQueue<'job>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    loop {
        while entropy.front().is_some_and(PendingEntropy::is_settled) {
            promote_front(entropy, scope, scheduler, lane);
        }
        if entropy.is_empty() || !splot_parallel::assist_pool_once() {
            return;
        }
    }
}

/// Opens one bounded entropy-context slot before the caller reserves frame storage.
pub(super) fn prepare_entropy_submission<'scope, 'job>(
    entropy: &mut PendingEntropyQueue<'job>,
    limit: usize,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    let limit = limit.max(1);
    drain_ready_entropy(entropy, scope, scheduler, lane);
    while entropy.len() >= limit {
        promote_front(entropy, scope, scheduler, lane);
    }
}

/// Promotes every pending entropy context before a non-inter frame or output barrier.
pub(super) fn drain_entropy_before_barrier<'scope, 'job>(
    entropy: &mut PendingEntropyQueue<'job>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
) where
    'job: 'scope,
{
    if entropy.is_empty() {
        return;
    }
    while !entropy.is_empty() {
        promote_front(entropy, scope, scheduler, lane);
    }
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
    fn entropy_contexts_reuse_the_same_bounded_slots() {
        let mut contexts = EntropyContexts::<u16>::new(12);
        let addresses: Vec<_> = (0..12)
            .map(|_| {
                let slot = contexts.claim();
                let frame = Arc::new(ScheduledFrame::<u16> {
                    prepared: vec![CompletionCell::new()],
                    frontier_done: vec![CompletionCell::new()],
                    filtered: vec![CompletionCell::new()],
                    ..Default::default()
                });
                let addresses = (
                    Arc::as_ptr(&slot),
                    Arc::as_ptr(&frame),
                    std::ptr::from_ref(&frame.prepared[0]),
                );
                *slot.frame.lock() = Some(frame);
                addresses
            })
            .collect();
        for index in 0..1200 {
            let slot = contexts.claim();
            assert_eq!(Arc::as_ptr(&slot), addresses[index % 12].0);
            let frame = slot.frame.lock();
            let frame = frame.as_ref().unwrap();
            assert_eq!(Arc::as_ptr(frame), addresses[index % 12].1);
            assert_eq!(
                std::ptr::from_ref(&frame.prepared[0]),
                addresses[index % 12].2
            );
            for cell in [
                &frame.prepared[0],
                &frame.frontier_done[0],
                &frame.filtered[0],
                &frame.filters_ready,
            ] {
                assert!(!cell.is_set());
                cell.set(()).unwrap();
            }
            assert!(slot.early.get().is_none());
            assert!(slot.tail.get().is_none());
            assert!(slot.early.set(Mutex::new(None)).is_ok());
            assert!(slot.tail.set(Mutex::new(None)).is_ok());
        }
        assert_eq!(contexts.slots.len(), 12);
    }

    #[test]
    fn frame_context_cannot_reset_while_a_task_holds_it() {
        let mut contexts = EntropyContexts::<u16>::new(1);
        let slot = contexts.claim();
        let frame = Arc::new(ScheduledFrame::default());
        *slot.frame.lock() = Some(Arc::clone(&frame));
        drop(slot);
        let context = Arc::get_mut(&mut contexts.slots[0]).unwrap();
        assert!(!context.reset());
        drop(frame);
        let temporal = Arc::clone(context.temporal.get_mut());
        assert!(!context.reset());
        drop(temporal);
        assert!(context.reset());
    }

    #[test]
    fn recon_lane_returns_the_same_typed_context_at_bounded_depth() {
        let mut lane = ReconAdmissionLane::new(1);
        let (prior, first) = lane.reserve_recon().expect("resident cell");
        assert!(prior.is_none());
        assert!(
            first
                .set(Mutex::new(Some(ScheduledReconScratch::Ten(
                    inter::InterDecodeScratch::default(),
                ))))
                .is_ok()
        );

        let (prior, _) = lane.reserve_recon().expect("resident cell");
        assert_eq!(lane.recon.len(), 1);
        let prior = prior.expect("prior context");
        let stored = prior.get().expect("settled prior context");
        let mut stored = stored.lock();
        assert!(u8::take_scheduled_scratch(&mut stored).is_none());
        assert!(u16::take_scheduled_scratch(&mut stored).is_some());
    }

    #[test]
    fn failed_recon_context_still_settles_the_lane() {
        let mut lane = ReconAdmissionLane::new(1);
        let (_, failed) = lane.reserve_recon().expect("resident cell");
        assert!(failed.set(Mutex::new(None)).is_ok());

        let (prior, _) = lane.reserve_recon().expect("resident cell");
        assert!(
            prior
                .expect("failed prior context")
                .get()
                .expect("settled failure")
                .lock()
                .is_none()
        );
    }
    #[test]
    fn admission_cells_reuse_only_after_readers_retire() {
        let mut lane = ReconAdmissionLane::new(1);
        let addresses = lane
            .filter_cells
            .iter()
            .map(Arc::as_ptr)
            .collect::<Vec<_>>();
        let mut readers = VecDeque::new();
        for _ in 0..1200 {
            let (prior, done) = lane.reserve_filter().expect("resident cell");
            assert!(done.get().is_none());
            assert!(addresses.contains(&Arc::as_ptr(&done)));
            if let Some(prior) = prior {
                assert!(prior.get().is_some());
                readers.push_back(prior);
            }
            assert!(done.set(()).is_ok());
            while readers.len() > 2 {
                readers.pop_front();
            }
            assert!(readers.iter().all(|reader| reader.get().is_some()));
        }
        assert_eq!(
            lane.filter_cells
                .iter()
                .map(Arc::as_ptr)
                .collect::<Vec<_>>(),
            addresses
        );
        drop(readers);
        let retained = lane.filter_cells.clone();
        let previous = Arc::as_ptr(&lane.filters[0]);
        assert!(lane.reserve_filter().is_err());
        assert_eq!(Arc::as_ptr(&lane.filters[0]), previous);
        drop(retained);
        assert!(lane.reserve_filter().is_ok());
    }
}
