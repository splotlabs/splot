// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Dependency-ordered task admission.
//!
//! Pool jobs must not wait for other pool jobs: work stealing can otherwise
//! place a consumer above its producer and deadlock the pool. A submitted job
//! is stored until all of its [`Condition`]s hold. A publication satisfying the
//! last unmet condition queues the job as ready; a scheduler drain spawns it.
//! Each scheduler job drains newly ready work after its body returns, while an
//! external publisher must call [`AdmissionScheduler::admit_ready`] after
//! publishing. The outstanding count starts at `conditions + 1`; the final unit
//! is cleared after registration, so publication cannot admit a partially
//! registered job. Reused slots carry generations to reject stale notices.
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::atomic::{AtomicBool, Ordering};
//! use splot_parallel::{
//!     AdmissionScheduler, CompletionCell, Condition, Job, NoTask, ThreadCount, WatermarkCell,
//!     WorkerPool, ready_task_scope,
//! };
//!
//! let pool = WorkerPool::new(ThreadCount::from(2usize))?;
//! let parsed = CompletionCell::new();
//! let rows = WatermarkCell::new();
//! let ran = AtomicBool::new(false);
//! let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
//! pool.install(|| {
//!     ready_task_scope(|scope| {
//!         scheduler.submit(
//!             scope,
//!             0,
//!             &[Condition::completion(&parsed), Condition::watermark(&rows, 2)],
//!             Job::Boxed(Box::new(|_| ran.store(true, Ordering::Release))),
//!         );
//!         assert!(parsed.set(()).is_ok());
//!         rows.publish(2);
//!         scheduler.admit_ready(scope);
//!     })
//! })?;
//! scheduler.finish()?;
//! assert!(ran.load(Ordering::Acquire));
//! # Ok(())
//! # }
//! ```
use parking_lot::Mutex;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::completion::CompletionCell;
use crate::error::ParallelError;
use crate::pool::{TaskScope, notify_installed_pool_progress};
use crate::watermark::WatermarkCell;

trait CompletionSource {
    fn register(&self, waiter: Arc<Waiter>) -> bool;
    fn is_ready(&self) -> bool;
}

impl<V> CompletionSource for CompletionCell<V> {
    fn register(&self, waiter: Arc<Waiter>) -> bool {
        self.register_waiter(waiter)
    }

    fn is_ready(&self) -> bool {
        self.is_set()
    }
}

#[derive(Clone, Copy)]
enum ConditionSource<'a> {
    Watermark(&'a WatermarkCell, usize),
    Completion(&'a dyn CompletionSource),
}

/// One dependency a submitted job needs before it may run.
#[derive(Clone, Copy)]
pub struct Condition<'a>(ConditionSource<'a>);

impl<'a> Condition<'a> {
    /// Requires `cell` to reach `threshold`.
    #[must_use]
    pub const fn watermark(cell: &'a WatermarkCell, threshold: usize) -> Self {
        Self(ConditionSource::Watermark(cell, threshold))
    }

    /// Requires `cell` to be completed.
    #[must_use]
    pub fn completion<V>(cell: &'a CompletionCell<V>) -> Self {
        Self(ConditionSource::Completion(cell))
    }

    fn register(self, waiter: Arc<Waiter>) -> bool {
        match self.0 {
            ConditionSource::Watermark(cell, threshold) => cell.register(threshold, waiter),
            ConditionSource::Completion(cell) => cell.register(waiter),
        }
    }

    /// Whether this condition already holds.
    ///
    /// Both sources only ever move toward satisfied, so a caller that finds
    /// every condition met needs no waiter to watch them.
    fn is_satisfied(&self) -> bool {
        match self.0 {
            ConditionSource::Watermark(cell, threshold) => cell.current() >= threshold,
            ConditionSource::Completion(cell) => cell.is_ready(),
        }
    }
}

