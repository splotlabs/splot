// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Dependency-ordered task admission ([`AdmissionScheduler`]).
//!
//! No pool task may wait. A task that blocks on state another frame produces can
//! be stolen onto the stack of the very task that would publish that state, and
//! the pool deadlocks — work-stealing dependency inversion. This scheduler
//! removes the need to wait: a job is submitted together with the
//! [`Condition`]s it needs, it is spawned only once every condition already
//! holds, and the publisher that satisfies the last condition is what admits it.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
//!
//! # Ordering guarantee
//!
//! Admissible jobs queue in a min-heap keyed by `order_key`, ties broken by
//! submission order, and each pop takes the lowest-keyed job queued at that
//! moment. A single drain therefore spawns the jobs it finds oldest-first.
//!
//! That is the whole guarantee, and it is a scheduling preference, not a
//! correctness property: what a job may do is fixed by the [`Condition`]s it
//! was submitted with, never by when it spawns. Two other things follow.
//! Drains run concurrently, and one that has popped a job but not yet spawned
//! it does not hold back another drain, so two jobs found by different drains
//! reach the pool in an unspecified order. And there is no global order at all:
//! a job that becomes admissible after a drain has passed its key waits for a
//! later drain. Serializing pop-to-spawn would buy neither, since a publish
//! that lands a moment later legally reorders the same pair; it would only put
//! one lock on the path every spawn takes. Spawn order fixes the pool's enqueue
//! order and nothing else — never which worker runs what, nor when.
//!
//! # Drains
//!
//! Spawning needs a [`TaskScope`], which a publisher running off the pool (the
//! driver) or inside another job does not always hold, so satisfying a condition
//! only queues a job; a *drain* spawns it. Drains run after every
//! scheduler-spawned job body, wherever a caller asks for one
//! ([`AdmissionScheduler::admit_ready`], or [`Admit::admit_ready`] from inside a
//! job that wants its dependents started before its own body ends), and at
//! [`AdmissionScheduler::submit`] when that submission queued its own job.
//!
//! Every queued entry is pushed by the single `satisfy` call that cleared its
//! job's last condition, and that caller drains after it: a publish inside a job
//! body is followed by the scheduler's post-body drain on the same thread,
//! `submit`'s own closing satisfy is followed by `submit`'s drain, and a driver
//! that publishes outside a job must call [`AdmissionScheduler::admit_ready`]
//! itself. A `submit` that leaves a condition outstanding pushed nothing, so
//! skipping its drain takes no entry's drainer away — it only declines to spawn
//! work another thread is already bound to drain.
//!
//! # Proven-ready jobs
//!
//! A job whose caller already knows it may run has nothing for the scheduler to
//! decide, and [`Admit::spawn_ready`] spawns it into the same scope directly:
//! no slot, no token, no heap round trip, and none of the lock acquisitions
//! those three cost. It keeps everything a submitted job has that the rest of
//! this module reasons about — the same scope and handle, and the same
//! post-body drain, so its publishes admit their dependents on the thread that
//! made them. It gives up only the `order_key`, entering the pool in call order
//! instead of the heap's, which is a scheduling preference and not a
//! correctness property.
//!
//! [`Admit::continue_ready`] records a single proven-ready serial successor for
//! the current job instead. The successor runs on the same worker only after
//! the predecessor returns and the post-body drain exposes other ready work;
//! a fixed budget returns long chains to normal ordered submission.
//!
//! Nothing is lost to [`AdmissionScheduler::finish`] either: it reports jobs
//! that never became admissible, and a job spawned this way was admissible when
//! it was handed over, so it can never be one of them.
//!
//! # Reentrancy, panics, and teardown
//!
//! Admission runs on arbitrary pool threads inside unrelated jobs. The drain is
//! iterative and never holds a lock across [`TaskScope::spawn`] or across a job
//! call; the scheduler takes at most one of its two locks at a time, so it
//! cannot deadlock against itself. A job that panics unwinds through the rayon
//! scope, which propagates the panic after the scope drains; nothing is
//! corrupted (waiter tokens are reference-counted and lock poisoning is
//! ignored), but the panicking job's publishes never happen, so its dependents
//! stay unadmitted and [`AdmissionScheduler::finish`] reports them. Jobs left
//! unadmitted when the scope ends are a bug in the caller's dependency graph;
//! `finish` surfaces them as a typed error instead of dropping them silently.
//!
//! # Slot and token reuse
//!
//! A ready entry names both a slot index and its generation. Taking a job moves
//! the job and waiter handle out under the slot lock before the index reaches
//! the free list, so the running closure owns everything it needs and the slot
//! may immediately serve another submission. A generation at `u64::MAX` is
//! retired instead of wrapped; even at one billion reuses per second reaching
//! that point would take more than five centuries, and retirement prevents the
//! theoretical wrap from aliasing an old ready entry.
//!
//! Token reuse is separate. Each submission publishes a new, immutable
//! `AdmissionWaiterHandle` that snapshots its generation. Condition cells
//! retain weak or strong references to that handle, never to the pooled token
//! state. The token is pooled only after the job has run and the handle is
//! uniquely detached; stale weak handles then cannot upgrade to a new job, and
//! stale strong handles prevent token pooling until they disappear.
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::completion::CompletionCell;
use crate::error::ParallelError;
use crate::pool::{TaskScope, notify_installed_pool_progress};
use crate::watermark::WatermarkCell;

/// The scheduler-side token a condition source notifies once its condition
/// holds.
///
/// The scheduler retains each token until its job runs, while condition sources
/// store reference-counted handles to it. The trait therefore carries no
/// lifetime and a source may outlive the scheduler that registered with it.
/// `Debug` keeps the cells that hold waiter lists printable.
pub trait AdmissionWaiter: fmt::Debug + Send + Sync {
    /// Reports that one registered condition now holds. Called at most once per
    /// successful registration.
    fn satisfy(&self);
}

/// A one-shot completion slot usable as an admission condition.
///
/// Implemented for every [`CompletionCell`], which is what makes
/// [`Condition::Completion`] usable without naming the cell's value type.
pub trait CompletionSource {
    /// Registers `waiter` to fire when the slot is set, returning `false` when
    /// it is set already (the waiter is then never called).
    fn register_completion(&self, waiter: Arc<dyn AdmissionWaiter>) -> bool;
}

impl<V> CompletionSource for CompletionCell<V> {
    fn register_completion(&self, waiter: Arc<dyn AdmissionWaiter>) -> bool {
        self.register_waiter(waiter)
    }
}

/// One dependency a submitted job needs before it may run.
#[derive(Clone, Copy)]
pub enum Condition<'a> {
    /// The watermark must have reached the given threshold.
    Watermark(&'a WatermarkCell, usize),
    /// The completion slot must have been set.
    Completion(&'a dyn CompletionSource),
}

impl Condition<'_> {
    /// Registers `waiter` with this condition's source, returning `false` when
    /// the condition already holds.
    fn register(self, waiter: Arc<dyn AdmissionWaiter>) -> bool {
        match self {
            Self::Watermark(cell, threshold) => cell.register(threshold, waiter),
            Self::Completion(cell) => cell.register_completion(waiter),
        }
    }
}

/// A unit of deferred work, run once every condition it was submitted with
/// holds.
///
/// The job receives an [`Admit`] handle rather than capturing the scheduler, so
/// its type never mentions the pool scope's lifetime; `'job` is the lifetime of
/// the state it borrows.
pub type Job<'job> = Box<dyn for<'a> FnOnce(&'a dyn Admit<'job>) + Send + 'job>;

