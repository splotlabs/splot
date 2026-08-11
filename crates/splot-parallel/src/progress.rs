// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pool-scoped event state for pipeline-driver waits.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Instant;

/// Instrumentation for assisted pipeline-driver waits in one worker pool.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PoolWaitMetrics {
    /// Calls that checked the pool for one runnable job.
    pub assist_calls: u64,
    /// Jobs executed on the driver by an assist call.
    pub assisted_jobs: u64,
    /// Idle waits that reached the condition variable.
    pub idle_parks: u64,
    /// Total duration spent in idle waits, in nanoseconds.
    pub park_nanos: u64,
    /// Longest idle wait, in nanoseconds.
    pub max_park_nanos: u64,
    /// Timeout returns. Event-driven waits never increment this counter.
    pub timeout_wakes: u64,
    /// Progress notifications published by this pool.
    pub notifications: u64,
    /// Idle waits that returned after a progress notification.
    pub progress_wakes: u64,
    /// Total notification-to-wake duration, in nanoseconds.
    pub wake_to_progress_nanos: u64,
    /// Longest notification-to-wake duration, in nanoseconds.
    pub max_wake_to_progress_nanos: u64,
}

impl PoolWaitMetrics {
    /// Returns the counters added after `earlier` was captured.
    #[must_use]
    pub fn since(self, earlier: Self) -> Self {
        Self {
            assist_calls: self.assist_calls.saturating_sub(earlier.assist_calls),
            assisted_jobs: self.assisted_jobs.saturating_sub(earlier.assisted_jobs),
            idle_parks: self.idle_parks.saturating_sub(earlier.idle_parks),
            park_nanos: self.park_nanos.saturating_sub(earlier.park_nanos),
            max_park_nanos: self.max_park_nanos,
            timeout_wakes: self.timeout_wakes.saturating_sub(earlier.timeout_wakes),
            notifications: self.notifications.saturating_sub(earlier.notifications),
            progress_wakes: self.progress_wakes.saturating_sub(earlier.progress_wakes),
            wake_to_progress_nanos: self
                .wake_to_progress_nanos
                .saturating_sub(earlier.wake_to_progress_nanos),
            max_wake_to_progress_nanos: self.max_wake_to_progress_nanos,
        }
    }
}

/// One decoder pool's generation and condition variable.
///
/// The waiter reads `generation` before checking its condition and the pool.
/// If neither made progress, it locks `state`, registers itself, and waits only
/// while the generation still matches. A publisher advances the generation
/// before taking `state` and notifying, so publication either changes the
/// waiter's final check or happens after the condition-variable wait has
/// atomically released the mutex. No notification can fall between those two
/// cases. Completion and watermark sources retain only a weak link to this
/// pool-owned event, so observing a source cannot extend the pool's lifetime.
#[derive(Debug)]
pub(crate) struct PoolProgressEvent {
    generation: AtomicU64,
    waiters: AtomicUsize,
    state: Mutex<()>,
    cond: Condvar,
    epoch: Instant,
    last_notification_nanos: AtomicU64,
    assist_calls: AtomicU64,
    assisted_jobs: AtomicU64,
    idle_parks: AtomicU64,
    park_nanos: AtomicU64,
    max_park_nanos: AtomicU64,
    progress_wakes: AtomicU64,
    wake_to_progress_nanos: AtomicU64,
    max_wake_to_progress_nanos: AtomicU64,
}

impl PoolProgressEvent {
    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            waiters: AtomicUsize::new(0),
            state: Mutex::new(()),
            cond: Condvar::new(),
            epoch: Instant::now(),
            last_notification_nanos: AtomicU64::new(0),
            assist_calls: AtomicU64::new(0),
            assisted_jobs: AtomicU64::new(0),
            idle_parks: AtomicU64::new(0),
            park_nanos: AtomicU64::new(0),
            max_park_nanos: AtomicU64::new(0),
            progress_wakes: AtomicU64::new(0),
            wake_to_progress_nanos: AtomicU64::new(0),
            max_wake_to_progress_nanos: AtomicU64::new(0),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn notify(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.last_notification_nanos
            .store(self.elapsed_nanos(), Ordering::Release);
        self.cond.notify_all();
        drop(state);
    }