/// Work the scheduler stores in its slot rather than on the heap.
///
/// dav2d keeps a preallocated array of task records and dispatches on a kind
/// tag; a boxed closure per task is the same thing with an allocation in front
/// of it. A caller with a fixed set of job shapes names them here and pays
/// nothing per task; anything else still boxes.
pub trait Task<'job>: Send + Sized + 'job {
    /// Runs this task.
    fn run(self, admit: &dyn Admit<'job, Self>);
}

/// A job shape for callers that have none worth naming.
pub enum NoTask {}

impl<'job> Task<'job> for NoTask {
    fn run(self, _admit: &dyn Admit<'job, Self>) {
        match self {}
    }
}

/// A job the scheduler had to put on the heap, because its shape is not named.
type BoxedJob<'job, F> = Box<dyn for<'a> FnOnce(&'a dyn Admit<'job, F>) + Send + 'job>;

/// A unit of deferred work.
pub enum Job<'job, F: Task<'job> = NoTask> {
    /// A named task, stored in the scheduler's slot.
    Inline(F),
    /// Anything else, on the heap.
    Boxed(BoxedJob<'job, F>),
}

impl<'job, F: Task<'job>> Job<'job, F> {
    fn run(self, admit: &dyn Admit<'job, F>) {
        match self {
            Self::Inline(task) => task.run(admit),
            Self::Boxed(job) => job(admit),
        }
    }
}

/// Operations available to a running admitted job.
pub trait Admit<'job, F: Task<'job> = NoTask>: Sync {
    /// Spawns all jobs that are admissible now.
    fn admit_ready(&self) -> usize;
    /// Submits a job under the same scheduler.
    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job, F>);
    /// Spawns a job already known to be ready, without a scheduler slot.
    fn spawn_ready(&self, job: Job<'job, F>);
    /// Submits proven-ready jobs as one ordered scheduler entry.
    fn submit_ready_batch(&self, order_key: u64, jobs: Vec<Job<'job, F>>);
    /// Records one serial successor for this worker.
    fn continue_ready(&self, order_key: u64, job: Job<'job, F>);
}

const CONTINUATION_BUDGET: usize = 8;

/// Continuation slots the scheduler builds up front, one per worker.
///
/// A fixed count rather than the pool's width: the scheduler is usually built
/// off a worker thread, where the width reads as one, and a single shared slot
/// would put every worker's continuation through the same lock.
const MAX_WORKER_CONTINUATIONS: usize = 64;

struct OrderedJob<'job, F: Task<'job>> {
    order_key: u64,
    job: Job<'job, F>,
}

/// The continuation a running job may leave for its own worker.
///
/// The flag is what keeps the lock cold. A slot is built per `run_job` frame,
/// and a platform lock is allocated the first time each one is locked, so a
/// job that leaves no continuation -- most of them -- paid an allocation for a
/// lock guarding a `None`.
struct ContinuationSlot<'job, F: Task<'job>> {
    pending: AtomicBool,
    job: Mutex<Option<OrderedJob<'job, F>>>,
}

impl<'job, F: Task<'job>> ContinuationSlot<'job, F> {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            job: Mutex::new(None),
        }
    }

    fn put(&self, next: OrderedJob<'job, F>) -> Result<(), OrderedJob<'job, F>> {
        let mut pending = self.job.lock();
        if pending.is_some() {
            return Err(next);
        }
        *pending = Some(next);
        self.pending.store(true, Ordering::Release);
        Ok(())
    }

    fn take(&self) -> Option<OrderedJob<'job, F>> {
        if !self.pending.load(Ordering::Acquire) {
            return None;
        }
        let taken = self.job.lock().take();
        self.pending.store(false, Ordering::Release);
        taken
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReadyEntry {
    order_key: u64,
    submission_order: u64,
    index: usize,
    generation: u64,
}

#[derive(Debug, Default)]
struct ReadyQueue(Mutex<BinaryHeap<Reverse<ReadyEntry>>>);

impl ReadyQueue {
    fn push(&self, entry: ReadyEntry) {
        self.0.lock().push(Reverse(entry));
    }

    fn pop(&self) -> Option<ReadyEntry> {
        self.0.lock().pop().map(|Reverse(entry)| entry)
    }
}

#[derive(Debug)]
pub(crate) struct Waiter {
    pending: AtomicUsize,
    entry: ReadyEntry,
    ready: Arc<ReadyQueue>,
}

impl Waiter {
    pub(crate) fn satisfy(&self) -> bool {
        let mut pending = self.pending.load(Ordering::Acquire);
        while pending != 0 {
            match self.pending.compare_exchange_weak(
                pending,
                pending - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) if pending == 1 => {
                    self.ready.push(self.entry);
                    return true;
                }
                Ok(_) => return false,
                Err(observed) => pending = observed,
            }
        }
        false
    }
}

struct Slot<'job, F: Task<'job>> {
    generation: u64,
    order_key: u64,
    job: Option<Job<'job, F>>,
    waiter: Option<Arc<Waiter>>,
}

/// Waiters kept for the next submission.
///
/// A waiter outlives its job only until the conditions holding it let go, so a
/// slot the scheduler has already drained usually holds the sole reference and
/// the allocation can serve the next job instead of a fresh one.
const MAX_SPARE_WAITERS: usize = 64;

struct Slots<'job, F: Task<'job>> {
    entries: Vec<Slot<'job, F>>,
    free: Vec<usize>,
    spare_waiters: Vec<Arc<Waiter>>,
    next_submission_order: u64,
}

impl<'job, F: Task<'job>> Default for Slots<'job, F> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            free: Vec::new(),
            spare_waiters: Vec::new(),
            next_submission_order: 0,
        }
    }
}

