// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pool-scoped event state for pipeline-driver waits.

use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{OnceLock, Weak};

/// One decoder pool's generation and condition variable.
///
/// The waiter reads `generation` before checking its condition and the pool.
/// If neither made progress, it locks `state`, registers itself, and waits only
/// while the generation still matches. A publisher advances the generation
/// before taking `state` and notifying, so publication either changes the
/// waiter's final check or happens after the condition-variable wait has
/// atomically released the mutex. No notification can fall between those two
/// cases. Completion and watermark sources retain only weak links to their
/// pool-owned event, so observing a source cannot extend the pool's lifetime.
#[derive(Debug)]
pub(crate) struct PoolProgressEvent {
    generation: AtomicU64,
    pending_installs: AtomicUsize,
    state: Mutex<()>,
    cond: Condvar,
    #[cfg(test)]
    parked_waiters: AtomicUsize,
}

impl PoolProgressEvent {
    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            pending_installs: AtomicUsize::new(0),
            state: Mutex::new(()),
            cond: Condvar::new(),
            #[cfg(test)]
            parked_waiters: AtomicUsize::new(0),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn notify(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let state = self.state.lock();
        self.cond.notify_all();
        drop(state);
    }

    pub(crate) fn install_submitted(&self) {
        self.pending_installs.fetch_add(1, Ordering::AcqRel);
        self.notify();
    }

    pub(crate) fn install_started(&self) {
        self.pending_installs.fetch_sub(1, Ordering::AcqRel);
        self.notify();
    }

    pub(crate) fn has_pending_install(&self) -> bool {
        self.pending_installs.load(Ordering::Acquire) != 0
    }

    pub(crate) fn wait_if_unchanged(&self, observed: u64) {
        self.wait_if_unchanged_inner(observed, || {});
    }

    fn wait_if_unchanged_inner(&self, observed: u64, setup: impl FnOnce()) {
        let state = self.state.lock();
        if self.generation() != observed {
            return;
        }
        setup();
        if self.generation() != observed {
            return;
        }
        #[cfg(test)]
        self.parked_waiters.fetch_add(1, Ordering::Release);
        let mut state = state;
        self.cond.wait(&mut state);
        #[cfg(test)]
        self.parked_waiters.fetch_sub(1, Ordering::Release);
        drop(state);
    }

    #[cfg(test)]
    pub(crate) fn parked_waiters(&self) -> usize {
        self.parked_waiters.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn spurious_notify(&self) {
        let state = self.state.lock();
        self.cond.notify_all();
        drop(state);
    }
}

#[derive(Debug)]
pub(crate) struct PoolProgressBindings {
    primary: OnceLock<Weak<PoolProgressEvent>>,
    additional: OnceLock<Mutex<Vec<Weak<PoolProgressEvent>>>>,
}

impl PoolProgressBindings {
    pub(crate) const fn new() -> Self {
        Self {
            primary: OnceLock::new(),
            additional: OnceLock::new(),
        }
    }

    pub(crate) fn bind(&self, binding: &Weak<PoolProgressEvent>) {
        if binding.strong_count() == 0 {
            return;
        }

        if self.primary.set(Weak::clone(binding)).is_ok() {
            return;
        }
        if self
            .primary
            .get()
            .is_some_and(|primary| primary.ptr_eq(binding))
        {
            return;
        }

        let mut additional = self
            .additional
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock();
        additional.retain(|bound| bound.strong_count() != 0);
        if !additional.iter().any(|bound| bound.ptr_eq(binding)) {
            additional.push(Weak::clone(binding));
        }
    }

    pub(crate) fn notify(&self) {
        if let Some(progress) = self.primary.get().and_then(Weak::upgrade) {
            progress.notify();
        }

        let Some(additional) = self.additional.get() else {
            return;
        };
        additional.lock().retain(|bound| {
            let Some(progress) = bound.upgrade() else {
                return false;
            };
            progress.notify();
            true
        });
    }
}

impl Default for PoolProgressBindings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    fn wait_until_parked(event: &PoolProgressEvent, count: usize) {
        while event.parked_waiters() < count {
            thread::yield_now();
        }
    }

    #[test]
    fn notification_before_wait_skips_the_park() {
        let event = PoolProgressEvent::new();
        let observed = event.generation();
        event.notify();
        event.wait_if_unchanged(observed);
        assert_eq!(event.parked_waiters(), 0);
        assert_eq!(event.generation(), 1);
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
        assert_eq!(event.parked_waiters(), 0);
        assert_eq!(event.generation(), 1);
    }

    #[test]
    fn notification_while_parked_wakes_the_waiter() {
        let event = Arc::new(PoolProgressEvent::new());
        let waiter_event = Arc::clone(&event);
        let waiter = thread::spawn(move || {
            let observed = waiter_event.generation();
            waiter_event.wait_if_unchanged(observed);
        });
        wait_until_parked(&event, 1);
        event.notify();
        waiter.join().expect("waiter");
        assert_eq!(event.parked_waiters(), 0);
        assert_eq!(event.generation(), 1);
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
        wait_until_parked(&event, 1);
        event.spurious_notify();
        waiter.join().expect("waiter");
        assert!(returned.load(Ordering::Acquire));
        assert_eq!(event.generation(), 0);
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
            wait_until_parked(&event, 1);
            event.notify();
        }
        waiter.join().expect("waiter");
        assert_eq!(completed.load(Ordering::Acquire), CYCLES);
        assert_eq!(event.generation(), CYCLES as u64);
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
        wait_until_parked(&first, 1);
        wait_until_parked(&second, 1);
        first.notify();
        first_waiter.join().expect("first waiter");
        assert_eq!(second.generation(), 0);
        assert_eq!(second.parked_waiters(), 1);
        second.notify();
        second_waiter.join().expect("second waiter");
    }

    #[test]
    fn bindings_rebind_after_the_first_event_drops() {
        let bindings = PoolProgressBindings::new();
        let first = Arc::new(PoolProgressEvent::new());
        bindings.bind(&Arc::downgrade(&first));
        assert!(bindings.additional.get().is_none());
        drop(first);

        let second = Arc::new(PoolProgressEvent::new());
        bindings.bind(&Arc::downgrade(&second));
        assert!(bindings.additional.get().is_some());
        bindings.notify();
        assert_eq!(second.generation(), 1);
    }

    #[test]
    fn bindings_notify_each_live_event() {
        let bindings = PoolProgressBindings::new();
        let first = Arc::new(PoolProgressEvent::new());
        let second = Arc::new(PoolProgressEvent::new());
        bindings.bind(&Arc::downgrade(&first));
        assert!(bindings.additional.get().is_none());
        bindings.bind(&Arc::downgrade(&second));
        assert!(bindings.additional.get().is_some());
        bindings.notify();
        assert_eq!(first.generation(), 1);
        assert_eq!(second.generation(), 1);
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
        assert_eq!(event.generation(), NOTIFICATIONS as u64);
        assert_eq!(published.load(Ordering::Acquire), NOTIFICATIONS);
    }
}