/// What a running job may do with the scheduler that spawned it.
pub trait Admit<'job>: Sync {
    /// Spawns every job that is admissible now, returning how many were
    /// spawned. A publisher calls this after publishing so its dependents start
    /// before its own body ends; the scheduler drains again once the body
    /// returns, so a call in tail position only repeats that drain.
    fn admit_ready(&self) -> usize;

    /// Submits a successor under the same rules as
    /// [`AdmissionScheduler::submit`]; an empty condition list spawns it at
    /// once.
    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job>);

    /// Spawns a job the caller has already proven ready, skipping the scheduler
    /// slot, token, and ready heap that [`Admit::submit`] would spend on it.
    ///
    /// The job lands in the same scope, receives the same handle, and is drained
    /// after exactly as a submitted job is, so it may publish conditions and
    /// submit gated successors. What it gives up is the `order_key`: it enters
    /// the pool in call order rather than competing with the heap's queued jobs,
    /// so reserve it for work whose start order is a matter of indifference.
    fn spawn_ready(&self, job: Job<'job>);

    /// Runs one proven-ready serial successor on this worker after the current
    /// job returns and the scheduler has exposed any other ready work.
    ///
    /// At most a small fixed number of successive links run inline. Once that
    /// budget is spent, the remaining job is submitted normally under
    /// `order_key`, so a long chain cannot monopolize one worker. Use this only
    /// when the successor is the current job's single serial continuation and
    /// no scheduler priority decision is required before it runs. Register the
    /// continuation as the predecessor's final action: a later panic unwinds
    /// and drops the not-yet-started successor with it.
    fn continue_ready(&self, order_key: u64, job: Job<'job>);
}

/// Test-only tally of the work the drain and fast paths are meant to avoid.
///
/// `transitions` counts ready-heap pushes, and over a joined scope every push
/// was popped, so the two fast-path savings a caller cares about — the slot a
/// job did not take and the heap round trip it did not make — are the gap
/// between `submitted` and `conditionless`, and the drop in `transitions`, when
/// the same graph is expressed with [`Admit::spawn_ready`].
#[cfg(test)]
#[derive(Debug, Default)]
struct AdmissionCounters {
    satisfies: AtomicUsize,
    transitions: AtomicUsize,
    drains: AtomicUsize,
    empty_drains: AtomicUsize,
    submitted: AtomicUsize,
    conditionless: AtomicUsize,
    direct: AtomicUsize,
    continued: AtomicUsize,
    continuation_fallbacks: AtomicUsize,
    maximum_continuation_burst: AtomicUsize,
}

const CONTINUATION_BUDGET: usize = 8;

struct OrderedJob<'job> {
    order_key: u64,
    job: Job<'job>,
}

struct ContinuationSlot<'job> {
    pending: Mutex<Option<OrderedJob<'job>>>,
}

impl Default for ContinuationSlot<'_> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }
}

impl<'job> ContinuationSlot<'job> {
    fn put(&self, next: OrderedJob<'job>) -> Result<(), OrderedJob<'job>> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.is_some() {
            return Err(next);
        }
        *pending = Some(next);
        Ok(())
    }

    fn take(&self) -> Option<OrderedJob<'job>> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// One slot occupancy's identity, incremented before an index is reused.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SlotGeneration(u64);

impl SlotGeneration {
    const INITIAL: Self = Self(0);

    fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// One queued job identity and its stable scheduling order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReadyEntry {
    order_key: u64,
    submission_order: u64,
    index: usize,
    generation: SlotGeneration,
}

/// The queue of jobs whose conditions all hold, ordered by key then submission.
///
/// Shared with the waiter tokens through an `Arc` and free of borrowed state, so
/// a condition source that outlives the scheduler can still push into it
/// harmlessly.
#[derive(Debug, Default)]
struct ReadyQueue {
    entries: Mutex<BinaryHeap<Reverse<ReadyEntry>>>,
    #[cfg(test)]
    counters: AdmissionCounters,
}

impl ReadyQueue {
    /// Queues one generation of a slot for the next drain.
    fn push(&self, entry: ReadyEntry) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Reverse(entry));
    }

    /// Takes the queued slot with the lowest `order_key`.
    fn pop(&self) -> Option<ReadyEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .map(|Reverse(entry)| entry)
    }
}

/// One job's outstanding-condition count.
///
/// Counting down instead of up means the thread that satisfies the last
/// condition is the one that queues the job, with no lock between the sources.
/// [`AdmissionScheduler::submit`] seeds the count one above the condition count
/// and clears that extra unit only once every source has been registered, so a
/// condition firing mid-registration cannot queue the job early. That closing
/// satisfy is therefore the only one a submission can observe a transition from.
#[derive(Debug)]
struct AdmissionToken {
    pending: AtomicUsize,
    generation: AtomicU64,
    ready: Arc<ReadyQueue>,
}

impl AdmissionToken {
    fn reset(&self, pending: usize, generation: SlotGeneration) {
        self.generation.store(generation.0, Ordering::Release);
        self.pending.store(pending, Ordering::Release);
    }

    /// Clears one outstanding condition, reporting whether this call is the one
    /// that took the count to zero and queued the job.
    ///
    /// At most one call per token can report `true`, and the thread it reports
    /// it on is the thread that pushed the ready entry, which is what lets a
    /// caller holding a [`TaskScope`] tell a drain it owes from one it does not.
    fn satisfy_reporting(&self, entry: ReadyEntry) -> bool {
        #[cfg(test)]
        self.ready
            .counters
            .satisfies
            .fetch_add(1, Ordering::Relaxed);
        if self.generation.load(Ordering::Acquire) != entry.generation.0 {
            return false;
        }
        let mut pending = self.pending.load(Ordering::Acquire);
        while pending > 0 {
            match self.pending.compare_exchange_weak(
                pending,
                pending - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    if pending > 1 {
                        return false;
                    }
                    #[cfg(test)]
                    self.ready
                        .counters
                        .transitions
                        .fetch_add(1, Ordering::Relaxed);
                    self.ready.push(entry);
                    return true;
                }
                Err(observed) => pending = observed,
            }
        }
        false
    }
}

/// One job generation's identity, retained while any condition may notify it.
///
/// The token state may be pooled, but this handle is never reset or reused. A
/// stale weak condition handle therefore cannot upgrade to a later job even if
/// the allocator and scheduler both reuse their underlying storage.
#[derive(Debug)]
struct AdmissionWaiterHandle {
    entry: ReadyEntry,
    token: Arc<AdmissionToken>,
}

impl AdmissionWaiterHandle {
    fn satisfy_reporting(&self) -> bool {
        self.token.satisfy_reporting(self.entry)
    }
}

impl AdmissionWaiter for AdmissionWaiterHandle {
    fn satisfy(&self) {
        self.satisfy_reporting();
    }
}

/// One submitted job and the key it is admitted under.
struct Slot<'job> {
    generation: SlotGeneration,
    order_key: u64,
    occupied_position: Option<usize>,
    job: Option<Job<'job>>,
    waiter: Option<Arc<AdmissionWaiterHandle>>,
}

#[derive(Default)]
struct SchedulerSlots<'job> {
    entries: Vec<Slot<'job>>,
    free: Vec<usize>,
    occupied_indices: Vec<usize>,
    tokens: Vec<Arc<AdmissionToken>>,
    next_submission_order: u64,
    #[cfg(test)]
    peak_occupied: usize,
    #[cfg(test)]
    reuses: usize,
}

impl<'job> SchedulerSlots<'job> {
    fn take_token(
        &mut self,
        pending: usize,
        generation: SlotGeneration,
        ready: &Arc<ReadyQueue>,
    ) -> Arc<AdmissionToken> {
        if let Some(token) = self.tokens.pop() {
            token.reset(pending, generation);
            return token;
        }
        Arc::new(AdmissionToken {
            pending: AtomicUsize::new(pending),
            generation: AtomicU64::new(generation.0),
            ready: Arc::clone(ready),
        })
    }