impl<'job, F: Task<'job>> Slots<'job, F> {
    fn store(
        &mut self,
        pending: usize,
        order_key: u64,
        job: Job<'job, F>,
        ready: &Arc<ReadyQueue>,
    ) -> Arc<Waiter> {
        let (index, generation) = self.take_slot();
        let entry = ReadyEntry {
            order_key,
            submission_order: self.next_submission_order,
            index,
            generation,
        };
        self.next_submission_order = self.next_submission_order.wrapping_add(1);
        let waiter = self.reuse_waiter(pending, entry).unwrap_or_else(|| {
            Arc::new(Waiter {
                pending: AtomicUsize::new(pending),
                entry,
                ready: Arc::clone(ready),
            })
        });
        let slot = &mut self.entries[index];
        slot.order_key = order_key;
        slot.job = Some(job);
        slot.waiter = Some(Arc::clone(&waiter));
        waiter
    }

    /// Stores a job whose conditions already hold, queueing it with no waiter.
    fn store_ready(&mut self, order_key: u64, job: Job<'job, F>, ready: &ReadyQueue) {
        let (index, generation) = self.take_slot();
        let entry = ReadyEntry {
            order_key,
            submission_order: self.next_submission_order,
            index,
            generation,
        };
        self.next_submission_order = self.next_submission_order.wrapping_add(1);
        let slot = &mut self.entries[index];
        slot.order_key = order_key;
        slot.job = Some(job);
        slot.waiter = None;
        ready.push(entry);
    }

    /// Reuses a spare waiter, if one is left holding the sole reference.
    fn reuse_waiter(&mut self, pending: usize, entry: ReadyEntry) -> Option<Arc<Waiter>> {
        while let Some(mut spare) = self.spare_waiters.pop() {
            let Some(waiter) = Arc::get_mut(&mut spare) else {
                continue;
            };
            waiter.pending = AtomicUsize::new(pending);
            waiter.entry = entry;
            return Some(spare);
        }
        None
    }

    fn take_slot(&mut self) -> (usize, u64) {
        while let Some(index) = self.free.pop() {
            let slot = &mut self.entries[index];
            let Some(generation) = slot.generation.checked_add(1) else {
                continue;
            };
            slot.generation = generation;
            return (index, generation);
        }
        let index = self.entries.len();
        self.entries.push(Slot {
            generation: 0,
            order_key: 0,
            job: None,
            waiter: None,
        });
        (index, 0)
    }

    fn take_job(&mut self, entry: ReadyEntry) -> Option<Job<'job, F>> {
        let slot = self.entries.get_mut(entry.index)?;
        if slot.generation != entry.generation {
            return None;
        }
        let job = slot.job.take()?;
        if let Some(waiter) = slot.waiter.take()
            && self.spare_waiters.len() < MAX_SPARE_WAITERS
        {
            self.spare_waiters.push(waiter);
        }
        self.free.push(entry.index);
        Some(job)
    }

    fn take_stranded(&mut self) -> Vec<(u64, Job<'job, F>)> {
        let mut stranded = Vec::new();
        for (index, slot) in self.entries.iter_mut().enumerate() {
            if let Some(job) = slot.job.take() {
                stranded.push((slot.order_key, job));
                slot.waiter.take();
                self.free.push(index);
            }
        }
        stranded
    }
}

