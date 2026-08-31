// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A monotonic progress watermark with threshold admission.
use std::cmp::{Ordering as CmpOrdering, Reverse};
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::admission::AdmissionWaiter;
use crate::pool::{bind_installed_pool_progress, notify_bound_pool_progress};
use crate::progress::PoolProgressBindings;

#[derive(Debug)]
struct ThresholdWaiter {
    threshold: usize,
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

    pub(crate) fn register(&self, threshold: usize, waiter: Arc<dyn AdmissionWaiter>) -> bool {
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
    use crate::{ThreadCount, WorkerPool, ready_task_scope};
    use std::sync::Barrier;
    use std::sync::Weak;

    #[derive(Debug, Default)]
    struct Count(AtomicUsize);

    impl AdmissionWaiter for Count {
        fn satisfy(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[derive(Debug)]
    struct Reentrant {
        cell: Weak<WatermarkCell>,
        nested: Arc<Count>,
    }

    impl AdmissionWaiter for Reentrant {
        fn satisfy(&self) {
            if let Some(cell) = self.cell.upgrade() {
                assert!(cell.register(2, self.nested.clone()));
            }
        }
    }

    #[test]
    fn publication_is_monotonic_and_thresholds_use_equality() {
        let cell = WatermarkCell::new();
        let at_three = Arc::new(Count::default());
        assert!(cell.register(3, at_three.clone()));
        assert_eq!(cell.publish(2), 2);
        assert_eq!(cell.publish(1), 2);
        assert_eq!(at_three.0.load(Ordering::Acquire), 0);
        assert_eq!(cell.publish(3), 3);
        assert_eq!(cell.publish(3), 3);
        assert_eq!(at_three.0.load(Ordering::Acquire), 1);
        let settled = Arc::new(Count::default());
        assert!(!cell.register(3, settled));
    }

    #[test]
    fn registration_racing_publication_is_rejected_or_fired_once() {
        for _ in 0..128 {
            let cell = Arc::new(WatermarkCell::new());
            let waiter = Arc::new(Count::default());
            let publishing = Arc::clone(&cell);
            let publisher = std::thread::spawn(move || publishing.publish(1));
            let registered = cell.register(1, waiter.clone());
            publisher.join().unwrap();
            assert_eq!(waiter.0.load(Ordering::Acquire), usize::from(registered));
        }
    }

    #[test]
    fn concurrent_pool_publishers_reach_max_and_fire_each_threshold_once() {
        const MAX: usize = 32;
        let cell = WatermarkCell::new();
        let waiters: Vec<_> = (1..=MAX).map(|_| Arc::new(Count::default())).collect();
        for (threshold, waiter) in (1..=MAX).zip(&waiters) {
            assert!(cell.register(threshold, waiter.clone()));
        }
        let start = Barrier::new(4);
        WorkerPool::new(ThreadCount::from(4usize))
            .unwrap()
            .install(|| {
                ready_task_scope(|scope| {
                    for lane in 0..4 {
                        let cell = &cell;
                        let start = &start;
                        scope.spawn(move |_| {
                            start.wait();
                            for value in (lane + 1..=MAX).step_by(4) {
                                cell.publish(value);
                            }
                        });
                    }
                })
                .unwrap();
            });
        assert_eq!(cell.current(), MAX);
        assert!(
            waiters
                .iter()
                .all(|waiter| waiter.0.load(Ordering::Acquire) == 1)
        );
    }

    #[test]
    fn callbacks_run_outside_the_waiter_lock() {
        let cell = Arc::new(WatermarkCell::new());
        let nested = Arc::new(Count::default());
        assert!(cell.register(
            1,
            Arc::new(Reentrant {
                cell: Arc::downgrade(&cell),
                nested: Arc::clone(&nested),
            }),
        ));
        cell.publish(1);
        cell.publish(2);
        assert_eq!(nested.0.load(Ordering::Acquire), 1);
    }

    #[test]
    fn failed_admits_every_waiter() {
        let cell = WatermarkCell::new();
        let waiters: Vec<_> = [1, 9, WatermarkCell::FAILED]
            .into_iter()
            .map(|threshold| {
                let waiter = Arc::new(Count::default());
                assert!(cell.register(threshold, waiter.clone()));
                waiter
            })
            .collect();
        cell.publish(WatermarkCell::FAILED);
        assert!(
            waiters
                .iter()
                .all(|waiter| waiter.0.load(Ordering::Acquire) == 1)
        );
    }
}
