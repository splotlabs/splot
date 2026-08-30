// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A monotonic progress watermark ([`WatermarkCell`]) with threshold admission.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
use std::cmp::{Ordering as CmpOrdering, Reverse};
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::admission::AdmissionWaiter;
use crate::pool::{bind_installed_pool_progress, notify_bound_pool_progress};
use crate::progress::PoolProgressBindings;

/// One registered threshold waiter: the admission token fires once the
/// watermark reaches `threshold`.
#[derive(Debug)]
struct ThresholdWaiter {
    /// The smallest published value that satisfies this waiter.
    threshold: usize,
    /// The token notified when the threshold is reached.
    waiter: Arc<dyn AdmissionWaiter>,
}

impl PartialEq for ThresholdWaiter {
    fn eq(&self, other: &Self) -> bool {
        self.threshold == other.threshold
    }
}

impl Eq for ThresholdWaiter {}

impl PartialOrd for ThresholdWaiter {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for ThresholdWaiter {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.threshold.cmp(&other.threshold)
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct WatermarkMetrics {
    registrations: AtomicUsize,
    publish_calls: AtomicUsize,
    waiters_inspected: AtomicUsize,
    waiters_fired: AtomicUsize,
    waiter_lock_acquisitions: AtomicUsize,
    maximum_waiter_count: AtomicUsize,
    publication_temporary_allocations: AtomicUsize,
}

#[cfg(test)]
impl WatermarkMetrics {
    const fn new() -> Self {
        Self {
            registrations: AtomicUsize::new(0),
            publish_calls: AtomicUsize::new(0),
            waiters_inspected: AtomicUsize::new(0),
            waiters_fired: AtomicUsize::new(0),
            waiter_lock_acquisitions: AtomicUsize::new(0),
            maximum_waiter_count: AtomicUsize::new(0),
            publication_temporary_allocations: AtomicUsize::new(0),
        }
    }

    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            registrations: self.registrations.load(Ordering::Relaxed),
            publish_calls: self.publish_calls.load(Ordering::Relaxed),
            waiters_inspected: self.waiters_inspected.load(Ordering::Relaxed),
            waiters_fired: self.waiters_fired.load(Ordering::Relaxed),
            waiter_lock_acquisitions: self.waiter_lock_acquisitions.load(Ordering::Relaxed),
            maximum_waiter_count: self.maximum_waiter_count.load(Ordering::Relaxed),
            publication_temporary_allocations: self
                .publication_temporary_allocations
                .load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MetricsSnapshot {
    registrations: usize,
    publish_calls: usize,
    waiters_inspected: usize,
    waiters_fired: usize,
    waiter_lock_acquisitions: usize,
    maximum_waiter_count: usize,
    publication_temporary_allocations: usize,
}

/// A monotonic `usize` progress watermark that admits threshold waiters.
///
/// A producer publishes how far it has got (rows filtered, sbrows resolved) and
/// a consumer states the smallest value it needs. A threshold is satisfied when
/// [`WatermarkCell::current`] is **greater than or equal to** it, so a consumer
/// that needs rows `0..n` registers threshold `n`. The fast path is a lock-free
/// `Acquire` load; the mutex guards only the waiter list.
///
/// Registered waiters fire exactly once, on the publish that first reaches
/// their threshold. [`WatermarkCell::register`] returns `false` when the
/// threshold already holds, in which case the waiter is never called and the
/// caller must count that condition as satisfied itself.
///
/// # Failure contract
///
/// A producer that fails must still publish, or its dependents are never
/// admitted. Publishing [`WatermarkCell::FAILED`] satisfies every threshold, so
/// dependents are admitted and then fail closed when they read the producer's
/// state.
#[derive(Debug, Default)]
pub struct WatermarkCell {
    value: AtomicUsize,
    progress: PoolProgressBindings,
    waiters: Mutex<BinaryHeap<Reverse<ThresholdWaiter>>>,
    #[cfg(test)]
    metrics: WatermarkMetrics,
}

impl WatermarkCell {
    /// The watermark a failed producer publishes so every dependent is admitted
    /// and can fail closed on the state it reads.
    pub const FAILED: usize = usize::MAX;