/// A dependency-ordered admission scheduler over a task scope.
pub struct AdmissionScheduler<'job, F: Task<'job> = NoTask> {
    slots: Mutex<Slots<'job, F>>,
    ready: Arc<ReadyQueue>,
    /// One continuation slot per worker, built once.
    ///
    /// A slot per running job allocated its platform lock the first time that
    /// job touched it. dav2d gives each worker one context and reuses it for
    /// every task the worker runs; these are that context's continuation.
    continuations: Vec<ContinuationSlot<'job, F>>,
}

impl<'job, F: Task<'job>> AdmissionScheduler<'job, F> {
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Slots::default()),
            ready: Arc::new(ReadyQueue::default()),
            continuations: (0..MAX_WORKER_CONTINUATIONS)
                .map(|_| ContinuationSlot::new())
                .collect(),
        }
    }

    /// Stores `job` until every condition holds.
    ///
    /// If every condition holds by the end of registration, the job is admitted
    /// immediately; this includes a racing publication absorbed during
    /// registration. A publication satisfying the last unmet condition after
    /// registration only queues the job, so an external publisher must call
    /// [`Self::admit_ready`] with the same live task scope.
    pub fn submit<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        order_key: u64,
        conditions: &[Condition<'_>],
        job: Job<'job, F>,
    ) where
        'job: 'scope,
    {
        if conditions.iter().all(Condition::is_satisfied) {
            // Nothing left to wait on, so the job needs no waiter of its own.
            self.slots.lock().store_ready(order_key, job, &self.ready);
            self.admit_ready(scope);
            return;
        }
        let waiter = self
            .slots
            .lock()
            .store(conditions.len() + 1, order_key, job, &self.ready);
        for condition in conditions {
            if !condition.register(Arc::clone(&waiter)) {
                waiter.satisfy();
            }
        }
        if waiter.satisfy() {
            self.admit_ready(scope);
        }
    }

    /// Spawns all jobs that are admissible now.
    ///
    /// Running scheduler jobs call this automatically after their work. Drivers
    /// and other external publishers must call it after publishing a condition.
    pub fn admit_ready<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>) -> usize
    where
        'job: 'scope,
    {
        // With peers, hand the job to the pool. Alone there is no peer to hand
        // it to, and Rayon heap-allocates a job for every spawn, so this worker
        // runs it -- still in the order the ready queue yields, which is
        // priority order.
        let alone = crate::pool::current_pool_width() <= 1;
        let mut spawned = 0;
        while let Some(entry) = self.ready.pop() {
            let job = self.slots.lock().take_job(entry);
            let Some(job) = job else { continue };
            if alone {
                self.run_job(scope, job);
            } else {
                scope.spawn(move |scope| self.run_job(scope, job));
            }
            spawned += 1;
        }
        if spawned != 0 {
            notify_installed_pool_progress();
        }
        spawned
    }

    /// Drains ready jobs, leaving the first on `continuations` rather than
    /// spawning it.
    ///
    /// Rayon heap-allocates a job for every spawn, so a job admitted from
    /// inside [`Self::run_job`] rides that loop instead: the caller is already
    /// draining continuations, and one worker running its own successor needs
    /// no hand-off at all.
    fn admit_ready_continuing<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        continuations: &ContinuationSlot<'job, F>,
    ) -> usize
    where
        'job: 'scope,
    {
        // Alone, run in pop order rather than parking: a parked job resumes
        // after the rest, which would put the highest-priority one last.
        let alone = crate::pool::current_pool_width() <= 1;
        let mut spawned = 0;
        let mut parked = alone;
        while let Some(entry) = self.ready.pop() {
            let job = self.slots.lock().take_job(entry);
            let Some(mut job) = job else { continue };
            if !parked {
                match continuations.put(OrderedJob {
                    order_key: entry.order_key,
                    job,
                }) {
                    Ok(()) => {
                        parked = true;
                        spawned += 1;
                        continue;
                    }
                    Err(back) => job = back.job,
                }
            }
            if alone {
                self.run_job(scope, job);
            } else {
                scope.spawn(move |scope| self.run_job(scope, job));
            }
            spawned += 1;
        }
        if spawned != 0 {
            notify_installed_pool_progress();
        }
        spawned
    }

    /// This worker's continuation slot, or a shared one when it has no index.
    fn worker_continuations(&self) -> &ContinuationSlot<'job, F> {
        let index = crate::pool::current_worker_index().unwrap_or(0);
        let count = self.continuations.len().max(1);
        &self.continuations[index % count]
    }

    fn run_job<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>, mut job: Job<'job, F>)
    where
        'job: 'scope,
    {
        let continuations = self.worker_continuations();
        let admit = ScopeAdmit {
            scheduler: self,
            scope,
            continuations,
        };
        let mut continued = 0;
        loop {
            job.run(&admit);
            self.admit_ready_continuing(scope, continuations);
            let Some(next) = continuations.take() else {
                return;
            };
            if continued == CONTINUATION_BUDGET {
                self.submit(scope, next.order_key, &[], next.job);
                return;
            }
            continued += 1;
            job = next.job;
        }
    }

    /// Reports and releases jobs that remain stored, including ready jobs that
    /// an external publisher queued without a subsequent drain.
    ///
    /// # Errors
    /// Returns [`ParallelError::JobsNeverAdmitted`] when a job remains.
    pub fn finish(&self) -> Result<(), ParallelError> {
        let stranded = self.slots.lock().take_stranded();
        let Some(lowest_order_key) = stranded.iter().map(|(key, _)| *key).min() else {
            return Ok(());
        };
        Err(ParallelError::JobsNeverAdmitted {
            count: stranded.len(),
            lowest_order_key,
        })
    }
}