    pub(crate) fn note_assist(&self) {
        self.assist_calls.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_assisted_job(&self) {
        self.assisted_jobs.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn wait_if_unchanged(&self, observed: u64) {
        self.wait_if_unchanged_inner(observed, || {});
    }

    fn wait_if_unchanged_inner(&self, observed: u64, setup: impl FnOnce()) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.generation() != observed {
            return;
        }
        self.waiters.fetch_add(1, Ordering::Release);
        setup();
        if self.generation() != observed {
            self.waiters.fetch_sub(1, Ordering::Release);
            return;
        }
        self.idle_parks.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        state = self
            .cond
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
        let parked = nanos(started.elapsed().as_nanos());
        self.park_nanos.fetch_add(parked, Ordering::Relaxed);
        self.max_park_nanos.fetch_max(parked, Ordering::Relaxed);
        self.waiters.fetch_sub(1, Ordering::Release);
        if self.generation() != observed {
            let notified = self.last_notification_nanos.load(Ordering::Acquire);
            let latency = self.elapsed_nanos().saturating_sub(notified);
            self.progress_wakes.fetch_add(1, Ordering::Relaxed);
            self.wake_to_progress_nanos
                .fetch_add(latency, Ordering::Relaxed);
            self.max_wake_to_progress_nanos
                .fetch_max(latency, Ordering::Relaxed);
        }
        drop(state);
    }

    pub(crate) fn metrics(&self) -> PoolWaitMetrics {
        PoolWaitMetrics {
            assist_calls: self.assist_calls.load(Ordering::Relaxed),
            assisted_jobs: self.assisted_jobs.load(Ordering::Relaxed),
            idle_parks: self.idle_parks.load(Ordering::Relaxed),
            park_nanos: self.park_nanos.load(Ordering::Relaxed),
            max_park_nanos: self.max_park_nanos.load(Ordering::Relaxed),
            timeout_wakes: 0,
            notifications: self.generation(),
            progress_wakes: self.progress_wakes.load(Ordering::Relaxed),
            wake_to_progress_nanos: self.wake_to_progress_nanos.load(Ordering::Relaxed),
            max_wake_to_progress_nanos: self.max_wake_to_progress_nanos.load(Ordering::Relaxed),
        }
    }

    fn elapsed_nanos(&self) -> u64 {
        nanos(self.epoch.elapsed().as_nanos())
    }

    #[cfg(test)]
    fn spurious_notify(&self) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.cond.notify_all();
        drop(state);
    }
}

