// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A monotonic progress watermark ([`WatermarkCell`]) with threshold admission.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::admission::AdmissionWaiter;
use crate::completion::ASSIST_PARK;
use crate::pool::{PoolAssist, assist_installed_pool};

/// One registered threshold waiter: the admission token fires once the
/// watermark reaches `threshold`.
#[derive(Debug)]
struct ThresholdWaiter {
    /// The smallest published value that satisfies this waiter.
    threshold: usize,
    /// The token notified when the threshold is reached.
    waiter: Arc<dyn AdmissionWaiter>,
}

/// A monotonic `usize` progress watermark that admits threshold waiters.
///
/// A producer publishes how far it has got (rows filtered, sbrows resolved) and
/// a consumer states the smallest value it needs. A threshold is satisfied when
/// [`WatermarkCell::current`] is **greater than or equal to** it, so a consumer
/// that needs rows `0..n` registers threshold `n`. The fast path is a lock-free
/// `Acquire` load; the mutex and condition variable exist only to hold the
/// waiter list and to park a blocked driver.
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
    waiters: Mutex<Vec<ThresholdWaiter>>,
    cond: Condvar,
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
            waiters: Mutex::new(Vec::new()),
            cond: Condvar::new(),
        }
    }

    /// The highest published value, without blocking.
    #[must_use]
    pub fn current(&self) -> usize {
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
        let previous = self.value.fetch_max(value, Ordering::AcqRel);
        if previous >= value {
            return self.current();
        }
        let fired = {
            let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
            let (fired, pending) = waiters
                .drain(..)
                .partition::<Vec<_>, _>(|entry| entry.threshold <= value);
            *waiters = pending;
            fired
        };
        self.cond.notify_all();
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
        let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        if self.value.load(Ordering::Acquire) >= threshold {
            return false;
        }
        waiters.push(ThresholdWaiter { threshold, waiter });
        true
    }

    /// Blocks until the watermark reaches `threshold`, running pool jobs while
    /// it waits, and returns the watermark observed at that point.
    ///
    /// Reserved for the pipeline driver thread, exactly like
    /// [`crate::CompletionCell::wait_with_pool_assist`] and with the same
    /// reentrancy contract: each assist step runs one arbitrary pool job, so the
    /// caller must hold no lock, no thread-local scope guard, and no borrow such
    /// a job could need. A pool task must never call this — a task that waits
    /// can be resumed only by work below it on its own stack. Pool tasks state
    /// their needs as an admission [`crate::Condition`] instead.
    pub fn wait_at_least(&self, threshold: usize) -> usize {
        loop {
            let current = self.current();
            if current >= threshold {
                return current;
            }
            match assist_installed_pool() {
                PoolAssist::Executed => (),
                PoolAssist::Idle => self.park_briefly(threshold),
                PoolAssist::OffPool => return self.park_until(threshold),
            }
        }
    }

    /// Parks for at most [`ASSIST_PARK`], returning early on a publish.
    fn park_briefly(&self, threshold: usize) {
        let waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        if self.current() < threshold {
            drop(
                self.cond
                    .wait_timeout(waiters, ASSIST_PARK)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
    }

    /// Blocks off-pool until the watermark reaches `threshold`.
    fn park_until(&self, threshold: usize) -> usize {
        let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let current = self.current();
            if current >= threshold {
                return current;
            }
            waiters = self
                .cond
                .wait(waiters)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

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
    fn failed_publish_admits_every_threshold() {
        let cell = WatermarkCell::new();
        let waiter = Arc::new(CountingWaiter::default());
        assert!(cell.register(WatermarkCell::FAILED, waiter.clone()));
        cell.publish(WatermarkCell::FAILED);
        assert_eq!(waiter.count(), 1);
        assert_eq!(cell.current(), WatermarkCell::FAILED);
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
    fn wait_at_least_returns_immediately_when_reached() {
        let cell = WatermarkCell::new();
        cell.publish(7);
        assert_eq!(cell.wait_at_least(7), 7);
        assert_eq!(cell.wait_at_least(0), 7);
    }

    #[test]
    fn wait_at_least_blocks_until_a_publisher_reaches_the_threshold() {
        let cell = Arc::new(WatermarkCell::new());
        let publisher = Arc::clone(&cell);
        let handle = std::thread::spawn(move || {
            publisher.publish(1);
            publisher.publish(4);
        });
        assert!(cell.wait_at_least(4) >= 4);
        handle.join().unwrap();
    }
}