impl<'job, F: Task<'job>> Default for AdmissionScheduler<'job, F> {
    fn default() -> Self {
        Self::new()
    }
}

struct ScopeAdmit<'a, 'handle, 'scope, 'job, F: Task<'job>> {
    scheduler: &'scope AdmissionScheduler<'job, F>,
    scope: &'a TaskScope<'handle, 'scope>,
    continuations: &'a ContinuationSlot<'job, F>,
}

impl<'job, F: Task<'job>> Admit<'job, F> for ScopeAdmit<'_, '_, '_, 'job, F> {
    fn admit_ready(&self) -> usize {
        self.scheduler
            .admit_ready_continuing(self.scope, self.continuations)
    }

    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job, F>) {
        self.scheduler
            .submit(self.scope, order_key, conditions, job);
    }

    fn spawn_ready(&self, job: Job<'job, F>) {
        let scheduler = self.scheduler;
        self.scope.spawn(move |scope| scheduler.run_job(scope, job));
    }

    fn submit_ready_batch(&self, order_key: u64, mut jobs: Vec<Job<'job, F>>) {
        match jobs.len() {
            0 => {}
            1 => {
                if let Some(job) = jobs.pop() {
                    self.scheduler.submit(self.scope, order_key, &[], job);
                }
            }
            _ => self.scheduler.submit(
                self.scope,
                order_key,
                &[],
                Job::Boxed(Box::new(move |admit: &dyn Admit<'job, F>| {
                    for job in jobs {
                        admit.spawn_ready(job);
                    }
                })),
            ),
        }
    }

    fn continue_ready(&self, order_key: u64, job: Job<'job, F>) {
        let next = OrderedJob { order_key, job };
        if let Err(next) = self.continuations.put(next) {
            self.scheduler
                .submit(self.scope, next.order_key, &[], next.job);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::unwrap_used)]

    /// Wraps a test closure as a boxed job.
    fn boxed<'job, F: Task<'job>>(
        job: impl for<'a> FnOnce(&'a dyn Admit<'job, F>) + Send + 'job,
    ) -> Job<'job, F> {
        Job::Boxed(Box::new(job))
    }

    use super::*;
    use crate::pool::{WorkerPool, ready_task_scope};
    use crate::thread_count::ThreadCount;
    use std::sync::Barrier;
    use std::sync::atomic::AtomicBool;

    fn pool(threads: usize) -> WorkerPool {
        WorkerPool::new(ThreadCount::Fixed(threads.try_into().unwrap())).unwrap()
    }

    #[test]
    fn publication_racing_registration_admits_exactly_once() {
        for _ in 0..32 {
            let done = Arc::new(CompletionCell::new());
            let ran = AtomicUsize::new(0);
            let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
            let barrier = Arc::new(Barrier::new(2));
            let publish_done = Arc::clone(&done);
            let publish_barrier = Arc::clone(&barrier);
            let publisher = std::thread::spawn(move || {
                publish_barrier.wait();
                publish_done.set(()).unwrap();
            });
            pool(2).install(|| {
                ready_task_scope(|scope| {
                    barrier.wait();
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::completion(done.as_ref())],
                        boxed(|_| {
                            ran.fetch_add(1, Ordering::Relaxed);
                        }),
                    );
                    publisher.join().unwrap();
                    scheduler.admit_ready(scope);
                })
                .unwrap();
            });
            assert_eq!(ran.load(Ordering::Relaxed), 1);
            scheduler.finish().unwrap();
        }
    }

    #[test]
    fn mixed_conditions_wait_for_the_last_source() {
        let rows = WatermarkCell::new();
        let done = CompletionCell::completed(());
        let last = CompletionCell::new();
        let ran = AtomicUsize::new(0);
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(2).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[
                        Condition::watermark(&rows, 3),
                        Condition::completion(&done),
                        Condition::completion(&last),
                    ],
                    boxed(|_| {
                        ran.fetch_add(1, Ordering::Relaxed);
                    }),
                );
                rows.publish(3);
                scheduler.admit_ready(scope);
                assert_eq!(ran.load(Ordering::Relaxed), 0);
                last.set(()).unwrap();
                scheduler.admit_ready(scope);
            })
            .unwrap();
        });
        assert_eq!(ran.load(Ordering::Relaxed), 1);
        scheduler.finish().unwrap();
    }

    #[test]
    fn stale_generation_cannot_take_a_reused_slot() {
        let ready = Arc::new(ReadyQueue::default());
        let mut slots: Slots<'_, NoTask> = Slots::default();
        let first = slots.store(1, 0, boxed(|_| {}), &ready);
        let stale = first.entry;
        first.satisfy();
        assert!(slots.take_job(ready.pop().unwrap()).is_some());

        let second = slots.store(1, 1, boxed(|_| {}), &ready);
        assert_eq!(second.entry.index, stale.index);
        assert_ne!(second.entry.generation, stale.generation);
        ready.push(stale);
        assert!(slots.take_job(ready.pop().unwrap()).is_none());

        second.satisfy();
        assert!(slots.take_job(ready.pop().unwrap()).is_some());
    }

    #[test]
    fn concurrent_drains_run_each_job_once() {
        const JOBS: usize = 64;
        let rows = WatermarkCell::new();
        let visits: Vec<_> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(4).install(|| {
            ready_task_scope(|scope| {
                for (index, visit) in visits.iter().enumerate() {
                    scheduler.submit(
                        scope,
                        index as u64,
                        &[Condition::watermark(&rows, 1)],
                        boxed(move |admit| {
                            visit.fetch_add(1, Ordering::Relaxed);
                            admit.admit_ready();
                        }),
                    );
                }
                rows.publish(1);
                scheduler.admit_ready(scope);
            })
            .unwrap();
        });
        assert!(
            visits
                .iter()
                .all(|visit| visit.load(Ordering::Relaxed) == 1)
        );
        scheduler.finish().unwrap();
    }

    #[test]
    fn finish_reports_and_releases_stranded_jobs() {
        let gate = CompletionCell::<()>::new();
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(scope, 9, &[Condition::completion(&gate)], boxed(|_| {}));
                scheduler.submit(scope, 4, &[Condition::completion(&gate)], boxed(|_| {}));
            })
            .unwrap();
        });
        assert!(matches!(
            scheduler.finish(),
            Err(ParallelError::JobsNeverAdmitted {
                count: 2,
                lowest_order_key: 4
            })
        ));
        scheduler.finish().unwrap();
    }

    #[test]
    fn failed_watermark_releases_dependents() {
        let rows = WatermarkCell::new();
        let ran = AtomicBool::new(false);
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::watermark(&rows, 99)],
                    boxed(|_| ran.store(true, Ordering::Relaxed)),
                );
                rows.publish(WatermarkCell::FAILED);
                scheduler.admit_ready(scope);
            })
            .unwrap();
        });
        assert!(ran.load(Ordering::Relaxed));
        scheduler.finish().unwrap();
    }

    fn chain<'job>(admit: &dyn Admit<'job>, count: &'job AtomicUsize, left: usize) {
        count.fetch_add(1, Ordering::Relaxed);
        if left != 0 {
            admit.continue_ready(left as u64, boxed(move |a| chain(a, count, left - 1)));
        }
    }

    fn leaf_chain<'job>(
        admit: &dyn Admit<'job>,
        visits: &'job [AtomicUsize],
        id: usize,
        left: usize,
    ) {
        visits[id].fetch_add(1, Ordering::Relaxed);
        if left != 0 {
            admit.submit(
                id as u64,
                &[],
                boxed(move |admit| leaf_chain(admit, visits, id + 1, left - 1)),
            );
        }
    }

    #[test]
    fn continuations_are_iterative_and_allow_nested_work() {
        let chained = AtomicUsize::new(0);
        let nested = AtomicUsize::new(0);
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(2).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    boxed(|admit| {
                        admit.spawn_ready(boxed(|_| {
                            nested.fetch_add(1, Ordering::Relaxed);
                        }));
                        chain(admit, &chained, 32);
                    }),
                );
            })
            .unwrap();
        });
        assert_eq!(chained.load(Ordering::Relaxed), 33);
        assert_eq!(nested.load(Ordering::Relaxed), 1);
        scheduler.finish().unwrap();
    }

    #[test]
    fn panic_propagates_without_corrupting_scheduler() {
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool(1).install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(scope, 0, &[], boxed(|_| panic!("job panic")));
                })
                .unwrap();
            });
        }));
        assert!(result.is_err());
        scheduler.finish().unwrap();
    }

    #[test]
    fn seeded_mixed_dags_match_priority_model_and_run_once() {
        const JOBS: usize = 8;
        for (threads, seed) in [1usize, 4]
            .into_iter()
            .flat_map(|threads| [1usize, 3, 5, 7].map(move |seed| (threads, seed)))
        {
            let first_rows = WatermarkCell::new();
            let second_rows = WatermarkCell::new();
            let parsed = CompletionCell::new();
            let prepared = CompletionCell::new();
            let visits: Vec<_> = (0..JOBS * 4).map(|_| AtomicUsize::new(0)).collect();
            let order = Mutex::new(Vec::new());
            let mut model: Vec<_> = (0..JOBS)
                .map(|submission| {
                    let id = submission * seed % JOBS;
                    (((id + seed) % 3) as u64, submission, id)
                })
                .collect();
            model.sort_unstable();
            let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
            pool(threads).install(|| {
                ready_task_scope(|scope| {
                    for submission in 0..JOBS {
                        let id = submission * seed % JOBS;
                        let key = ((id + seed) % 3) as u64;
                        let visits = &visits;
                        let order = &order;
                        let first_rows = &first_rows;
                        let second_rows = &second_rows;
                        let parsed = &parsed;
                        let prepared = &prepared;
                        scheduler.submit(
                            scope,
                            key,
                            &[
                                Condition::watermark(first_rows, 2),
                                Condition::watermark(second_rows, 3),
                                Condition::completion(parsed),
                                Condition::completion(prepared),
                            ],
                            boxed(move |admit| {
                                assert_eq!(first_rows.current(), 2);
                                assert_eq!(second_rows.current(), 3);
                                assert!(parsed.is_set() && prepared.is_set());
                                let base = id * 4;
                                visits[base].fetch_add(1, Ordering::Relaxed);
                                order.lock().push(id);
                                admit.submit(
                                    key,
                                    &[Condition::completion(prepared)],
                                    boxed(move |admit| {
                                        leaf_chain(admit, visits, base + 1, 1);
                                    }),
                                );
                                admit.submit(
                                    key,
                                    &[Condition::watermark(second_rows, 3)],
                                    boxed(move |_| {
                                        visits[base + 3].fetch_add(1, Ordering::Relaxed);
                                    }),
                                );
                            }),
                        );
                    }
                    let first_rows = &first_rows;
                    let second_rows = &second_rows;
                    let parsed = &parsed;
                    let prepared = &prepared;
                    scheduler.submit(
                        scope,
                        u64::MAX,
                        &[],
                        boxed(move |admit| {
                            first_rows.publish(2);
                            parsed.set(()).unwrap();
                            admit.spawn_ready(boxed(|admit| {
                                second_rows.publish(3);
                                prepared.set(()).unwrap();
                                admit.admit_ready();
                            }));
                        }),
                    );
                })
                .unwrap();
            });
            if threads == 1 {
                let expected: Vec<_> = model.into_iter().map(|(_, _, id)| id).collect();
                assert_eq!(*order.lock(), expected);
            }
            assert!(
                visits
                    .iter()
                    .all(|visit| visit.load(Ordering::Relaxed) == 1)
            );
            scheduler.finish().unwrap();
        }
    }
}