fn nanos(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    fn wait_until_waiting(event: &PoolProgressEvent) {
        while event.waiters.load(Ordering::Acquire) == 0 {
            thread::yield_now();
        }
    }

    #[test]
    fn notification_before_wait_skips_the_park() {
        let event = PoolProgressEvent::new();
        let observed = event.generation();
        event.notify();
        event.wait_if_unchanged(observed);
        let metrics = event.metrics();
        assert_eq!(metrics.idle_parks, 0);
        assert_eq!(metrics.notifications, 1);
    }

    #[test]
    fn notification_during_wait_setup_cannot_be_lost() {
        let event = Arc::new(PoolProgressEvent::new());
        let release_publisher = Arc::new(AtomicBool::new(false));
        let publisher_event = Arc::clone(&event);
        let publisher_release = Arc::clone(&release_publisher);
        let publisher = thread::spawn(move || {
            while !publisher_release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            publisher_event.notify();
        });
        let observed = event.generation();
        event.wait_if_unchanged_inner(observed, || {
            release_publisher.store(true, Ordering::Release);
            while event.generation() == observed {
                thread::yield_now();
            }
        });
        publisher.join().expect("publisher");
        assert_eq!(event.metrics().idle_parks, 0);
    }

    #[test]
    fn notification_while_parked_wakes_the_waiter() {
        let event = Arc::new(PoolProgressEvent::new());
        let waiter_event = Arc::clone(&event);
        let waiter = thread::spawn(move || {
            let observed = waiter_event.generation();
            waiter_event.wait_if_unchanged(observed);
        });
        wait_until_waiting(&event);
        event.notify();
        waiter.join().expect("waiter");
        let metrics = event.metrics();
        assert_eq!(metrics.idle_parks, 1);
        assert_eq!(metrics.progress_wakes, 1);
    }

    #[test]
    fn spurious_wake_returns_for_a_condition_recheck() {
        let event = Arc::new(PoolProgressEvent::new());
        let waiter_event = Arc::clone(&event);
        let returned = Arc::new(AtomicBool::new(false));
        let waiter_returned = Arc::clone(&returned);
        let waiter = thread::spawn(move || {
            let observed = waiter_event.generation();
            waiter_event.wait_if_unchanged(observed);
            waiter_returned.store(true, Ordering::Release);
        });
        wait_until_waiting(&event);
        event.spurious_notify();
        waiter.join().expect("waiter");
        assert!(returned.load(Ordering::Acquire));
        assert_eq!(event.metrics().progress_wakes, 0);
    }

    #[test]
    fn repeated_wake_cycles_do_not_lose_progress() {
        const CYCLES: usize = 1_000;
        let event = Arc::new(PoolProgressEvent::new());
        let completed = Arc::new(AtomicUsize::new(0));
        let waiter_event = Arc::clone(&event);
        let waiter_completed = Arc::clone(&completed);
        let waiter = thread::spawn(move || {
            for cycle in 0..CYCLES {
                let observed = waiter_event.generation();
                waiter_event.wait_if_unchanged(observed);
                waiter_completed.store(cycle + 1, Ordering::Release);
            }
        });
        for cycle in 0..CYCLES {
            while completed.load(Ordering::Acquire) != cycle {
                thread::yield_now();
            }
            wait_until_waiting(&event);
            event.notify();
        }
        waiter.join().expect("waiter");
        assert_eq!(completed.load(Ordering::Acquire), CYCLES);
        let metrics = event.metrics();
        assert_eq!(metrics.notifications, CYCLES as u64);
        assert!(metrics.idle_parks <= CYCLES as u64);
    }

    #[test]
    fn independent_events_do_not_cross_wake() {
        let first = Arc::new(PoolProgressEvent::new());
        let second = Arc::new(PoolProgressEvent::new());
        let barrier = Arc::new(Barrier::new(3));
        let first_waiter = {
            let event = Arc::clone(&first);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let observed = event.generation();
                barrier.wait();
                event.wait_if_unchanged(observed);
            })
        };
        let second_waiter = {
            let event = Arc::clone(&second);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let observed = event.generation();
                barrier.wait();
                event.wait_if_unchanged(observed);
            })
        };
        barrier.wait();
        wait_until_waiting(&first);
        wait_until_waiting(&second);
        first.notify();
        first_waiter.join().expect("first waiter");
        assert_eq!(second.metrics().notifications, 0);
        assert_eq!(second.waiters.load(Ordering::Acquire), 1);
        second.notify();
        second_waiter.join().expect("second waiter");
    }

    #[test]
    fn high_volume_publication_has_no_lost_wake() {
        const NOTIFICATIONS: usize = 50_000;
        let event = Arc::new(PoolProgressEvent::new());
        let published = Arc::new(AtomicUsize::new(0));
        let waiter_event = Arc::clone(&event);
        let waiter_published = Arc::clone(&published);
        let waiter = thread::spawn(move || {
            while waiter_published.load(Ordering::Acquire) < NOTIFICATIONS {
                let observed = waiter_event.generation();
                if waiter_published.load(Ordering::Acquire) < NOTIFICATIONS {
                    waiter_event.wait_if_unchanged(observed);
                }
            }
        });
        for value in 1..=NOTIFICATIONS {
            published.store(value, Ordering::Release);
            event.notify();
        }
        waiter.join().expect("waiter");
        assert_eq!(event.metrics().notifications, NOTIFICATIONS as u64);
        assert_eq!(published.load(Ordering::Acquire), NOTIFICATIONS);
    }
}
