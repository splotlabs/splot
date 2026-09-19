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
//! publishing. Registration holds one outstanding unit and adds another before
//! registering each unmet condition. Releasing the final unit after enumeration
//! prevents publication from admitting a partially registered job. Reused slots carry generations to reject stale notices.
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
use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use crate::completion::CompletionCell;
use crate::error::ParallelError;
use crate::pool::{
    TaskScope, assist_pool_or_park, bind_installed_pool_progress, current_pool_width,
    notify_installed_pool_progress, pool_progress_snapshot,
};
use crate::progress::PoolProgressBindings;
use crate::watermark::WatermarkCell;

std::thread_local! {
    static RUNNER_BLOCKED: Cell<bool> = const { Cell::new(false) };
}

struct RunnerBlock(bool);
impl RunnerBlock {
    fn enter() -> Self {
        Self(RUNNER_BLOCKED.replace(true))
    }
}
impl Drop for RunnerBlock {
    fn drop(&mut self) {
        RUNNER_BLOCKED.set(self.0);
    }
}

#[derive(Default)]
struct Runners {
    enabled: AtomicBool,
    done: AtomicBool,
    started: AtomicUsize,
    active: AtomicUsize,
    jobs: AtomicUsize,
}
struct DriverDone<'a>(&'a Runners);
impl Drop for DriverDone<'_> {
    fn drop(&mut self) {
        self.0.done.store(true, Ordering::Release);
        notify_installed_pool_progress();
    }
}
struct RunnerExit<'a>(&'a Runners);
impl Drop for RunnerExit<'_> {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.enabled.store(false, Ordering::Release);
        }
        notify_installed_pool_progress();
    }
}
struct ActiveJob<'a>(&'a AtomicUsize);
impl Drop for ActiveJob<'_> {
    fn drop(&mut self) {
        if self.0.fetch_sub(1, Ordering::AcqRel) == 1 {
            notify_installed_pool_progress();
        }
    }
}

trait CompletionSource {
    fn register(&self, waiter: Waiter) -> bool;
    fn is_ready(&self) -> bool;
}