    fn store_job(
        &mut self,
        pending: usize,
        order_key: u64,
        job: Job<'job>,
        ready: &Arc<ReadyQueue>,
    ) -> Arc<AdmissionWaiterHandle> {
        let (index, generation) = self.take_slot();
        let submission_order = self.next_submission_order;
        self.next_submission_order = self.next_submission_order.wrapping_add(1);
        let token = self.take_token(pending, generation, ready);
        let waiter = Arc::new(AdmissionWaiterHandle {
            entry: ReadyEntry {
                order_key,
                submission_order,
                index,
                generation,
            },
            token,
        });
        let occupied_position = self.occupied_indices.len();
        let slot = &mut self.entries[index];
        slot.order_key = order_key;
        slot.occupied_position = Some(occupied_position);
        slot.job = Some(job);
        slot.waiter = Some(Arc::clone(&waiter));
        self.occupied_indices.push(index);
        #[cfg(test)]
        {
            self.peak_occupied = self.peak_occupied.max(self.occupied_indices.len());
        }
        waiter
    }

    fn take_slot(&mut self) -> (usize, SlotGeneration) {
        while let Some(index) = self.free.pop() {
            let Some(slot) = self.entries.get_mut(index) else {
                continue;
            };
            let Some(generation) = slot.generation.next() else {
                continue;
            };
            slot.generation = generation;
            #[cfg(test)]
            {
                self.reuses += 1;
            }
            return (index, generation);
        }

        let index = self.entries.len();
        self.entries.push(Slot {
            generation: SlotGeneration::INITIAL,
            order_key: 0,
            occupied_position: None,
            job: None,
            waiter: None,
        });
        (index, SlotGeneration::INITIAL)
    }

    fn take_job(&mut self, ready: ReadyEntry) -> Option<(Job<'job>, Arc<AdmissionWaiterHandle>)> {
        let slot = self.entries.get_mut(ready.index)?;
        if slot.generation != ready.generation || slot.job.is_none() || slot.waiter.is_none() {
            return None;
        }
        let occupied_position = slot.occupied_position?;
        if self.occupied_indices.get(occupied_position) != Some(&ready.index) {
            return None;
        }
        let job = slot.job.take()?;
        let waiter = slot.waiter.take()?;
        slot.occupied_position = None;
        self.occupied_indices.swap_remove(occupied_position);
        if let Some(&moved_index) = self.occupied_indices.get(occupied_position) {
            self.entries[moved_index].occupied_position = Some(occupied_position);
        }
        self.free.push(ready.index);
        Some((job, waiter))
    }

    fn take_stranded(&mut self) -> Vec<(u64, Job<'job>)> {
        let mut stranded = Vec::with_capacity(self.occupied_indices.len());
        for index in core::mem::take(&mut self.occupied_indices) {
            let slot = &mut self.entries[index];
            if let Some(job) = slot.job.take() {
                stranded.push((slot.order_key, job));
            }
            slot.waiter.take();
            slot.occupied_position = None;
            self.free.push(index);
        }
        stranded
    }
}

/// A dependency-ordered admission scheduler over a [`TaskScope`].
///
/// Create it **before** entering [`crate::ready_task_scope`] and after the state
/// its jobs borrow, then share `&scheduler` into the scope: the jobs it holds
/// borrow that state for `'job`, and a job is spawned into the scope only once
/// every condition it named already holds. Memory is one small slot per
/// concurrently occupied job plus reusable token state, so it tracks peak live
/// work rather than historical submissions; use one scheduler per scope.
///
/// Call [`AdmissionScheduler::finish`] once the scope has joined. It is the only
/// report of jobs that were never admitted: dropping the scheduler instead
/// releases them without a diagnosis, and the scheduler cannot raise one from
/// its own destructor without risking a panic while the scope is already
/// unwinding.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::atomic::{AtomicUsize, Ordering};
/// use splot_parallel::{
///     AdmissionScheduler, Condition, ThreadCount, WatermarkCell, WorkerPool, ready_task_scope,
/// };
///
/// let rows = WatermarkCell::new();
/// let ran = AtomicUsize::new(0);
/// let scheduler = AdmissionScheduler::new();
/// let pool = WorkerPool::new(ThreadCount::Fixed(2.try_into()?))?;
/// pool.install(|| {
///     ready_task_scope(|scope| {
///         scheduler.submit(
///             scope,
///             1,
///             &[Condition::Watermark(&rows, 4)],
///             Box::new(|_| {
///                 ran.fetch_add(1, Ordering::Relaxed);
///             }),
///         );
///         scheduler.submit(
///             scope,
///             0,
///             &[],
///             Box::new(|_| {
///                 rows.publish(4);
///             }),
///         );
///     })
/// })?;
/// scheduler.finish()?;
/// assert_eq!(ran.load(Ordering::Relaxed), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
pub struct AdmissionScheduler<'job> {
    slots: Mutex<SchedulerSlots<'job>>,
    ready: Arc<ReadyQueue>,
}

