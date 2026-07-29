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
//! only queues a job; a *drain* spawns it. Drains run at
//! [`AdmissionScheduler::submit`], after every scheduler-spawned job body, and
//! wherever a caller asks for one ([`AdmissionScheduler::admit_ready`], or
//! [`Admit::admit_ready`] from inside a job that wants its dependents started
//! before its own body ends). A driver that publishes between drains must call
//! [`AdmissionScheduler::admit_ready`] itself.
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
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::completion::CompletionCell;
use crate::error::ParallelError;
use crate::pool::TaskScope;
use crate::watermark::WatermarkCell;

/// The scheduler-side token a condition source notifies once its condition
/// holds.
///
/// Sources store tokens as `Arc<dyn AdmissionWaiter>`, so the trait carries no
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
    /// before its own body ends.
    fn admit_ready(&self) -> usize;

    /// Submits a successor under the same rules as
    /// [`AdmissionScheduler::submit`]; an empty condition list spawns it at
    /// once.
    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job>);
}

/// The queue of jobs whose conditions all hold, ordered by `order_key`.
///
/// Shared with the waiter tokens through an `Arc` and free of borrowed state, so
/// a condition source that outlives the scheduler can still push into it
/// harmlessly.
#[derive(Debug, Default)]
struct ReadyQueue {
    entries: Mutex<BinaryHeap<Reverse<(u64, usize)>>>,
}

impl ReadyQueue {
    /// Queues the slot at `index` for the next drain.
    fn push(&self, order_key: u64, index: usize) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Reverse((order_key, index)));
    }

    /// Takes the queued slot with the lowest `order_key`.
    fn pop(&self) -> Option<usize> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .map(|Reverse((_, index))| index)
    }
}

/// One job's outstanding-condition count.
///
/// Counting down instead of up means the thread that satisfies the last
/// condition is the one that queues the job, with no lock between the sources.
#[derive(Debug)]
struct AdmissionToken {
    pending: AtomicUsize,
    order_key: u64,
    index: usize,
    ready: Arc<ReadyQueue>,
}

impl AdmissionWaiter for AdmissionToken {
    fn satisfy(&self) {
        let mut pending = self.pending.load(Ordering::Acquire);
        while pending > 0 {
            match self.pending.compare_exchange_weak(
                pending,
                pending - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    if pending == 1 {
                        self.ready.push(self.order_key, self.index);
                    }
                    return;
                }
                Err(observed) => pending = observed,
            }
        }
    }
}

/// One submitted job and the key it is admitted under.
struct Slot<'job> {
    order_key: u64,
    job: Option<Job<'job>>,
}

/// A dependency-ordered admission scheduler over a [`TaskScope`].
///
/// Create it **before** entering [`crate::ready_task_scope`] and after the state
/// its jobs borrow, then share `&scheduler` into the scope: the jobs it holds
/// borrow that state for `'job`, and a job is spawned into the scope only once
/// every condition it named already holds. Memory is one small slot per
/// submitted job plus one token per job, so it is bounded by the jobs the caller
/// submits; use one scheduler per scope.
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
///             Box::new(|admit| {
///                 rows.publish(4);
///                 admit.admit_ready();
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
    slots: Mutex<Vec<Slot<'job>>>,
    ready: Arc<ReadyQueue>,
}

impl<'job> AdmissionScheduler<'job> {
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
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
    /// and the next drain spawns it.
    pub fn submit<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        order_key: u64,
        conditions: &[Condition<'_>],
        job: Job<'job>,
    ) where
        'job: 'scope,
    {
        let index = {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots.push(Slot {
                order_key,
                job: Some(job),
            });
            slots.len() - 1
        };
        let token = Arc::new(AdmissionToken {
            pending: AtomicUsize::new(conditions.len() + 1),
            order_key,
            index,
            ready: Arc::clone(&self.ready),
        });
        for condition in conditions {
            if !condition.register(Arc::clone(&token) as Arc<dyn AdmissionWaiter>) {
                token.satisfy();
            }
        }
        token.satisfy();
        self.admit_ready(scope);
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
        while let Some(index) = self.ready.pop() {
            let Some(job) = self.take_job(index) else {
                continue;
            };
            scope.spawn(move |scope| {
                job(&ScopeAdmit {
                    scheduler: self,
                    scope,
                });
                self.admit_ready(scope);
            });
            spawned += 1;
        }
        spawned
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
        let mut stranded = Vec::new();
        {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            for slot in slots.iter_mut() {
                if let Some(job) = slot.job.take() {
                    stranded.push((slot.order_key, job));
                }
            }
        }
        let Some(lowest_order_key) = stranded.iter().map(|(key, _)| *key).min() else {
            return Ok(());
        };
        Err(ParallelError::JobsNeverAdmitted {
            count: stranded.len(),
            lowest_order_key,
        })
    }

    /// Takes the job stored in `index`, or `None` when another drain won it.
    fn take_job(&self, index: usize) -> Option<Job<'job>> {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(index)
            .and_then(|slot| slot.job.take())
    }
}

/// The [`Admit`] handle handed to a running job: the scheduler plus the scope
/// the job itself was spawned into.
struct ScopeAdmit<'a, 'handle, 'scope, 'job> {
    scheduler: &'scope AdmissionScheduler<'job>,
    scope: &'a TaskScope<'handle, 'scope>,
}

impl<'job> Admit<'job> for ScopeAdmit<'_, '_, '_, 'job> {
    fn admit_ready(&self) -> usize {
        self.scheduler.admit_ready(self.scope)
    }

    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job>) {
        self.scheduler
            .submit(self.scope, order_key, conditions, job);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::pool::{WorkerPool, ready_task_scope};
    use crate::thread_count::ThreadCount;
    use core::num::NonZeroUsize;
    use std::sync::atomic::AtomicUsize;

    fn pool(threads: usize) -> WorkerPool {
        WorkerPool::new(ThreadCount::Fixed(NonZeroUsize::new(threads).unwrap())).unwrap()
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
    }

    #[test]
    fn a_job_whose_conditions_already_hold_is_spawned_at_submit() {
        let rows = WatermarkCell::new();
        let done = CompletionCell::completed(7u32);
        let log = RunLog::default();
        let scheduler = AdmissionScheduler::new();
        rows.publish(3);
        let pool = pool(2);
        pool.install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::Watermark(&rows, 3), Condition::Completion(&done)],
                    Box::new(|_| log.record(0)),
                );
            })
        })
        .unwrap();
        scheduler.finish().unwrap();
        assert_eq!(log.keys(), vec![0]);
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