    /// Creates a watermark at `0` with no waiters.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: AtomicUsize::new(0),
            progress: PoolProgressBindings::new(),
            waiters: Mutex::new(BinaryHeap::new()),
            #[cfg(test)]
            metrics: WatermarkMetrics::new(),
        }
    }

    /// The highest published value, without blocking.
    #[must_use]
    pub fn current(&self) -> usize {
        bind_installed_pool_progress(&self.progress);
        self.value.load(Ordering::Acquire)
    }

    /// Publishes `value` and fires every waiter it satisfies, returning the
    /// watermark afterwards.
    ///
    /// The watermark is a running maximum: publishing a value at or below the
    /// current one is a no-op that fires nothing, because the publish that
    /// raised the watermark already fired every waiter at or below it.
    ///
    /// The returned value is the watermark this call observed last, which a
    /// concurrent publisher may already have raised past `value`; it is never
    /// below what this call established. A caller that acts on progress
    /// therefore never acts on a stale figure.
    ///
    /// Waiters are collected under the lock and notified after it is released,
    /// so no scheduler lock is ever held across a waiter callback.
    pub fn publish(&self, value: usize) -> usize {
        #[cfg(test)]
        self.metrics.publish_calls.fetch_add(1, Ordering::Relaxed);
        let previous = self.value.fetch_max(value, Ordering::AcqRel);
        if previous >= value {
            return self.current();
        }
        notify_bound_pool_progress(&self.progress);
        let fired = {
            #[cfg(test)]
            self.metrics
                .waiter_lock_acquisitions
                .fetch_add(1, Ordering::Relaxed);
            let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
            let mut fired = Vec::new();
            while let Some(Reverse(entry)) = waiters.peek() {
                #[cfg(test)]
                self.metrics
                    .waiters_inspected
                    .fetch_add(1, Ordering::Relaxed);
                if entry.threshold > value {
                    break;
                }
                let Some(Reverse(entry)) = waiters.pop() else {
                    break;
                };
                #[cfg(test)]
                let capacity = fired.capacity();
                fired.push(entry);
                #[cfg(test)]
                if fired.capacity() != capacity {
                    self.metrics
                        .publication_temporary_allocations
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            #[cfg(test)]
            self.metrics
                .waiters_fired
                .fetch_add(fired.len(), Ordering::Relaxed);
            fired
        };
        for entry in fired {
            entry.waiter.satisfy();
        }
        self.current()
    }

    /// Registers `waiter` to fire when the watermark reaches `threshold`.
    ///
    /// Returns `false` when the threshold already holds; the waiter is then not
    /// stored and never called, so the caller must treat the condition as
    /// satisfied itself. Returning `true` promises exactly one later
    /// [`AdmissionWaiter::satisfy`] call.
    pub fn register(&self, threshold: usize, waiter: Arc<dyn AdmissionWaiter>) -> bool {
        bind_installed_pool_progress(&self.progress);
        #[cfg(test)]
        self.metrics
            .waiter_lock_acquisitions
            .fetch_add(1, Ordering::Relaxed);
        let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        if self.value.load(Ordering::Acquire) >= threshold {
            return false;
        }
        waiters.push(Reverse(ThresholdWaiter { threshold, waiter }));
        #[cfg(test)]
        {
            self.metrics.registrations.fetch_add(1, Ordering::Relaxed);
            self.metrics
                .maximum_waiter_count
                .fetch_max(waiters.len(), Ordering::Relaxed);
        }
        true
    }

    #[cfg(test)]
    fn metrics(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Barrier, Weak};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A waiter that records how often it was satisfied.
    #[derive(Debug, Default)]
    struct CountingWaiter(AtomicUsize);

    impl CountingWaiter {
        fn count(&self) -> usize {
            self.0.load(Ordering::Acquire)
        }
    }

    impl AdmissionWaiter for CountingWaiter {
        fn satisfy(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[derive(Debug)]
    struct RecordingWaiter {
        id: usize,
        fired: Arc<Mutex<Vec<usize>>>,
    }

    impl AdmissionWaiter for RecordingWaiter {
        fn satisfy(&self) {
            self.fired
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(self.id);
        }
    }

    #[derive(Debug)]
    struct ReentrantWaiter {
        cell: Weak<WatermarkCell>,
        nested: Arc<CountingWaiter>,
    }

    impl AdmissionWaiter for ReentrantWaiter {
        fn satisfy(&self) {
            if let Some(cell) = self.cell.upgrade() {
                assert!(cell.register(2, self.nested.clone()));
            }
        }
    }

    fn check_registration_order(thresholds: &[usize]) {
        let cell = WatermarkCell::new();
        let waiters: Vec<_> = thresholds
            .iter()
            .map(|&threshold| {
                let waiter = Arc::new(CountingWaiter::default());
                assert!(cell.register(threshold, waiter.clone()));
                (threshold, waiter)
            })
            .collect();
        for published in 1..=thresholds.iter().copied().max().unwrap_or_default() {
            cell.publish(published);
            for (threshold, waiter) in &waiters {
                assert_eq!(waiter.count(), usize::from(*threshold <= published));
            }
        }
    }

    #[test]
    fn publish_is_monotonic() {
        let cell = WatermarkCell::new();
        assert_eq!(cell.current(), 0);
        assert_eq!(cell.publish(5), 5);
        assert_eq!(cell.publish(3), 5);
        assert_eq!(cell.publish(5), 5);
        assert_eq!(cell.current(), 5);
        assert_eq!(cell.publish(6), 6);
    }

    #[test]
    fn register_reports_an_already_satisfied_threshold() {
        let cell = WatermarkCell::new();
        cell.publish(4);
        let waiter = Arc::new(CountingWaiter::default());
        assert!(!cell.register(4, waiter.clone()), "equality already holds");
        assert!(!cell.register(0, waiter.clone()), "zero always holds");
        assert!(cell.register(5, waiter.clone()));
        assert_eq!(waiter.count(), 0);
    }

    #[test]
    fn thresholds_fire_at_equality_and_only_once() {
        let cell = WatermarkCell::new();
        let at_three = Arc::new(CountingWaiter::default());
        let at_four = Arc::new(CountingWaiter::default());
        assert!(cell.register(3, at_three.clone()));
        assert!(cell.register(4, at_four.clone()));

        cell.publish(3);
        assert_eq!(at_three.count(), 1, "threshold 3 fires at exactly 3");
        assert_eq!(at_four.count(), 0);

        cell.publish(3);
        cell.publish(9);
        assert_eq!(at_three.count(), 1, "a fired waiter never fires again");
        assert_eq!(at_four.count(), 1);
    }

    #[test]
    fn thresholds_fire_after_increasing_decreasing_and_random_registration() {
        check_registration_order(&[1, 2, 3, 4, 5, 6, 7, 8]);
        check_registration_order(&[8, 7, 6, 5, 4, 3, 2, 1]);
        check_registration_order(&[6, 1, 8, 3, 7, 2, 5, 4]);
    }

    #[test]
    fn duplicate_thresholds_all_fire_once() {
        let cell = WatermarkCell::new();
        let waiters: Vec<_> = (0..128)
            .map(|_| Arc::new(CountingWaiter::default()))
            .collect();
        for waiter in &waiters {
            assert!(cell.register(7, waiter.clone()));
        }
        cell.publish(6);
        assert!(waiters.iter().all(|waiter| waiter.count() == 0));
        cell.publish(7);
        cell.publish(9);
        assert!(waiters.iter().all(|waiter| waiter.count() == 1));
    }

    #[test]
    fn failed_publish_admits_every_threshold() {
        let cell = WatermarkCell::new();
        let waiters: Vec<_> = [1, 9, WatermarkCell::FAILED]
            .into_iter()
            .map(|threshold| {
                let waiter = Arc::new(CountingWaiter::default());
                assert!(cell.register(threshold, waiter.clone()));
                waiter
            })
            .collect();
        cell.publish(WatermarkCell::FAILED);
        assert!(waiters.iter().all(|waiter| waiter.count() == 1));
        assert_eq!(cell.current(), WatermarkCell::FAILED);
    }

    #[test]
    fn failed_publish_wakes_an_assisted_driver() {
        use crate::pool::{WorkerPool, assist_pool_or_park, pool_progress_snapshot};
        use crate::thread_count::ThreadCount;

        let pool = WorkerPool::new(ThreadCount::Fixed(2.try_into().unwrap())).unwrap();
        let cell = Arc::new(WatermarkCell::new());
        let publisher_cell = Arc::clone(&cell);
        let publisher_pool = pool.clone();
        let publisher = thread::spawn(move || {
            while publisher_pool.parked_waiters() == 0 {
                thread::yield_now();
            }
            publisher_cell.publish(WatermarkCell::FAILED);
        });
        pool.install(|| {
            loop {
                let progress = pool_progress_snapshot();
                if cell.current() == WatermarkCell::FAILED {
                    break;
                }
                assist_pool_or_park(&progress);
            }
        });
        publisher.join().unwrap();
    }

    #[test]
    fn callbacks_run_outside_the_waiter_mutex() {
        let cell = Arc::new(WatermarkCell::new());
        let nested = Arc::new(CountingWaiter::default());
        assert!(cell.register(
            1,
            Arc::new(ReentrantWaiter {
                cell: Arc::downgrade(&cell),
                nested: nested.clone(),
            }),
        ));
        let completed = Arc::new(AtomicBool::new(false));
        let publisher = {
            let cell = cell.clone();
            let completed = completed.clone();
            thread::spawn(move || {
                cell.publish(1);
                completed.store(true, Ordering::Release);
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while !completed.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            completed.load(Ordering::Acquire),
            "reentrant registration must not deadlock"
        );
        publisher.join().unwrap();
        cell.publish(2);
        assert_eq!(nested.count(), 1);
    }

    #[test]
    fn registration_racing_publication_is_rejected_or_fired_once() {
        const RACES: usize = 2_048;
        let cells: Vec<_> = (0..RACES).map(|_| Arc::new(WatermarkCell::new())).collect();
        let waiters: Vec<_> = (0..RACES)
            .map(|_| Arc::new(CountingWaiter::default()))
            .collect();
        let barrier = Arc::new(Barrier::new(2));
        let registrations = thread::scope(|scope| {
            let register_barrier = barrier.clone();
            let register_cells = &cells;
            let register_waiters = &waiters;
            let register = scope.spawn(move || {
                let mut registered = Vec::with_capacity(RACES);
                for (cell, waiter) in register_cells.iter().zip(register_waiters) {
                    register_barrier.wait();
                    registered.push(cell.register(1, waiter.clone()));
                    register_barrier.wait();
                }
                registered
            });
            let publish_barrier = barrier.clone();
            let publish_cells = &cells;
            scope.spawn(move || {
                for cell in publish_cells {
                    publish_barrier.wait();
                    cell.publish(1);
                    publish_barrier.wait();
                }
            });
            register.join().unwrap()
        });
        for ((registered, waiter), cell) in registrations.iter().zip(&waiters).zip(&cells) {
            assert_eq!(waiter.count(), usize::from(*registered));
            assert_eq!(cell.current(), 1);
        }
    }

    #[test]
    fn concurrent_publishers_fire_every_waiter_once() {
        const WAITERS: usize = 512;
        const PUBLISHERS: usize = 8;
        let cell = Arc::new(WatermarkCell::new());
        let waiters: Vec<_> = (1..=WAITERS)
            .map(|threshold| {
                let waiter = Arc::new(CountingWaiter::default());
                assert!(cell.register(threshold, waiter.clone()));
                waiter
            })
            .collect();
        let barrier = Arc::new(Barrier::new(PUBLISHERS));
        thread::scope(|scope| {
            for publisher in 0..PUBLISHERS {
                let cell = cell.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    for value in (publisher + 1..=WAITERS).step_by(PUBLISHERS) {
                        cell.publish(value);
                        thread::yield_now();
                    }
                });
            }
        });
        assert_eq!(cell.current(), WAITERS);
        assert!(waiters.iter().all(|waiter| waiter.count() == 1));
    }

    #[test]
    fn no_op_publications_take_no_waiter_lock_and_metrics_cover_publication_cost() {
        let cell = WatermarkCell::new();
        let waiter = Arc::new(CountingWaiter::default());
        assert!(cell.register(5, waiter.clone()));
        for value in [0, 3, 3, 2, 5, 4] {
            cell.publish(value);
        }
        assert_eq!(waiter.count(), 1);
        assert_eq!(
            cell.metrics(),
            MetricsSnapshot {
                registrations: 1,
                publish_calls: 6,
                waiters_inspected: 2,
                waiters_fired: 1,
                waiter_lock_acquisitions: 3,
                maximum_waiter_count: 1,
                publication_temporary_allocations: 1,
            }
        );
    }

    #[test]
    fn publish_reports_a_racing_publisher_rather_than_its_own_value() {
        let cell = Arc::new(WatermarkCell::new());
        let ahead = Arc::clone(&cell);
        let waiter = Arc::new(CountingWaiter::default());
        assert!(cell.register(3, waiter));
        ahead.publish(10);

        assert_eq!(
            cell.publish(5),
            10,
            "a no-op publish reports the watermark, not the value it submitted"
        );
        assert_eq!(
            cell.publish(20),
            20,
            "a raising publish reports the watermark it established"
        );
    }

    #[test]
    fn randomized_operations_match_a_scanning_reference_model() {
        const OPERATIONS: usize = 50_000;
        let cell = WatermarkCell::new();
        let fired = Arc::new(Mutex::new(Vec::new()));
        let mut pending = Vec::new();
        let mut expected_current = 0usize;
        let mut random = 0xD1B5_4A32_D192_ED03u64;
        let mut next_id = 0usize;
        let mut expected_registrations = 0usize;

        for operation in 0..OPERATIONS {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            if random & 3 != 0 {
                let already_satisfied = random.trailing_zeros() >= 5;
                let threshold = if already_satisfied {
                    expected_current.saturating_sub((random as usize >> 8) & 7)
                } else {
                    expected_current + 1 + ((random as usize >> 8) & 255)
                };
                let registered = cell.register(
                    threshold,
                    Arc::new(RecordingWaiter {
                        id: next_id,
                        fired: fired.clone(),
                    }),
                );
                assert_eq!(
                    registered,
                    threshold > expected_current,
                    "operation {operation}"
                );
                if registered {
                    pending.push((threshold, next_id));
                    expected_registrations += 1;
                }
                next_id += 1;
                continue;
            }

            let value = expected_current + ((random as usize >> 8) & 15);
            expected_current = expected_current.max(value);
            assert_eq!(
                cell.publish(value),
                expected_current,
                "operation {operation}"
            );
            let mut expected_fired = Vec::new();
            pending.retain(|&(threshold, id)| {
                if threshold <= expected_current {
                    expected_fired.push(id);
                    false
                } else {
                    true
                }
            });
            let mut actual_fired =
                core::mem::take(&mut *fired.lock().unwrap_or_else(PoisonError::into_inner));
            expected_fired.sort_unstable();
            actual_fired.sort_unstable();
            assert_eq!(actual_fired, expected_fired, "operation {operation}");
        }

        cell.publish(WatermarkCell::FAILED);
        let mut actual_fired =
            core::mem::take(&mut *fired.lock().unwrap_or_else(PoisonError::into_inner));
        let mut expected_fired: Vec<_> = pending.into_iter().map(|(_, id)| id).collect();
        actual_fired.sort_unstable();
        expected_fired.sort_unstable();
        assert_eq!(actual_fired, expected_fired);
        assert_eq!(cell.metrics().registrations, expected_registrations);
    }

    #[derive(Debug, Default)]
    struct ScanningWatermark {
        value: AtomicUsize,
        waiters: Mutex<Vec<ThresholdWaiter>>,
        metrics: WatermarkMetrics,
    }

    impl ScanningWatermark {
        fn register(&self, threshold: usize, waiter: Arc<dyn AdmissionWaiter>) {
            self.metrics
                .waiter_lock_acquisitions
                .fetch_add(1, Ordering::Relaxed);
            let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
            waiters.push(ThresholdWaiter { threshold, waiter });
            self.metrics.registrations.fetch_add(1, Ordering::Relaxed);
            self.metrics
                .maximum_waiter_count
                .fetch_max(waiters.len(), Ordering::Relaxed);
        }

        fn publish(&self, value: usize) {
            self.metrics.publish_calls.fetch_add(1, Ordering::Relaxed);
            self.value.store(value, Ordering::Release);
            self.metrics
                .waiter_lock_acquisitions
                .fetch_add(1, Ordering::Relaxed);
            let fired = {
                let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
                let mut fired = Vec::new();
                let mut pending = Vec::new();
                for entry in waiters.drain(..) {
                    self.metrics
                        .waiters_inspected
                        .fetch_add(1, Ordering::Relaxed);
                    let target = if entry.threshold <= value {
                        &mut fired
                    } else {
                        &mut pending
                    };
                    let capacity = target.capacity();
                    target.push(entry);
                    if target.capacity() != capacity {
                        self.metrics
                            .publication_temporary_allocations
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                *waiters = pending;
                self.metrics
                    .waiters_fired
                    .fetch_add(fired.len(), Ordering::Relaxed);
                fired
            };
            for entry in fired {
                entry.waiter.satisfy();
            }
        }
    }

    fn benchmark_thresholds() -> Vec<usize> {
        const WAITERS: usize = 10_000;
        let mut random = 0x9E37_79B9_7F4A_7C15u64;
        (0..WAITERS)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random as usize % 1_000 + 1
            })
            .collect()
    }

    fn benchmark_ordered(thresholds: &[usize]) -> (Duration, MetricsSnapshot) {
        let cell = WatermarkCell::new();
        let waiter = Arc::new(CountingWaiter::default());
        let start = Instant::now();
        for &threshold in thresholds {
            assert!(cell.register(threshold, waiter.clone()));
        }
        for value in 1..=1_000 {
            cell.publish(value);
        }
        let elapsed = start.elapsed();
        assert_eq!(waiter.count(), thresholds.len());
        (elapsed, cell.metrics())
    }

    fn benchmark_scanning(thresholds: &[usize]) -> (Duration, MetricsSnapshot) {
        let cell = ScanningWatermark::default();
        let waiter = Arc::new(CountingWaiter::default());
        let start = Instant::now();
        for &threshold in thresholds {
            cell.register(threshold, waiter.clone());
        }
        for value in 1..=1_000 {
            cell.publish(value);
        }
        let elapsed = start.elapsed();
        assert_eq!(waiter.count(), thresholds.len());
        (elapsed, cell.metrics.snapshot())
    }

    #[test]
    #[ignore = "run with --release --ignored --nocapture for the local microbenchmark"]
    fn ordered_waiters_microbenchmark() {
        let thresholds = benchmark_thresholds();
        let mut ordered: Vec<_> = (0..9).map(|_| benchmark_ordered(&thresholds)).collect();
        let mut scanning: Vec<_> = (0..9).map(|_| benchmark_scanning(&thresholds)).collect();
        ordered.sort_by_key(|sample| sample.0);
        scanning.sort_by_key(|sample| sample.0);
        let (ordered_time, ordered_metrics) = ordered[4];
        let (scanning_time, scanning_metrics) = scanning[4];
        eprintln!(
            "watermark microbenchmark ordered={ordered_time:?} {ordered_metrics:?} scanning={scanning_time:?} {scanning_metrics:?}"
        );
        assert_eq!(ordered_metrics.registrations, thresholds.len());
        assert_eq!(ordered_metrics.waiters_fired, thresholds.len());
        assert_eq!(ordered_metrics.maximum_waiter_count, thresholds.len());
        assert_eq!(ordered_metrics.publish_calls, 1_000);
        assert_eq!(
            ordered_metrics.waiter_lock_acquisitions,
            thresholds.len() + 1_000
        );
        assert!(
            ordered_metrics.waiters_inspected < scanning_metrics.waiters_inspected / 100,
            "the ordered queue must remove the repeated pending-waiter scan"
        );
    }
}