impl<'job> AdmissionScheduler<'job> {
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(SchedulerSlots::default()),
            ready: Arc::new(ReadyQueue::default()),
        }
    }

    /// Submits `job` under `order_key`, to run once every entry of `conditions`
    /// holds.
    ///
    /// A job with no unsatisfied conditions is queued and spawned before this
    /// call returns, ahead of any lower-keyed job already queued. Otherwise the
    /// job is stored and each unsatisfied source is asked to notify the job's
    /// token; the notification that clears the last condition queues the job,
    /// and the next drain spawns it. A submission that leaves a condition
    /// outstanding queued nothing and therefore does not drain — see the
    /// module's account of which thread owes each queued entry its drain.
    pub fn submit<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        order_key: u64,
        conditions: &[Condition<'_>],
        job: Job<'job>,
    ) where
        'job: 'scope,
    {
        #[cfg(test)]
        {
            self.ready
                .counters
                .submitted
                .fetch_add(1, Ordering::Relaxed);
            if conditions.is_empty() {
                self.ready
                    .counters
                    .conditionless
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        let token = {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots.store_job(conditions.len() + 1, order_key, job, &self.ready)
        };
        for condition in conditions {
            if !condition.register(Arc::clone(&token) as Arc<dyn AdmissionWaiter>) {
                token.satisfy();
            }
        }
        let queued = token.satisfy_reporting();
        drop(token);
        if queued {
            self.admit_ready(scope);
        }
    }

    /// Spawns every job that is admissible now, returning how many were spawned.
    ///
    /// The drain is iterative: each spawned job re-drains after its body runs,
    /// so a job that admits further work does not nest drains on one stack. No
    /// lock is held across a spawn, which is also why concurrent drains
    /// interleave rather than queue — see the module's ordering guarantee.
    pub fn admit_ready<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>) -> usize
    where
        'job: 'scope,
    {
        let mut spawned = 0;
        while let Some(ready) = self.ready.pop() {
            let Some((job, token)) = self.take_job(ready) else {
                continue;
            };
            scope.spawn(move |scope| {
                self.run_job(scope, job);
                self.recycle_token(token);
            });
            spawned += 1;
        }
        if spawned != 0 {
            notify_installed_pool_progress();
        }
        #[cfg(test)]
        {
            self.ready.counters.drains.fetch_add(1, Ordering::Relaxed);
            if spawned == 0 {
                self.ready
                    .counters
                    .empty_drains
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        spawned
    }

    fn run_job<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>, mut job: Job<'job>)
    where
        'job: 'scope,
    {
        let continuations = ContinuationSlot::default();
        let admit = ScopeAdmit {
            scheduler: self,
            scope,
            continuations: &continuations,
        };
        let mut continued = 0;
        loop {
            job(&admit);
            self.admit_ready(scope);
            let Some(next) = continuations.take() else {
                break;
            };
            if continued >= CONTINUATION_BUDGET {
                #[cfg(test)]
                self.ready
                    .counters
                    .continuation_fallbacks
                    .fetch_add(1, Ordering::Relaxed);
                self.submit(scope, next.order_key, &[], next.job);
                break;
            }
            continued += 1;
            #[cfg(test)]
            self.ready
                .counters
                .continued
                .fetch_add(1, Ordering::Relaxed);
            job = next.job;
        }
        #[cfg(test)]
        self.ready
            .counters
            .maximum_continuation_burst
            .fetch_max(continued, Ordering::Relaxed);
    }

    /// Reports jobs that were never admitted and releases them.
    ///
    /// Call it after the task scope has joined: at that point every admitted job
    /// has run, so anything still held is a job whose conditions never all held
    /// — a bug in the caller's dependency graph (a producer that failed without
    /// publishing [`WatermarkCell::FAILED`], or a cycle). The stranded jobs are
    /// dropped, so a second call reports success.
    ///
    /// # Errors
    /// Returns [`ParallelError::JobsNeverAdmitted`] when at least one submitted
    /// job was still waiting on a condition.
    pub fn finish(&self) -> Result<(), ParallelError> {
        let stranded = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_stranded();
        let Some(lowest_order_key) = stranded.iter().map(|(key, _)| *key).min() else {
            return Ok(());
        };
        Err(ParallelError::JobsNeverAdmitted {
            count: stranded.len(),
            lowest_order_key,
        })
    }

    /// Takes one generation's job, or `None` when it is stale or another drain won it.
    fn take_job(&self, ready: ReadyEntry) -> Option<(Job<'job>, Arc<AdmissionWaiterHandle>)> {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_job(ready)
    }

    fn recycle_token(&self, waiter: Arc<AdmissionWaiterHandle>) {
        if let Ok(waiter) = Arc::try_unwrap(waiter)
            && Arc::strong_count(&waiter.token) == 1
        {
            self.slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .tokens
                .push(waiter.token);
        }
    }
}

/// The [`Admit`] handle handed to a running job: the scheduler plus the scope
/// the job itself was spawned into.
struct ScopeAdmit<'a, 'handle, 'scope, 'job> {
    scheduler: &'scope AdmissionScheduler<'job>,
    scope: &'a TaskScope<'handle, 'scope>,
    continuations: &'a ContinuationSlot<'job>,
}

impl<'job> Admit<'job> for ScopeAdmit<'_, '_, '_, 'job> {
    fn admit_ready(&self) -> usize {
        self.scheduler.admit_ready(self.scope)
    }

    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job>) {
        self.scheduler
            .submit(self.scope, order_key, conditions, job);
    }

    fn spawn_ready(&self, job: Job<'job>) {
        let scheduler = self.scheduler;
        #[cfg(test)]
        scheduler
            .ready
            .counters
            .direct
            .fetch_add(1, Ordering::Relaxed);
        self.scope.spawn(move |scope| {
            scheduler.run_job(scope, job);
        });
    }

    fn continue_ready(&self, order_key: u64, job: Job<'job>) {
        let next = OrderedJob { order_key, job };
        if let Err(next) = self.continuations.put(next) {
            #[cfg(test)]
            self.scheduler
                .ready
                .counters
                .continuation_fallbacks
                .fetch_add(1, Ordering::Relaxed);
            self.scheduler
                .submit(self.scope, next.order_key, &[], next.job);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::pool::{WorkerPool, ready_task_scope};
    use crate::thread_count::ThreadCount;
    use core::num::NonZeroUsize;
    use std::cell::Cell;
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;

    std::thread_local! {
        static CONTINUATION_TEST_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    struct ContinuationDepthGuard;

    impl Drop for ContinuationDepthGuard {
        fn drop(&mut self) {
            CONTINUATION_TEST_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }

    fn pool(threads: usize) -> WorkerPool {
        WorkerPool::new(ThreadCount::Fixed(NonZeroUsize::new(threads).unwrap())).unwrap()
    }

    #[test]
    fn token_state_is_reused_with_a_new_slot_generation_without_waiter_aba() {
        let scheduler = AdmissionScheduler::new();
        let token = scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_token(3, SlotGeneration(4), &scheduler.ready);
        let address = Arc::as_ptr(&token);
        let waiter = Arc::new(AdmissionWaiterHandle {
            entry: ReadyEntry {
                order_key: 7,
                submission_order: 0,
                index: 11,
                generation: SlotGeneration(4),
            },
            token,
        });
        let stale = Arc::downgrade(&waiter);
        scheduler.recycle_token(waiter);
        assert!(
            stale.upgrade().is_none(),
            "the job-specific handle dies even while its weak allocation remains"
        );

        let token = scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_token(2, SlotGeneration(9), &scheduler.ready);
        assert_eq!(Arc::as_ptr(&token), address);
        assert_eq!(token.pending.load(Ordering::Acquire), 2);
        assert_eq!(token.generation.load(Ordering::Relaxed), 9);
    }

    /// Every tally in declaration order, read after the scope has joined so all
    /// of the run's drains are already counted. Tests take the window they are
    /// about: `[..4]` is the drain policy, `[4..]` the submission paths.
    fn counters(scheduler: &AdmissionScheduler<'_>) -> [usize; 7] {
        let counters = &scheduler.ready.counters;
        [
            counters.satisfies.load(Ordering::Relaxed),
            counters.transitions.load(Ordering::Relaxed),
            counters.drains.load(Ordering::Relaxed),
            counters.empty_drains.load(Ordering::Relaxed),
            counters.submitted.load(Ordering::Relaxed),
            counters.conditionless.load(Ordering::Relaxed),
            counters.direct.load(Ordering::Relaxed),
        ]
    }

    fn continue_chain<'job>(admit: &dyn Admit<'job>, visits: &'job [AtomicUsize], index: usize) {
        visits[index].fetch_add(1, Ordering::AcqRel);
        if index + 1 < visits.len() {
            admit.continue_ready(
                index as u64 + 1,
                Box::new(move |admit| continue_chain(admit, visits, index + 1)),
            );
        }
    }

    fn run_continuation_chain(jobs: usize) -> ([usize; 3], Vec<usize>) {
        let visits: Vec<_> = (0..jobs).map(|_| AtomicUsize::new(0)).collect();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| continue_chain(admit, &visits, 0)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        let counters = &scheduler.ready.counters;
        let metrics = [
            counters.continued.load(Ordering::Relaxed),
            counters.continuation_fallbacks.load(Ordering::Relaxed),
            counters.maximum_continuation_burst.load(Ordering::Relaxed),
        ];
        let visits = visits
            .iter()
            .map(|count| count.load(Ordering::Acquire))
            .collect();
        (metrics, visits)
    }

    #[test]
    fn a_one_link_continuation_runs_once_inline() {
        let (metrics, visits) = run_continuation_chain(2);
        assert_eq!(visits, [1, 1]);
        assert_eq!(metrics, [1, 0, 1]);
    }

    #[test]
    fn a_long_continuation_chain_is_iterative_and_exactly_once() {
        const JOBS: usize = 100_000;
        let (metrics, visits) = run_continuation_chain(JOBS);
        assert!(visits.iter().all(|visits| *visits == 1));
        assert_eq!(metrics[2], CONTINUATION_BUDGET);
        assert!(metrics[1] > 0);
    }

    #[test]
    fn the_continuation_budget_falls_back_on_the_next_link() {
        let (within, _) = run_continuation_chain(CONTINUATION_BUDGET + 1);
        assert_eq!(within, [CONTINUATION_BUDGET, 0, CONTINUATION_BUDGET]);

        let (over, _) = run_continuation_chain(CONTINUATION_BUDGET + 2);
        assert_eq!(over, [CONTINUATION_BUDGET, 1, CONTINUATION_BUDGET]);
    }

    #[test]
    fn a_failure_in_the_middle_settles_without_running_a_successor() {
        fn link<'job>(
            admit: &dyn Admit<'job>,
            visits: &'job [AtomicUsize],
            failed: &'job CompletionCell<usize>,
            index: usize,
        ) {
            visits[index].fetch_add(1, Ordering::AcqRel);
            if index == 2 {
                let _ = failed.set(index);
            } else if index + 1 < visits.len() {
                admit.continue_ready(
                    index as u64 + 1,
                    Box::new(move |admit| link(admit, visits, failed, index + 1)),
                );
            }
        }
        let visits: Vec<_> = (0..5).map(|_| AtomicUsize::new(0)).collect();
        let failed = CompletionCell::new();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| link(admit, &visits, &failed, 0)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(failed.get(), Some(&2));
        assert_eq!(
            visits
                .iter()
                .map(|visits| visits.load(Ordering::Acquire))
                .collect::<Vec<_>>(),
            [1, 1, 1, 0, 0]
        );
    }

    #[test]
    fn a_panicking_inline_continuation_unwinds_through_the_scope() {
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(|admit| {
                            admit.continue_ready(1, Box::new(|_| panic!("continuation")));
                        }),
                    );
                })
            })
        }))
        .expect_err("the scope must not swallow an inline continuation's panic");
        assert_eq!(
            panicked.downcast_ref::<&str>().copied(),
            Some("continuation")
        );
        scheduler.finish().unwrap();
    }

    #[test]
    fn independent_work_can_run_during_an_inline_chain() {
        let rendezvous = Barrier::new(2);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                let rendezvous = &rendezvous;
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(move |admit| {
                        let independent = rendezvous;
                        admit.spawn_ready(Box::new(move |_| {
                            independent.wait();
                        }));
                        admit.continue_ready(
                            1,
                            Box::new(move |_| {
                                rendezvous.wait();
                            }),
                        );
                    }),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
    }

    #[test]
    fn continuation_depth_does_not_grow_with_chain_length() {
        fn link<'job>(admit: &dyn Admit<'job>, maximum: &'job AtomicUsize, remaining: usize) {
            let depth = CONTINUATION_TEST_DEPTH.with(|depth| {
                let next = depth.get() + 1;
                depth.set(next);
                next
            });
            let _active = ContinuationDepthGuard;
            maximum.fetch_max(depth, Ordering::AcqRel);
            if remaining > 1 {
                admit.continue_ready(
                    remaining as u64,
                    Box::new(move |admit| link(admit, maximum, remaining - 1)),
                );
            }
        }

        let maximum = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| link(admit, &maximum, 10_000)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(maximum.load(Ordering::Acquire), 1);
    }

    /// Submission and slot-storage metrics, read after the scope has joined.
    fn slot_metrics(scheduler: &AdmissionScheduler<'_>) -> [usize; 4] {
        let submitted = scheduler.ready.counters.submitted.load(Ordering::Relaxed);
        let slots = scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        [
            submitted,
            slots.peak_occupied,
            slots.entries.len(),
            slots.reuses,
        ]
    }

    fn submit_chain<'job>(admit: &dyn Admit<'job>, ran: &'job AtomicUsize, remaining: usize) {
        ran.fetch_add(1, Ordering::AcqRel);
        if remaining > 1 {
            admit.submit(
                0,
                &[],
                Box::new(move |admit| submit_chain(admit, ran, remaining - 1)),
            );
        }
    }

    #[test]
    fn one_hundred_thousand_short_jobs_track_peak_live_slots() {
        const JOBS: usize = 100_000;
        let ran = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(4);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| submit_chain(admit, &ran, JOBS)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();

        assert_eq!(ran.load(Ordering::Acquire), JOBS);
        assert_eq!(slot_metrics(&scheduler), [JOBS, 1, 1, JOBS - 1]);
    }

    fn store_test_job(
        scheduler: &AdmissionScheduler<'static>,
        pending: usize,
        order_key: u64,
    ) -> Arc<AdmissionWaiterHandle> {
        scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store_job(pending, order_key, Box::new(|_| {}), &scheduler.ready)
    }

    #[test]
    fn a_stale_ready_entry_is_rejected_after_immediate_slot_reuse() {
        let scheduler = AdmissionScheduler::new();
        let first = store_test_job(&scheduler, 1, 3);
        assert!(first.satisfy_reporting());
        let stale = scheduler.ready.pop().unwrap();
        let (job, first) = scheduler.take_job(stale).unwrap();
        drop(job);
        scheduler.recycle_token(first);

        let second = store_test_job(&scheduler, 1, 5);
        assert_eq!(second.entry.index, stale.index);
        assert_ne!(second.entry.generation, stale.generation);
        scheduler.ready.push(stale);
        assert!(scheduler.take_job(scheduler.ready.pop().unwrap()).is_none());

        assert!(second.satisfy_reporting());
        let current = scheduler.ready.pop().unwrap();
        assert_eq!(current.generation, second.entry.generation);
        assert!(scheduler.take_job(current).is_some());
    }

    #[test]
    fn an_old_waiter_notification_cannot_satisfy_a_reused_slot() {
        let scheduler = AdmissionScheduler::new();
        let first = store_test_job(&scheduler, 1, 3);
        assert!(first.satisfy_reporting());
        let first_ready = scheduler.ready.pop().unwrap();
        let (job, first) = scheduler.take_job(first_ready).unwrap();
        drop(job);

        let second = store_test_job(&scheduler, 2, 5);
        assert_eq!(second.entry.index, first_ready.index);
        assert_ne!(second.entry.generation, first.entry.generation);
        assert!(!second.satisfy_reporting(), "the closing count remains");
        assert!(
            !first.satisfy_reporting(),
            "the old token was already closed"
        );
        assert!(scheduler.ready.pop().is_none());
        assert!(second.satisfy_reporting());
        assert_eq!(
            scheduler.ready.pop().unwrap().generation,
            second.entry.generation
        );
    }

    #[test]
    fn a_generation_at_max_is_retired_instead_of_wrapping() {
        let scheduler = AdmissionScheduler::new();
        let first = store_test_job(&scheduler, 1, 0);
        assert!(first.satisfy_reporting());
        let ready = scheduler.ready.pop().unwrap();
        let (job, first) = scheduler.take_job(ready).unwrap();
        drop(job);
        scheduler.recycle_token(first);
        scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries[ready.index]
            .generation = SlotGeneration(u64::MAX);

        let second = store_test_job(&scheduler, 1, 1);
        assert_ne!(second.entry.index, ready.index);
        assert_eq!(second.entry.generation, SlotGeneration::INITIAL);
        let slots = scheduler
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(slots.entries.len(), 2);
        assert_eq!(
            slots.entries[ready.index].generation,
            SlotGeneration(u64::MAX)
        );
    }

    #[test]
    fn exactly_one_concurrent_satisfy_reports_the_transition() {
        for _ in 0..256 {
            let ready = Arc::new(ReadyQueue::default());
            let token = Arc::new(AdmissionToken {
                pending: AtomicUsize::new(4),
                generation: AtomicU64::new(2),
                ready: Arc::clone(&ready),
            });
            let waiter = Arc::new(AdmissionWaiterHandle {
                entry: ReadyEntry {
                    order_key: 7,
                    submission_order: 9,
                    index: 3,
                    generation: SlotGeneration(2),
                },
                token,
            });
            let reported: usize = std::thread::scope(|threads| {
                let handles: Vec<_> = (0..4)
                    .map(|_| {
                        let waiter = Arc::clone(&waiter);
                        threads.spawn(move || usize::from(waiter.satisfy_reporting()))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .sum()
            });
            assert_eq!(reported, 1, "only the closing satisfy queues the job");
            assert_eq!(
                ready.pop(),
                Some(ReadyEntry {
                    order_key: 7,
                    submission_order: 9,
                    index: 3,
                    generation: SlotGeneration(2),
                })
            );
            assert_eq!(ready.pop(), None, "the job is queued exactly once");
        }
    }

    /// Records the order in which jobs ran.
    #[derive(Debug, Default)]
    struct RunLog(Mutex<Vec<u64>>);

    impl RunLog {
        fn record(&self, key: u64) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(key);
        }

        fn keys(&self) -> Vec<u64> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[test]
    fn an_unconditional_job_is_spawned_at_submit() {
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(scope, 0, &[], Box::new(|_| log.record(0)));
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
        assert_eq!(
            counters(&scheduler)[..4],
            [1, 1, 2, 1],
            "submit drains the job it queued; the job's post-body drain finds nothing"
        );
    }

    #[test]
    fn a_job_with_one_already_satisfied_condition_is_spawned_at_submit() {
        let rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        rows.publish(3);
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::Watermark(&rows, 3)],
                    Box::new(|_| log.record(0)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
        assert_eq!(counters(&scheduler)[..4], [2, 1, 2, 1]);
    }

    #[test]
    fn a_job_whose_conditions_already_hold_is_spawned_at_submit() {
        let rows = WatermarkCell::new();
        let done = CompletionCell::completed(7u32);
        let settled = CompletionCell::completed(());
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        rows.publish(3);
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[
                        Condition::Watermark(&rows, 3),
                        Condition::Completion(&done),
                        Condition::Completion(&settled),
                    ],
                    Box::new(|_| log.record(0)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
        assert_eq!(counters(&scheduler)[..4], [4, 1, 2, 1]);
    }

    #[test]
    fn a_submission_with_an_outstanding_condition_does_not_drain() {
        let rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::Watermark(&rows, 1)],
                    Box::new(|_| log.record(0)),
                );
                assert_eq!(
                    counters(&scheduler)[..4],
                    [1, 0, 0, 0],
                    "nothing was queued, so nothing needed spawning"
                );
                rows.publish(1);
                assert_eq!(scheduler.admit_ready(scope), 1);
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
    }

    #[test]
    fn a_queued_unrelated_job_survives_a_submission_that_queues_nothing() {
        let ready_rows = WatermarkCell::new();
        let blocked_rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::Watermark(&ready_rows, 1)],
                    Box::new(|_| log.record(0)),
                );
                ready_rows.publish(1);
                scheduler.submit(
                    scope,
                    1,
                    &[Condition::Watermark(&blocked_rows, 1)],
                    Box::new(|_| log.record(1)),
                );
                assert!(
                    log.keys().is_empty(),
                    "the blocked submission spawned nothing"
                );
                assert_eq!(
                    scheduler.admit_ready(scope),
                    1,
                    "the publisher's own drain still finds the queued job"
                );
                blocked_rows.publish(1);
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0, 1]);
    }

    #[test]
    fn a_condition_settled_concurrently_with_submission_admits_exactly_once() {
        for threads in 1..=4 {
            for _ in 0..64 {
                let done = CompletionCell::new();
                let ran = AtomicUsize::new(0);
                let scheduler = AdmissionScheduler::new();
                let pool = pool(threads);
                pool.install(|| {
                    ready_task_scope(|scope| {
                        scheduler.submit(
                            scope,
                            0,
                            &[],
                            Box::new(|_| {
                                let _ = done.set(());
                            }),
                        );
                        scheduler.submit(
                            scope,
                            1,
                            &[Condition::Completion(&done)],
                            Box::new(|_| {
                                ran.fetch_add(1, Ordering::AcqRel);
                            }),
                        );
                    })
                })
                .unwrap();
                scheduler.finish().unwrap();
                assert_eq!(ran.load(Ordering::Acquire), 1, "at {threads} thread(s)");
            }
        }
    }

    #[test]
    fn a_chain_submitted_before_its_producers_strands_nothing() {
        const LINKS: usize = 32;
        for threads in 1..=4 {
            let gates: Vec<CompletionCell<()>> =
                (0..LINKS).map(|_| CompletionCell::new()).collect();
            let ran = AtomicUsize::new(0);
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    let gates = &gates;
                    let ran = &ran;
                    for index in (0..LINKS).rev() {
                        let conditions: Vec<Condition<'_>> = index
                            .checked_sub(1)
                            .map(|previous| vec![Condition::Completion(&gates[previous])])
                            .unwrap_or_default();
                        scheduler.submit(
                            scope,
                            index as u64,
                            &conditions,
                            Box::new(move |_| {
                                ran.fetch_add(1, Ordering::AcqRel);
                                let _ = gates[index].set(());
                            }),
                        );
                    }
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(ran.load(Ordering::Acquire), LINKS, "at {threads} thread(s)");
        }
    }

    #[test]
    fn a_multi_condition_job_waits_for_its_last_condition() {
        let rows = WatermarkCell::new();
        let done = CompletionCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(1);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    2,
                    &[Condition::Watermark(&rows, 2), Condition::Completion(&done)],
                    Box::new(|_| log.record(2)),
                );
                assert!(log.keys().is_empty());
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| {
                        rows.publish(2);
                        assert_eq!(admit.admit_ready(), 0, "one condition still missing");
                        let _ = done.set(());
                        assert_eq!(admit.admit_ready(), 1, "the last condition admits it");
                        log.record(0);
                    }),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0, 2]);
    }

    #[test]
    fn a_drain_spawns_the_lowest_order_key_first() {
        let rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(1);
        pool.install(|| {
            ready_task_scope(|scope| {
                let log = &log;
                for key in [40u64, 10, 30, 20] {
                    scheduler.submit(
                        scope,
                        key,
                        &[Condition::Watermark(&rows, 1)],
                        Box::new(move |_| log.record(key)),
                    );
                }
                rows.publish(1);
                assert_eq!(scheduler.admit_ready(scope), 4);
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![10, 20, 30, 40]);
    }

    #[test]
    fn reused_indices_do_not_change_submission_order_ties() {
        const JOBS: usize = 32;
        let first_gate = WatermarkCell::new();
        let second_gate = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(1);
        pool.install(|| {
            ready_task_scope(|scope| {
                let log = &log;
                for _ in 0..JOBS {
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::Watermark(&first_gate, 1)],
                        Box::new(|_| {}),
                    );
                }
                first_gate.publish(1);
                assert_eq!(scheduler.admit_ready(scope), JOBS);

                for index in 0..JOBS {
                    scheduler.submit(
                        scope,
                        7,
                        &[Condition::Watermark(&second_gate, 1)],
                        Box::new(move |_| log.record(index as u64)),
                    );
                }
                second_gate.publish(1);
                assert_eq!(scheduler.admit_ready(scope), JOBS);
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), (0..JOBS as u64).collect::<Vec<_>>());
    }

    #[test]
    fn concurrent_drains_take_each_slot_generation_once() {
        const JOBS: usize = 256;
        const DRAINS: usize = 8;
        let gate = WatermarkCell::new();
        let runs: Vec<AtomicUsize> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(4);
        pool.install(|| {
            ready_task_scope(|scope| {
                for (index, ran) in runs.iter().enumerate() {
                    scheduler.submit(
                        scope,
                        index as u64,
                        &[Condition::Watermark(&gate, 1)],
                        Box::new(move |_| {
                            ran.fetch_add(1, Ordering::AcqRel);
                        }),
                    );
                }
                gate.publish(1);
                let drained: usize = std::thread::scope(|threads| {
                    let drains: Vec<_> = (0..DRAINS)
                        .map(|_| threads.spawn(|| scheduler.admit_ready(scope)))
                        .collect();
                    drains.into_iter().map(|drain| drain.join().unwrap()).sum()
                });
                assert!(
                    (1..=JOBS).contains(&drained),
                    "post-body drains may take entries after the concurrent callers spawn"
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert!(runs.iter().all(|ran| ran.load(Ordering::Acquire) == 1));
        assert_eq!(slot_metrics(&scheduler), [JOBS, JOBS, JOBS, 0]);
    }

    #[test]
    fn finish_reports_jobs_that_were_never_admitted() {
        let rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                let log = &log;
                for key in [5u64, 3] {
                    scheduler.submit(
                        scope,
                        key,
                        &[Condition::Watermark(&rows, 1)],
                        Box::new(move |_| log.record(key)),
                    );
                }
            })
        })
        .unwrap();
        assert!(log.keys().is_empty());
        assert!(matches!(
            scheduler.finish(),
            Err(ParallelError::JobsNeverAdmitted {
                count: 2,
                lowest_order_key: 3,
            })
        ));
        {
            let slots = scheduler
                .slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            assert!(slots.occupied_indices.is_empty());
            assert_eq!(slots.entries.len(), 2);
            assert_eq!(slots.free.len(), 2);
        }
        assert!(
            scheduler.finish().is_ok(),
            "stranded jobs are released once"
        );
    }

    #[test]
    fn a_failed_producer_admits_its_dependents() {
        let rows = WatermarkCell::new();
        let observed = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    1,
                    &[Condition::Watermark(&rows, 64)],
                    Box::new(|_| observed.store(rows.current(), Ordering::Release)),
                );
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| {
                        rows.publish(WatermarkCell::FAILED);
                        admit.admit_ready();
                    }),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(
            observed.load(Ordering::Acquire),
            WatermarkCell::FAILED,
            "the dependent runs and can fail closed on what it reads"
        );
    }

    #[test]
    fn a_driver_publish_admits_on_the_next_driver_drain() {
        let rows = WatermarkCell::new();
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::Watermark(&rows, 1)],
                    Box::new(|_| log.record(0)),
                );
                rows.publish(1);
                assert_eq!(scheduler.admit_ready(scope), 1);
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
    }

    #[test]
    fn a_publisher_admits_dependents_between_its_own_sub_jobs() {
        for threads in 2..=4 {
            let rows = WatermarkCell::new();
            let ran = AtomicUsize::new(0);
            let observed = AtomicUsize::new(0);
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        10,
                        &[Condition::Watermark(&rows, 2)],
                        Box::new(|_| {
                            observed.store(rows.current(), Ordering::Release);
                            ran.fetch_add(1, Ordering::AcqRel);
                        }),
                    );
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(|admit| {
                            admit.submit(
                                1,
                                &[],
                                Box::new(|_| {
                                    ran.fetch_add(1, Ordering::AcqRel);
                                }),
                            );
                            rows.publish(2);
                            admit.admit_ready();
                            admit.submit(
                                2,
                                &[],
                                Box::new(|_| {
                                    ran.fetch_add(1, Ordering::AcqRel);
                                }),
                            );
                            ran.fetch_add(1, Ordering::AcqRel);
                        }),
                    );
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(ran.load(Ordering::Acquire), 4, "at {threads} thread(s)");
            assert!(
                observed.load(Ordering::Acquire) >= 2,
                "at {threads} thread(s)"
            );
        }
    }

    #[test]
    fn a_tail_publish_needs_no_drain_of_its_own() {
        for threads in 1..=4 {
            let rows = WatermarkCell::new();
            let done = CompletionCell::new();
            let ran = AtomicUsize::new(0);
            let nested = AtomicUsize::new(0);
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        2,
                        &[Condition::Watermark(&rows, 2), Condition::Completion(&done)],
                        Box::new(|admit| {
                            ran.fetch_add(1, Ordering::AcqRel);
                            admit.submit(
                                3,
                                &[],
                                Box::new(|_| {
                                    nested.fetch_add(1, Ordering::AcqRel);
                                }),
                            );
                        }),
                    );
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(|_| {
                            rows.publish(2);
                            let _ = done.set(());
                        }),
                    );
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(ran.load(Ordering::Acquire), 1, "at {threads} thread(s)");
            assert_eq!(nested.load(Ordering::Acquire), 1, "at {threads} thread(s)");
        }
    }

    #[test]
    fn a_tail_failure_settlement_needs_no_drain_of_its_own() {
        for threads in 1..=4 {
            let rows = WatermarkCell::new();
            let observed = AtomicUsize::new(0);
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        1,
                        &[Condition::Watermark(&rows, 64)],
                        Box::new(|_| observed.store(rows.current(), Ordering::Release)),
                    );
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(|_| {
                            rows.publish(WatermarkCell::FAILED);
                        }),
                    );
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(
                observed.load(Ordering::Acquire),
                WatermarkCell::FAILED,
                "at {threads} thread(s)"
            );
        }
    }

    /// Builds the same fan-out twice, once per submission path, and reports the
    /// tallies each cost.
    fn fan_out(fast: bool) -> [usize; 7] {
        const JOBS: usize = 16;
        let ran = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| {
                        for _ in 0..JOBS {
                            let job: Job<'_> = Box::new(|_| {
                                ran.fetch_add(1, Ordering::AcqRel);
                            });
                            if fast {
                                admit.spawn_ready(job);
                            } else {
                                admit.submit(1, &[], job);
                            }
                        }
                    }),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(ran.load(Ordering::Acquire), JOBS);
        counters(&scheduler)
    }

    #[test]
    fn a_directly_spawned_job_costs_no_slot_and_no_heap_round_trip() {
        let scheduled = fan_out(false);
        let direct = fan_out(true);

        assert_eq!(
            scheduled[4..],
            [17, 17, 0],
            "submitting the fan-out stores every job in a scheduler slot"
        );
        assert_eq!(
            direct[4..],
            [1, 1, 16],
            "spawning it ready stores only the entry job"
        );
        assert_eq!(
            scheduled[1] - direct[1],
            16,
            "each fast-path job skips one ready-heap push, and so one pop"
        );
    }

    #[test]
    fn a_direct_job_submits_and_admits_a_gated_successor() {
        for threads in 1..=4 {
            let gate = CompletionCell::new();
            let log = RunLog::default();
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    let (log, gate) = (&log, &gate);
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(move |admit| {
                            admit.spawn_ready(Box::new(move |admit| {
                                admit.submit(
                                    2,
                                    &[Condition::Completion(gate)],
                                    Box::new(move |_| log.record(2)),
                                );
                                log.record(1);
                                let _ = gate.set(());
                            }));
                        }),
                    );
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(log.keys(), vec![1, 2], "at {threads} thread(s)");
        }
    }

    /// One link of a chain of directly spawned jobs, each spawning the next.
    fn nest<'job>(admit: &dyn Admit<'job>, visits: &'job AtomicUsize, remaining: usize) {
        visits.fetch_add(1, Ordering::AcqRel);
        if remaining > 0 {
            admit.spawn_ready(Box::new(move |admit| nest(admit, visits, remaining - 1)));
        }
    }

    #[test]
    fn nested_direct_jobs_each_run_once() {
        const DEPTH: usize = 8;
        for threads in 1..=4 {
            let visits = AtomicUsize::new(0);
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(scope, 0, &[], Box::new(|admit| nest(admit, &visits, DEPTH)));
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(
                visits.load(Ordering::Acquire),
                DEPTH + 1,
                "at {threads} thread(s)"
            );
            assert_eq!(counters(&scheduler)[4..], [1, 1, DEPTH]);
        }
    }

    #[test]
    fn a_direct_and_a_gated_job_share_a_scope_and_each_run_once() {
        const JOBS: usize = 24;
        for threads in 1..=4 {
            let gates: Vec<CompletionCell<()>> = (0..JOBS).map(|_| CompletionCell::new()).collect();
            let direct: Vec<AtomicUsize> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
            let gated: Vec<AtomicUsize> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
            let scheduler = AdmissionScheduler::new();
            let pool = pool(threads);
            pool.install(|| {
                ready_task_scope(|scope| {
                    let (gates, direct, gated) = (&gates, &direct, &gated);
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(move |admit| {
                            for index in 0..JOBS {
                                admit.submit(
                                    (index as u64) * 2 + 2,
                                    &[Condition::Completion(&gates[index])],
                                    Box::new(move |_| {
                                        gated[index].fetch_add(1, Ordering::AcqRel);
                                    }),
                                );
                            }
                            for index in 0..JOBS {
                                admit.spawn_ready(Box::new(move |_| {
                                    direct[index].fetch_add(1, Ordering::AcqRel);
                                    let _ = gates[index].set(());
                                }));
                            }
                        }),
                    );
                })
            })
            .unwrap();
            scheduler.finish().unwrap();
            assert_eq!(counters(&scheduler)[4..], [JOBS + 1, 1, JOBS]);
            for index in 0..JOBS {
                assert_eq!(
                    (
                        direct[index].load(Ordering::Acquire),
                        gated[index].load(Ordering::Acquire),
                    ),
                    (1, 1),
                    "job {index} at {threads} thread(s)"
                );
            }
        }
    }

    #[test]
    fn a_scope_completes_only_after_its_direct_jobs_have_run() {
        const JOBS: usize = 8;
        let ran = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| {
                        for _ in 0..JOBS {
                            admit.spawn_ready(Box::new(|_| {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                                ran.fetch_add(1, Ordering::AcqRel);
                            }));
                        }
                    }),
                );
            })
        })
        .unwrap();
        assert_eq!(
            ran.load(Ordering::Acquire),
            JOBS,
            "the scope joined every direct job before returning"
        );
        scheduler.finish().unwrap();
    }

    #[test]
    fn a_panicking_direct_job_unwinds_through_the_scope() {
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[],
                        Box::new(|admit| {
                            admit.spawn_ready(Box::new(|_| panic!("direct job")));
                        }),
                    );
                })
            })
        }))
        .expect_err("the scope must not swallow a direct job's panic");
        assert_eq!(panicked.downcast_ref::<&str>().copied(), Some("direct job"));
        scheduler.finish().unwrap();
    }

    #[test]
    fn a_panicking_scheduled_job_does_not_corrupt_the_free_list() {
        let ran = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        let pool = pool(2);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(scope, 0, &[], Box::new(|_| panic!("scheduled job")));
                })
            })
        }));
        assert!(panicked.is_err());
        assert_eq!(slot_metrics(&scheduler), [1, 1, 1, 0]);

        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    1,
                    &[],
                    Box::new(|_| {
                        ran.fetch_add(1, Ordering::AcqRel);
                    }),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(ran.load(Ordering::Acquire), 1);
        assert_eq!(slot_metrics(&scheduler), [2, 1, 1, 1]);
    }

    /// A tiny deterministic linear congruential generator: the property test
    /// must replay exactly, and the crate has no random-number dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() as usize) % bound
        }
    }

    /// One generated job: the sources it waits on and the watermarks it drives.
    struct JobSpec {
        waits: Vec<(usize, usize)>,
        completions: Vec<usize>,
        publishes: Vec<(usize, usize)>,
        sets: Vec<usize>,
    }

    /// Builds an acyclic random dependency graph: a source is driven by exactly
    /// one job, and a job may only wait on sources driven by an earlier job.
    fn generate_specs(rng: &mut Lcg, jobs: usize, watermarks: usize, slots: usize) -> Vec<JobSpec> {
        let mut specs: Vec<JobSpec> = (0..jobs)
            .map(|_| JobSpec {
                waits: Vec::new(),
                completions: Vec::new(),
                publishes: Vec::new(),
                sets: Vec::new(),
            })
            .collect();
        let mut watermark_owner = vec![0usize; watermarks];
        let mut slot_owner = vec![0usize; slots];
        for (source, owner) in watermark_owner.iter_mut().enumerate() {
            *owner = 1 + rng.below(jobs - 1);
            let rows = 1 + rng.below(3);
            specs[*owner].publishes.push((source, rows));
        }
        for (slot, owner) in slot_owner.iter_mut().enumerate() {
            *owner = 1 + rng.below(jobs - 1);
            specs[*owner].sets.push(slot);
        }
        for job in 1..jobs {
            for _ in 0..=rng.below(3) {
                let source = rng.below(watermarks);
                if watermark_owner[source] < job {
                    let rows = specs[watermark_owner[source]]
                        .publishes
                        .iter()
                        .find(|(candidate, _)| *candidate == source)
                        .map_or(1, |(_, rows)| *rows);
                    specs[job].waits.push((source, 1 + rng.below(rows)));
                }
                let slot = rng.below(slots);
                if slot_owner[slot] < job {
                    specs[job].completions.push(slot);
                }
            }
        }
        specs
    }

    #[test]
    fn random_dependency_graphs_run_every_job_exactly_once() {
        const JOBS: usize = 16;
        const WATERMARKS: usize = 5;
        const SLOTS: usize = 3;
        const ITERATIONS: u64 = 2048;

        for threads in [1usize, 2, 4] {
            let pool = pool(threads);
            let mut dependencies = 0usize;
            for seed in 0..ITERATIONS {
                let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ threads as u64);
                let specs = generate_specs(&mut rng, JOBS, WATERMARKS, SLOTS);
                let watermarks: Vec<WatermarkCell> =
                    (0..WATERMARKS).map(|_| WatermarkCell::new()).collect();
                let slots: Vec<CompletionCell<()>> =
                    (0..SLOTS).map(|_| CompletionCell::new()).collect();
                let runs: Vec<AtomicUsize> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
                let violations = AtomicUsize::new(0);
                dependencies += specs
                    .iter()
                    .map(|spec| spec.waits.len() + spec.completions.len())
                    .sum::<usize>();
                let scheduler = AdmissionScheduler::new();

                let watermarks = &watermarks;
                let slots = &slots;
                let runs = &runs;
                let violations = &violations;
                pool.install(|| {
                    ready_task_scope(|scope| {
                        for (index, spec) in specs.iter().enumerate() {
                            let mut conditions: Vec<Condition<'_>> = spec
                                .waits
                                .iter()
                                .map(|(source, rows)| {
                                    Condition::Watermark(&watermarks[*source], *rows)
                                })
                                .collect();
                            conditions.extend(
                                spec.completions
                                    .iter()
                                    .map(|slot| Condition::Completion(&slots[*slot])),
                            );
                            scheduler.submit(
                                scope,
                                index as u64,
                                &conditions,
                                Box::new(move |admit| {
                                    for (source, rows) in &spec.waits {
                                        if watermarks[*source].current() < *rows {
                                            violations.fetch_add(1, Ordering::AcqRel);
                                        }
                                    }
                                    for slot in &spec.completions {
                                        if !slots[*slot].is_set() {
                                            violations.fetch_add(1, Ordering::AcqRel);
                                        }
                                    }
                                    runs[index].fetch_add(1, Ordering::AcqRel);
                                    for (source, rows) in &spec.publishes {
                                        for row in 1..=*rows {
                                            watermarks[*source].publish(row);
                                            admit.admit_ready();
                                        }
                                    }
                                    for slot in &spec.sets {
                                        let _ = slots[*slot].set(());
                                        admit.admit_ready();
                                    }
                                }),
                            );
                        }
                    })
                })
                .unwrap();

                assert!(
                    scheduler.finish().is_ok(),
                    "seed {seed} at {threads} thread(s) stranded a job"
                );
                assert_eq!(violations.load(Ordering::Acquire), 0, "seed {seed}");
                for (index, runs) in runs.iter().enumerate() {
                    assert_eq!(
                        runs.load(Ordering::Acquire),
                        1,
                        "job {index} of seed {seed} at {threads} thread(s)"
                    );
                }
            }
            assert!(
                dependencies >= 4 * ITERATIONS as usize,
                "the generated graphs must not be vacuous at {threads} thread(s)"
            );
        }
    }
}
