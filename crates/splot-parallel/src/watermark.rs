// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A monotonic progress watermark with threshold admission.
use std::cmp::{Ordering as CmpOrdering, Reverse};
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::admission::Waiter;
use crate::pool::{bind_installed_pool_progress, notify_bound_pool_progress};
use crate::progress::PoolProgressBindings;

#[derive(Debug)]
struct ThresholdWaiter {
    threshold: usize,
    waiter: Arc<Waiter>,
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

/// A monotonic progress watermark that admits threshold waiters.
///
/// A threshold holds when the current value is greater than or equal to it.
/// A failed producer publishes [`WatermarkCell::FAILED`] so every dependent is
/// admitted and can observe the producer's failure state.
#[derive(Debug, Default)]
pub struct WatermarkCell {
    value: AtomicUsize,
    progress: PoolProgressBindings,
    waiters: Mutex<BinaryHeap<Reverse<ThresholdWaiter>>>,
}

impl WatermarkCell {
    /// The terminal value published by a failed producer.
    pub const FAILED: usize = usize::MAX;

    /// Creates a watermark at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: AtomicUsize::new(0),
            progress: PoolProgressBindings::new(),
            waiters: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Returns the highest published value without blocking.
    #[must_use]
    pub fn current(&self) -> usize {
        bind_installed_pool_progress(&self.progress);
        self.value.load(Ordering::Acquire)
    }

    /// Raises the watermark and fires newly satisfied waiters.
    ///
    /// Callbacks run after the waiter lock is released.
    pub fn publish(&self, value: usize) -> usize {
        if self.value.fetch_max(value, Ordering::AcqRel) >= value {
            return self.current();
        }
        notify_bound_pool_progress(&self.progress);
        let fired = {
            let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
            let mut fired = Vec::new();
            while waiters
                .peek()
                .is_some_and(|Reverse(entry)| entry.threshold <= value)
            {
                if let Some(Reverse(entry)) = waiters.pop() {
                    fired.push(entry.waiter);
                }
            }
            fired
        };
        for waiter in fired {
            waiter.satisfy();
        }
        self.current()
    }

    pub(crate) fn register(&self, threshold: usize, waiter: Arc<Waiter>) -> bool {
        bind_installed_pool_progress(&self.progress);
        let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        if self.value.load(Ordering::Acquire) >= threshold {
            return false;
        }
        waiters.push(Reverse(ThresholdWaiter { threshold, waiter }));
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{AdmissionScheduler, Condition, ThreadCount, WorkerPool, ready_task_scope};
    use std::sync::Barrier;

    #[test]
    fn publication_is_monotonic_and_thresholds_use_equality() {
        let cell = WatermarkCell::new();
        let visits: Vec<_> = (0..2).map(|_| AtomicUsize::new(0)).collect();
        let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> = AdmissionScheduler::new();
        WorkerPool::new(ThreadCount::from(2usize))
            .unwrap()
            .install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::watermark(&cell, 3)],
                        crate::admission::Job::Boxed(Box::new(|_| {
                            visits[0].fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                    assert_eq!(cell.publish(2), 2);
                    assert_eq!(cell.publish(1), 2);
                    scheduler.admit_ready(scope);
                    assert_eq!(visits[0].load(Ordering::Acquire), 0);
                    assert_eq!(cell.publish(3), 3);
                    assert_eq!(cell.publish(3), 3);
                    scheduler.admit_ready(scope);
                    scheduler.submit(
                        scope,
                        1,
                        &[Condition::watermark(&cell, 3)],
                        crate::admission::Job::Boxed(Box::new(|_| {
                            visits[1].fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                })
                .unwrap();
            });
        assert!(
            visits
                .iter()
                .all(|visit| visit.load(Ordering::Acquire) == 1)
        );
        scheduler.finish().unwrap();
    }

    #[test]
    fn registration_racing_publication_is_rejected_or_fired_once() {
        let pool = WorkerPool::new(ThreadCount::from(2usize)).unwrap();
        for _ in 0..128 {
            let cell = Arc::new(WatermarkCell::new());
            let ran = AtomicUsize::new(0);
            let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> =
                AdmissionScheduler::new();
            let publishing = Arc::clone(&cell);
            let publisher = std::thread::spawn(move || publishing.publish(1));
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::watermark(cell.as_ref(), 1)],
                        crate::admission::Job::Boxed(Box::new(|_| {
                            ran.fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                    publisher.join().unwrap();
                    scheduler.admit_ready(scope);
                })
                .unwrap();
            });
            assert_eq!(ran.load(Ordering::Acquire), 1);
            scheduler.finish().unwrap();
        }
    }

    #[test]
    fn concurrent_pool_publishers_reach_max_and_fire_each_threshold_once() {
        const MAX: usize = 32;
        let cell = WatermarkCell::new();
        let visits: Vec<_> = (1..=MAX).map(|_| AtomicUsize::new(0)).collect();
        let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> = AdmissionScheduler::new();
        let start = Barrier::new(4);
        WorkerPool::new(ThreadCount::from(4usize))
            .unwrap()
            .install(|| {
                ready_task_scope(|scope| {
                    for (threshold, visit) in (1..=MAX).zip(&visits) {
                        scheduler.submit(
                            scope,
                            threshold as u64,
                            &[Condition::watermark(&cell, threshold)],
                            crate::admission::Job::Boxed(Box::new(move |_| {
                                visit.fetch_add(1, Ordering::Relaxed);
                            })),
                        );
                    }
                    for lane in 0..4 {
                        let cell = &cell;
                        let start = &start;
                        let scheduler = &scheduler;
                        scope.spawn(move |scope| {
                            start.wait();
                            for value in (lane + 1..=MAX).step_by(4) {
                                cell.publish(value);
                            }
                            scheduler.admit_ready(scope);
                        });
                    }
                })
                .unwrap();
            });
        assert_eq!(cell.current(), MAX);
        assert!(
            visits
                .iter()
                .all(|visit| visit.load(Ordering::Acquire) == 1)
        );
        scheduler.finish().unwrap();
    }

    #[test]
    fn failed_admits_every_waiter() {
        let cell = WatermarkCell::new();
        let visits: Vec<_> = (0..3).map(|_| AtomicUsize::new(0)).collect();
        let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> = AdmissionScheduler::new();
        WorkerPool::new(ThreadCount::from(2usize))
            .unwrap()
            .install(|| {
                ready_task_scope(|scope| {
                    for (threshold, visit) in [1, 9, WatermarkCell::FAILED].into_iter().zip(&visits)
                    {
                        scheduler.submit(
                            scope,
                            threshold as u64,
                            &[Condition::watermark(&cell, threshold)],
                            crate::admission::Job::Boxed(Box::new(move |_| {
                                visit.fetch_add(1, Ordering::Relaxed);
                            })),
                        );
                    }
                    cell.publish(WatermarkCell::FAILED);
                    scheduler.admit_ready(scope);
                })
                .unwrap();
            });
        assert!(
            visits
                .iter()
                .all(|visit| visit.load(Ordering::Acquire) == 1)
        );
        scheduler.finish().unwrap();
    }
}