impl<V> CompletionSource for CompletionCell<V> {
    fn register(&self, waiter: Waiter) -> bool {
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

    fn register(self, waiter: Waiter) -> bool {
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
    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job, F>) {
        self.submit_iter(order_key, &mut conditions.iter().copied(), job);
    }
    /// Registers dependencies directly, without collecting a condition list.
    fn submit_iter(
        &self,
        order_key: u64,
        conditions: &mut dyn Iterator<Item = Condition<'_>>,
        job: Job<'job, F>,
    );
    /// Spawns a job already known to be ready, without a scheduler slot.
    fn spawn_ready(&self, job: Job<'job, F>);
    /// Submits proven-ready jobs as one ordered scheduler entry.
    fn submit_ready_batch(&self, order_key: u64, jobs: Vec<Job<'job, F>>);
    /// Records one serial successor for this worker.
    fn continue_ready(&self, order_key: u64, job: Job<'job, F>);
}

const CONTINUATION_BUDGET: usize = 8;

struct OrderedJob<'job, F: Task<'job>> {
    order_key: u64,
    job: Job<'job, F>,
}

struct ContinuationSlot<'job, F: Task<'job>>(Mutex<Option<OrderedJob<'job, F>>>);

impl<'job, F: Task<'job>> ContinuationSlot<'job, F> {
    fn new() -> Self {
        Self(Mutex::new(None))
    }

    fn put(&self, next: OrderedJob<'job, F>) -> Result<(), OrderedJob<'job, F>> {
        let mut pending = self.0.lock();
        if pending.is_some() {
            return Err(next);
        }
        *pending = Some(next);
        Ok(())
    }

    fn take(&self) -> Option<OrderedJob<'job, F>> {
        self.0.lock().take()
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
struct ReadyQueue {
    entries: Mutex<BinaryHeap<Reverse<ReadyEntry>>>,
    progress: PoolProgressBindings,
}

impl ReadyQueue {
    fn push(&self, entry: ReadyEntry) {
        let mut entries = self.entries.lock();
        let wake = entries.is_empty();
        entries.push(Reverse(entry));
        drop(entries);
        if wake {
            self.progress.notify();
        }
    }

    fn pop(&self) -> Option<ReadyEntry> {
        self.entries.lock().pop().map(|Reverse(entry)| entry)
    }
}

#[derive(Debug)]
struct WaiterPending {
    count: usize,
    entry: ReadyEntry,
}

#[derive(Debug)]
struct WaiterRecord {
    pending: Mutex<WaiterPending>,
    ready: Arc<ReadyQueue>,
}

#[derive(Clone, Debug)]
pub(crate) struct Waiter {
    record: Arc<WaiterRecord>,
    generation: u64,
}

#[derive(Debug)]
pub(crate) struct WeakWaiter {
    record: Weak<WaiterRecord>,
    generation: u64,
}

impl WeakWaiter {
    pub(crate) fn upgrade(&self) -> Option<Waiter> {
        self.record.upgrade().map(|record| Waiter {
            record,
            generation: self.generation,
        })
    }
}

impl Waiter {
    pub(crate) fn downgrade(&self) -> WeakWaiter {
        WeakWaiter {
            record: Arc::downgrade(&self.record),
            generation: self.generation,
        }
    }

    fn add_condition(&self) {
        let mut pending = self.record.pending.lock();
        if pending.entry.generation == self.generation {
            pending.count += 1;
        }
    }

    pub(crate) fn satisfy(&self) -> bool {
        let entry = {
            let mut pending = self.record.pending.lock();
            if pending.entry.generation != self.generation || pending.count == 0 {
                return false;
            }
            pending.count -= 1;
            if pending.count != 0 {
                return false;
            }
            pending.entry
        };
        self.record.ready.push(entry);
        true
    }
}

struct Slot<'job, F: Task<'job>> {
    generation: u64,
    order_key: u64,
    job: Option<Job<'job, F>>,
    waiter: Option<Arc<WaiterRecord>>,
}

struct Slots<'job, F: Task<'job>> {
    entries: Vec<Slot<'job, F>>,
    free: Vec<usize>,
    next_submission_order: u64,
}

impl<'job, F: Task<'job>> Default for Slots<'job, F> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            free: Vec::new(),
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
    ) -> Waiter {
        let (index, generation) = self.take_slot();
        let entry = ReadyEntry {
            order_key,
            submission_order: self.next_submission_order,
            index,
            generation,
        };
        self.next_submission_order = self.next_submission_order.wrapping_add(1);
        let slot = &mut self.entries[index];
        let record = slot.waiter.get_or_insert_with(|| {
            Arc::new(WaiterRecord {
                pending: Mutex::new(WaiterPending {
                    count: pending,
                    entry,
                }),
                ready: Arc::clone(ready),
            })
        });
        *record.pending.lock() = WaiterPending {
            count: pending,
            entry,
        };
        slot.order_key = order_key;
        slot.job = Some(job);
        Waiter {
            record: Arc::clone(record),
            generation,
        }
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
        ready.push(entry);
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
        self.free.push(entry.index);
        Some(job)
    }

    fn take_stranded(&mut self) -> Vec<(u64, Job<'job, F>)> {
        let mut stranded = Vec::new();
        for (index, slot) in self.entries.iter_mut().enumerate() {
            if let Some(job) = slot.job.take() {
                stranded.push((slot.order_key, job));
                if let Some(waiter) = &slot.waiter {
                    waiter.pending.lock().count = 0;
                }
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
    runners: Runners,
}

impl<'job, F: Task<'job>> AdmissionScheduler<'job, F> {
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Slots::default()),
            ready: Arc::new(ReadyQueue::default()),
            runners: Runners::default(),
        }
    }

    /// Runs a driver with persistent workers consuming this scheduler's queue.
    /// Nested tasks keep the ordinary scoped path when every worker is occupied.
    pub fn run_with_runners<'scope, R>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        driver: impl FnOnce() -> R,
    ) -> R
    where
        'job: 'scope,
    {
        if current_pool_width() <= 1 || self.runners.enabled.swap(true, Ordering::AcqRel) {
            return driver();
        }
        let _blocked = RunnerBlock::enter();
        bind_installed_pool_progress(&self.ready.progress);
        self.runners.done.store(false, Ordering::Release);
        self.runners.started.store(0, Ordering::Release);
        let _done = DriverDone(&self.runners);
        scope.broadcast(move |scope| {
            if RUNNER_BLOCKED.get() {
                self.runners.started.fetch_add(1, Ordering::AcqRel);
                notify_installed_pool_progress();
                return;
            }
            self.runners.active.fetch_add(1, Ordering::AcqRel);
            let _exit = RunnerExit(&self.runners);
            self.runners.started.fetch_add(1, Ordering::AcqRel);
            notify_installed_pool_progress();
            self.run_queue(scope);
        });
        loop {
            let snapshot = pool_progress_snapshot();
            if self.runners.started.load(Ordering::Acquire) == current_pool_width() {
                break;
            }
            assist_pool_or_park(&snapshot);
        }
        if self.runners.active.load(Ordering::Acquire) == 0 {
            self.runners.enabled.store(false, Ordering::Release);
        }
        driver()
    }

    fn run_queue<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>)
    where
        'job: 'scope,
    {
        loop {
            let snapshot = pool_progress_snapshot();
            if self.assist_ready(scope) {
                continue;
            }
            if self.runners.done.load(Ordering::Acquire) {
                let entries = self.ready.entries.lock();
                if entries.is_empty() && self.runners.jobs.load(Ordering::Acquire) == 0 {
                    break;
                }
            }
            assist_pool_or_park(&snapshot);
        }
    }

    /// Runs one queued job while the driver is waiting for a dependency.
    pub fn assist_ready<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>) -> bool
    where
        'job: 'scope,
    {
        let entry = {
            let mut entries = self.ready.entries.lock();
            let Some(Reverse(entry)) = entries.pop() else {
                return false;
            };
            self.runners.jobs.fetch_add(1, Ordering::AcqRel);
            entry
        };
        let _active = ActiveJob(&self.runners.jobs);
        let job = self.slots.lock().take_job(entry);
        if let Some(job) = job {
            self.run_job(scope, job);
        }
        true
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
        self.submit_iter(scope, order_key, &mut conditions.iter().copied(), job);
    }

