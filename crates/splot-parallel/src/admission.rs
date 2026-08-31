// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Dependency-ordered task admission.
//!
//! Pool jobs must not wait for other pool jobs: work stealing can otherwise
//! place a consumer above its producer and deadlock the pool. A submitted job
//! is stored until all of its [`Condition`]s hold. Final publication queues the
//! job as ready; a scheduler drain spawns it. Jobs running inside a drain keep
//! draining automatically, while an external publisher must call
//! [`AdmissionScheduler::admit_ready`] after publishing. The outstanding count
//! starts at `conditions + 1`; the final unit is cleared after registration, so
//! publication cannot admit a partially registered job. Reused slots carry
//! generations to reject stale notices.
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::atomic::{AtomicBool, Ordering};
//! use splot_parallel::{
//!     AdmissionScheduler, CompletionCell, Condition, ThreadCount, WatermarkCell, WorkerPool,
//!     ready_task_scope,
//! };
//!
//! let pool = WorkerPool::new(ThreadCount::from(2usize))?;
//! let parsed = CompletionCell::new();
//! let rows = WatermarkCell::new();
//! let ran = AtomicBool::new(false);
//! let scheduler = AdmissionScheduler::new();
//! pool.install(|| {
//!     ready_task_scope(|scope| {
//!         scheduler.submit(
//!             scope,
//!             0,
//!             &[Condition::completion(&parsed), Condition::watermark(&rows, 2)],
//!             Box::new(|_| ran.store(true, Ordering::Release)),
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
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::completion::CompletionCell;
use crate::error::ParallelError;
use crate::pool::{TaskScope, notify_installed_pool_progress};
use crate::watermark::WatermarkCell;

trait CompletionSource {
    fn register(&self, waiter: Arc<Waiter>) -> bool;
}

impl<V> CompletionSource for CompletionCell<V> {
    fn register(&self, waiter: Arc<Waiter>) -> bool {
        self.register_waiter(waiter)
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
}

/// A unit of deferred work.
pub type Job<'job> = Box<dyn for<'a> FnOnce(&'a dyn Admit<'job>) + Send + 'job>;

/// Operations available to a running admitted job.
pub trait Admit<'job>: Sync {
    /// Spawns all jobs that are admissible now.
    fn admit_ready(&self) -> usize;
    /// Submits a job under the same scheduler.
    fn submit(&self, order_key: u64, conditions: &[Condition<'_>], job: Job<'job>);
    /// Spawns a job already known to be ready, without a scheduler slot.
    fn spawn_ready(&self, job: Job<'job>);
    /// Submits proven-ready jobs as one ordered scheduler entry.
    fn submit_ready_batch(&self, order_key: u64, jobs: Vec<Job<'job>>);
    /// Records one serial successor for this worker.
    fn continue_ready(&self, order_key: u64, job: Job<'job>);
}

const CONTINUATION_BUDGET: usize = 8;

struct OrderedJob<'job> {
    order_key: u64,
    job: Job<'job>,
}

struct ContinuationSlot<'job>(Mutex<Option<OrderedJob<'job>>>);

impl<'job> ContinuationSlot<'job> {
    fn put(&self, next: OrderedJob<'job>) -> Result<(), OrderedJob<'job>> {
        let mut pending = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.is_some() {
            return Err(next);
        }
        *pending = Some(next);
        Ok(())
    }

    fn take(&self) -> Option<OrderedJob<'job>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Reverse(entry));
    }

    fn pop(&self) -> Option<ReadyEntry> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .map(|Reverse(entry)| entry)
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

struct Slot<'job> {
    generation: u64,
    order_key: u64,
    job: Option<Job<'job>>,
    waiter: Option<Arc<Waiter>>,
}

#[derive(Default)]
struct Slots<'job> {
    entries: Vec<Slot<'job>>,
    free: Vec<usize>,
    next_submission_order: u64,
}

impl<'job> Slots<'job> {
    fn store(
        &mut self,
        pending: usize,
        order_key: u64,
        job: Job<'job>,
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
        let waiter = Arc::new(Waiter {
            pending: AtomicUsize::new(pending),
            entry,
            ready: Arc::clone(ready),
        });
        let slot = &mut self.entries[index];
        slot.order_key = order_key;
        slot.job = Some(job);
        slot.waiter = Some(Arc::clone(&waiter));
        waiter
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

    fn take_job(&mut self, entry: ReadyEntry) -> Option<Job<'job>> {
        let slot = self.entries.get_mut(entry.index)?;
        if slot.generation != entry.generation {
            return None;
        }
        let job = slot.job.take()?;
        slot.waiter.take();
        self.free.push(entry.index);
        Some(job)
    }

    fn take_stranded(&mut self) -> Vec<(u64, Job<'job>)> {
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
pub struct AdmissionScheduler<'job> {
    slots: Mutex<Slots<'job>>,
    ready: Arc<ReadyQueue>,
}

impl<'job> AdmissionScheduler<'job> {
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Slots::default()),
            ready: Arc::new(ReadyQueue::default()),
        }
    }

    /// Stores `job` until every condition holds.
    ///
    /// If every condition holds by the end of registration, the job is admitted
    /// immediately; this includes a racing publication absorbed during
    /// registration. Publication after registration only queues the job, so an
    /// external publisher must call [`Self::admit_ready`] with the same live
    /// task scope.
    pub fn submit<'scope>(
        &'scope self,
        scope: &TaskScope<'_, 'scope>,
        order_key: u64,
        conditions: &[Condition<'_>],
        job: Job<'job>,
    ) where
        'job: 'scope,
    {
        let waiter = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
        let mut spawned = 0;
        while let Some(entry) = self.ready.pop() {
            let job = self
                .slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take_job(entry);
            let Some(job) = job else { continue };
            scope.spawn(move |scope| self.run_job(scope, job));
            spawned += 1;
        }
        if spawned != 0 {
            notify_installed_pool_progress();
        }
        spawned
    }

    fn run_job<'scope>(&'scope self, scope: &TaskScope<'_, 'scope>, mut job: Job<'job>)
    where
        'job: 'scope,
    {
        let continuations = ContinuationSlot(Mutex::new(None));
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
}

impl Default for AdmissionScheduler<'_> {
    fn default() -> Self {
        Self::new()
    }
}

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
        self.scope.spawn(move |scope| scheduler.run_job(scope, job));
    }

    fn submit_ready_batch(&self, order_key: u64, mut jobs: Vec<Job<'job>>) {
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
                Box::new(move |admit| {
                    for job in jobs {
                        admit.spawn_ready(job);
                    }
                }),
            ),
        }
    }

    fn continue_ready(&self, order_key: u64, job: Job<'job>) {
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
            let scheduler = AdmissionScheduler::new();
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
                        Box::new(|_| {
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
        let scheduler = AdmissionScheduler::new();
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
                    Box::new(|_| {
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
        let mut slots = Slots::default();
        let first = slots.store(1, 0, Box::new(|_| {}), &ready);
        let stale = first.entry;
        first.satisfy();
        assert!(slots.take_job(ready.pop().unwrap()).is_some());

        let second = slots.store(1, 1, Box::new(|_| {}), &ready);
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
        let scheduler = AdmissionScheduler::new();
        pool(4).install(|| {
            ready_task_scope(|scope| {
                for (index, visit) in visits.iter().enumerate() {
                    scheduler.submit(
                        scope,
                        index as u64,
                        &[Condition::watermark(&rows, 1)],
                        Box::new(move |admit| {
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
        let scheduler = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(scope, 9, &[Condition::completion(&gate)], Box::new(|_| {}));
                scheduler.submit(scope, 4, &[Condition::completion(&gate)], Box::new(|_| {}));
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
        let scheduler = AdmissionScheduler::new();
        pool(1).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[Condition::watermark(&rows, 99)],
                    Box::new(|_| ran.store(true, Ordering::Relaxed)),
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
            admit.continue_ready(left as u64, Box::new(move |a| chain(a, count, left - 1)));
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
                Box::new(move |admit| leaf_chain(admit, visits, id + 1, left - 1)),
            );
        }
    }

    #[test]
    fn continuations_are_iterative_and_allow_nested_work() {
        let chained = AtomicUsize::new(0);
        let nested = AtomicUsize::new(0);
        let scheduler = AdmissionScheduler::new();
        pool(2).install(|| {
            ready_task_scope(|scope| {
                scheduler.submit(
                    scope,
                    0,
                    &[],
                    Box::new(|admit| {
                        admit.spawn_ready(Box::new(|_| {
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
        let scheduler = AdmissionScheduler::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool(1).install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(scope, 0, &[], Box::new(|_| panic!("job panic")));
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
            let scheduler = AdmissionScheduler::new();
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
                            Box::new(move |admit| {
                                assert_eq!(first_rows.current(), 2);
                                assert_eq!(second_rows.current(), 3);
                                assert!(parsed.is_set() && prepared.is_set());
                                let base = id * 4;
                                visits[base].fetch_add(1, Ordering::Relaxed);
                                order.lock().unwrap().push(id);
                                admit.submit(
                                    key,
                                    &[Condition::completion(prepared)],
                                    Box::new(move |admit| {
                                        leaf_chain(admit, visits, base + 1, 1);
                                    }),
                                );
                                admit.submit(
                                    key,
                                    &[Condition::watermark(second_rows, 3)],
                                    Box::new(move |_| {
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
                        Box::new(move |admit| {
                            first_rows.publish(2);
                            parsed.set(()).unwrap();
                            admit.spawn_ready(Box::new(|admit| {
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
                assert_eq!(*order.lock().unwrap(), expected);
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