    /// Registers a stream of dependencies before making the job admissible.
    pub fn submit_iter<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        order_key: u64,
        conditions: &mut dyn Iterator<Item = Condition<'_>>,
        job: Job<'job, F>,
    ) where
        'job: 'scope,
    {
        let mut pending = conditions.filter(|condition| !condition.is_satisfied());
        let Some(first) = pending.next() else {
            self.slots.lock().store_ready(order_key, job, &self.ready);
            self.admit_ready(scope);
            return;
        };
        let waiter = self.slots.lock().store(1, order_key, job, &self.ready);
        for condition in std::iter::once(first).chain(pending) {
            waiter.add_condition();
            if !condition.register(waiter.clone()) {
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
        if self.runners.enabled.load(Ordering::Acquire) {
            return 0;
        }
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
        if self.runners.enabled.load(Ordering::Acquire) {
            let mut next = continuations.0.lock();
            if next.is_none()
                && let Some(entry) = self.ready.pop()
                && let Some(job) = self.slots.lock().take_job(entry)
            {
                *next = Some(OrderedJob {
                    order_key: entry.order_key,
                    job,
                });
                return 1;
            }
            return 0;
        }
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

    fn run_job<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>, mut job: Job<'job, F>)
    where
        'job: 'scope,
    {
        let _blocked = RunnerBlock::enter();
        let continuations = ContinuationSlot::new();
        let admit = ScopeAdmit {
            scheduler: self,
            scope,
            continuations: &continuations,
        };
        let mut continued: usize = 0;
        loop {
            job.run(&admit);
            self.admit_ready_continuing(scope, &continuations);
            let Some(next) = continuations.take() else {
                return;
            };
            if continued == CONTINUATION_BUDGET && crate::pool::current_pool_width() > 1 {
                self.submit(scope, next.order_key, &[], next.job);
                return;
            }
            continued = continued.saturating_add(1);
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

    fn submit_iter(
        &self,
        order_key: u64,
        conditions: &mut dyn Iterator<Item = Condition<'_>>,
        job: Job<'job, F>,
    ) {
        self.scheduler
            .submit_iter(self.scope, order_key, conditions, job);
    }

    fn spawn_ready(&self, job: Job<'job, F>) {
        if self.scheduler.runners.enabled.load(Ordering::Acquire) {
            self.scheduler.submit(self.scope, 0, &[], job);
            return;
        }
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
    fn task_slot_reuses_its_waiter_while_stale_notices_race() {
        let ready = Arc::new(ReadyQueue::default());
        let mut slots = Slots::<NoTask>::default();
        let stale = slots.store(1, 0, boxed(|_| {}), &ready);
        let weak = stale.downgrade();
        assert!(stale.satisfy());
        drop(slots.take_job(ready.pop().unwrap()));
        std::thread::scope(|threads| {
            for _ in 0..12 {
                let stale = stale.clone();
                threads.spawn(move || {
                    for _ in 0..1200 {
                        assert!(!stale.satisfy());
                    }
                });
            }
            for index in 0..1200 {
                slots.store_ready(index, boxed(|_| {}), &ready);
                drop(slots.take_job(ready.pop().unwrap()));
                let current = slots.store(1, index, boxed(|_| {}), &ready);
                assert!(Arc::ptr_eq(&current.record, &stale.record));
                assert!(!weak.upgrade().unwrap().satisfy());
                assert!(ready.pop().is_none());
                assert!(current.satisfy());
                drop(slots.take_job(ready.pop().unwrap()));
            }
        });
        assert_eq!(slots.entries.len(), 1);
        let stranded = slots.store(1, 0, boxed(|_| {}), &ready);
        drop(slots.take_stranded());
        assert!(!stranded.satisfy());
        assert!(ready.pop().is_none());
    }

    #[test]
    fn dependency_publication_during_enumeration_cannot_admit_early() {
        let first = CompletionCell::new();
        let second = CompletionCell::new();
        let ran = AtomicBool::new(false);
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                let mut conditions = std::iter::once(Condition::completion(&first)).chain(
                    std::iter::once_with(|| {
                        first.set(()).unwrap();
                        Condition::completion(&second)
                    }),
                );
                scheduler.submit_iter(
                    scope,
                    0,
                    &mut conditions,
                    boxed(|_| {
                        assert!(second.is_set());
                        ran.store(true, Ordering::Release);
                    }),
                );
                assert!(!ran.load(Ordering::Acquire));
                second.set(()).unwrap();
                scheduler.admit_ready(scope);
            })
            .unwrap();
        });
        scheduler.finish().unwrap();
        assert!(ran.load(Ordering::Acquire));
    }

    #[test]
    fn persistent_runners_wake_after_external_publication() {
        let gates: Vec<_> = (0..128).map(|_| CompletionCell::new()).collect();
        let visits = AtomicUsize::new(0);
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(12).install(|| {
            ready_task_scope(|scope| {
                scheduler.run_with_runners(scope, || {
                    let driver = std::thread::current().id();
                    let visits = &visits;
                    for (index, gate) in gates.iter().enumerate() {
                        scheduler.submit(
                            scope,
                            index as u64,
                            &[Condition::completion(gate)],
                            boxed(move |_| {
                                assert_ne!(std::thread::current().id(), driver);
                                visits.fetch_add(1, Ordering::Release);
                            }),
                        );
                    }
                    std::thread::scope(|threads| {
                        threads.spawn(|| {
                            for gate in &gates {
                                gate.set(()).unwrap();
                            }
                        });
                        loop {
                            let snapshot = pool_progress_snapshot();
                            if visits.load(Ordering::Acquire) == gates.len() {
                                break;
                            }
                            assist_pool_or_park(&snapshot);
                        }
                    });
                });
            })
            .unwrap();
        });
        scheduler.finish().unwrap();
    }

    #[test]
    fn persistent_runners_service_nested_tile_scopes() {
        for width in [2, 4, 12] {
            let done = CompletionCell::new();
            let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
            pool(width).install(|| {
                ready_task_scope(|scope| {
                    scheduler.run_with_runners(scope, || {
                        scheduler.submit(
                            scope,
                            0,
                            &[],
                            boxed(|_| {
                                let visits = AtomicUsize::new(0);
                                let nested: AdmissionScheduler<'_, NoTask> =
                                    AdmissionScheduler::new();
                                ready_task_scope(|scope| {
                                    for index in 0..128 {
                                        nested.submit(
                                            scope,
                                            index,
                                            &[],
                                            boxed(|_| {
                                                visits.fetch_add(1, Ordering::Relaxed);
                                            }),
                                        );
                                    }
                                })
                                .unwrap();
                                nested.finish().unwrap();
                                assert_eq!(visits.load(Ordering::Relaxed), 128);
                                done.set(()).unwrap();
                            }),
                        );
                        let () = done.wait_with_pool_assist();
                    });
                })
                .unwrap();
            });
            scheduler.finish().unwrap();
        }
    }

    #[test]
    fn concurrent_persistent_drivers_do_not_trap_each_other() {
        use rayon::prelude::*;
        pool(12).install(|| {
            (0..24).into_par_iter().for_each(|_| {
                let done = CompletionCell::new();
                let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
                ready_task_scope(|scope| {
                    scheduler.run_with_runners(scope, || {
                        scheduler.submit(
                            scope,
                            0,
                            &[],
                            boxed(|_| {
                                done.set(()).unwrap();
                            }),
                        );
                        let () = done.wait_with_pool_assist();
                    });
                })
                .unwrap();
                scheduler.finish().unwrap();
            });
        });
    }

    #[test]
    fn waiting_driver_assists_when_the_runner_is_occupied() {
        let started = CompletionCell::new();
        let released = CompletionCell::new();
        let done = CompletionCell::new();
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(2).install(|| {
            ready_task_scope(|scope| {
                scheduler.run_with_runners(scope, || {
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        boxed(|_| {
                            started.set(()).unwrap();
                            let () = released.wait_with_pool_assist();
                            done.set(()).unwrap();
                        }),
                    );
                    let () = started.wait_with_pool_assist();
                    let driver = std::thread::current().id();
                    let release = &released;
                    scheduler.submit(
                        scope,
                        1,
                        &[],
                        boxed(move |_| {
                            assert_eq!(std::thread::current().id(), driver);
                            release.set(()).unwrap();
                        }),
                    );
                    let () = done.wait_with_assist(|| scheduler.assist_ready(scope));
                });
            })
            .unwrap();
        });
        scheduler.finish().unwrap();
    }

    #[test]
    fn persistent_runners_exit_when_the_driver_unwinds() {
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool(4).install(|| {
                ready_task_scope(|scope| {
                    let _: () = scheduler.run_with_runners(scope, || panic!("driver failure"));
                })
                .unwrap();
            });
        }));
        assert!(result.is_err());
        assert_eq!(scheduler.runners.active.load(Ordering::Acquire), 0);
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
        let stale = first.record.pending.lock().entry;
        first.satisfy();
        assert!(slots.take_job(ready.pop().unwrap()).is_some());

        let second = slots.store(1, 1, boxed(|_| {}), &ready);
        assert_eq!(second.record.pending.lock().entry.index, stale.index);
        assert_ne!(second.generation, stale.generation);
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
    fn shared_continuation_slot_does_not_hide_concurrent_submissions() {
        let slot: ContinuationSlot<'_, NoTask> = ContinuationSlot::new();
        let mut taken = 0;
        let submitted = std::thread::scope(|scope| {
            let producer = scope.spawn(|| {
                (0..100_000)
                    .filter(|&order_key| {
                        slot.put(OrderedJob {
                            order_key,
                            job: boxed(|_| {}),
                        })
                        .is_ok()
                    })
                    .count()
            });
            while !producer.is_finished() {
                taken += usize::from(slot.take().is_some());
            }
            producer.join().unwrap()
        });
        taken += usize::from(slot.take().is_some());
        assert_eq!(taken, submitted);
    }

    #[test]
    fn nested_job_does_not_run_its_parents_continuation() {
        let events = Mutex::new(Vec::new());
        let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    boxed(|admit| {
                        admit.continue_ready(1, boxed(|_| events.lock().push("successor")));
                        admit.spawn_ready(boxed(|_| events.lock().push("nested")));
                        events.lock().push("parent returned");
                    }),
                );
            })
            .unwrap();
        });
        assert_eq!(*events.lock(), ["nested", "parent returned", "successor"]);
        scheduler.finish().unwrap();
    }

    #[test]
    fn continuations_are_iterative_and_allow_nested_work() {
        for threads in [1, 2] {
            let chained = AtomicUsize::new(0);
            let nested = AtomicUsize::new(0);
            let scheduler: AdmissionScheduler<'_, NoTask> = AdmissionScheduler::new();
            pool(threads).install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        boxed(|admit| {
                            admit.spawn_ready(boxed(|_| {
                                nested.fetch_add(1, Ordering::Relaxed);
                            }));
                            chain(admit, &chained, 100_000);
                        }),
                    );
                })
                .unwrap();
            });
            assert_eq!(chained.load(Ordering::Relaxed), 100_001);
            assert_eq!(nested.load(Ordering::Relaxed), 1);
            scheduler.finish().unwrap();
        }
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
