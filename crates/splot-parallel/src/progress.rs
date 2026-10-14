// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pool-scoped event state for pipeline-driver waits.

use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{OnceLock, Weak};

/// A generation counter whose waiters sleep only while it is unchanged.
///
/// A waiter reads `generation` before it checks its condition. To sleep, it
/// locks `state`, registers in `waiters`, and re-reads `generation`; a
/// notifier advances `generation` and then reads `waiters`. Both orders are
/// sequentially consistent, so either the waiter sees the new generation or
/// the notifier sees the waiter and notifies under `state`, which the waiter
/// holds until its wait releases it.
#[derive(Debug, Default)]
pub(crate) struct EventCount {
    generation: AtomicU64,
    waiters: AtomicUsize,
    state: Mutex<()>,
    cond: Condvar,
    #[cfg(test)]
    parked: AtomicUsize,
}

impl EventCount {
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn notify(&self, all: bool) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            return;
        }
        let state = self.state.lock();
        if all {
            self.cond.notify_all();
        } else {
            self.cond.notify_one();
        }
        drop(state);
    }

    pub(crate) fn wait_if_unchanged(&self, observed: u64) {
        self.wait_if_unchanged_inner(observed, || {});
    }

    fn wait_if_unchanged_inner(&self, observed: u64, setup: impl FnOnce()) {
        let mut state = self.state.lock();
        self.waiters.fetch_add(1, Ordering::SeqCst);
        if self.generation() == observed {
            setup();
            if self.generation() == observed {
                #[cfg(test)]
                self.parked.fetch_add(1, Ordering::Release);
                self.cond.wait(&mut state);
                #[cfg(test)]
                self.parked.fetch_sub(1, Ordering::Release);
            }
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        drop(state);
    }

    #[cfg(test)]
    fn parked(&self) -> usize {
        self.parked.load(Ordering::Acquire)
    }
}

/// One decoder pool's wake-up events.
///
/// The pipeline driver waits on `progress`, which every publication advances.
/// Idle queue workers wait on `work`, which only queued work advances, and
/// queued work also wakes the driver so it can steal. Completion and watermark
/// sources retain only weak links to their pool-owned event, so observing a
/// source cannot extend the pool's lifetime.
#[derive(Debug, Default)]
pub(crate) struct PoolProgressEvent {
    progress: EventCount,
    work: EventCount,
    pending_installs: AtomicUsize,
}

impl PoolProgressEvent {
    /// The driver's event, or the idle queue workers' event when `work` is set.
    pub(crate) fn channel(&self, work: bool) -> &EventCount {
        if work { &self.work } else { &self.progress }
    }

    pub(crate) fn notify(&self) {
        self.progress.notify(true);
    }

    /// Wakes one idle queue worker, or all of them when `all` is set.
    pub(crate) fn notify_work(&self, all: bool) {
        self.work.notify(all);
        self.progress.notify(true);
    }

    pub(crate) fn install_submitted(&self) {
        self.pending_installs.fetch_add(1, Ordering::AcqRel);
        self.notify_work(true);
    }

    pub(crate) fn install_started(&self) {
        self.pending_installs.fetch_sub(1, Ordering::AcqRel);
        self.notify();
    }

    pub(crate) fn has_pending_install(&self) -> bool {
        self.pending_installs.load(Ordering::Acquire) != 0
    }

    #[cfg(test)]
    pub(crate) fn parked_waiters(&self) -> usize {
        self.progress.parked() + self.work.parked()
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

    fn each(&self, notify: impl Fn(&PoolProgressEvent)) {
        if let Some(progress) = self.primary.get().and_then(Weak::upgrade) {
            notify(&progress);
        }
        if let Some(additional) = self.additional.get() {
            additional
                .lock()
                .retain(|bound| bound.upgrade().map(|progress| notify(&progress)).is_some());
        }
    }

    pub(crate) fn notify(&self) {
        self.each(PoolProgressEvent::notify);
    }

    /// Wakes one idle queue worker of each bound pool.
    pub(crate) fn notify_work_one(&self) {
        self.each(|progress| progress.notify_work(false));
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

    fn wait_until_parked(event: &EventCount, count: usize) {
        while event.parked() < count {
            thread::yield_now();
        }
    }

    fn spurious_notify(event: &EventCount) {
        let state = event.state.lock();
        event.cond.notify_all();
        drop(state);
    }

    #[test]
    fn notification_before_wait_skips_the_park() {
        let event = EventCount::default();
        let observed = event.generation();
        event.notify(true);
        event.wait_if_unchanged(observed);
        assert_eq!(event.parked(), 0);
        assert_eq!(event.generation(), 1);
    }

    #[test]
    fn notification_during_wait_setup_cannot_be_lost() {
        let event = Arc::new(EventCount::default());
        let release_publisher = Arc::new(AtomicBool::new(false));
        let publisher_event = Arc::clone(&event);
        let publisher_release = Arc::clone(&release_publisher);
        let publisher = thread::spawn(move || {
            while !publisher_release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            publisher_event.notify(true);
        });
        let observed = event.generation();
        event.wait_if_unchanged_inner(observed, || {
            release_publisher.store(true, Ordering::Release);
            while event.generation() == observed {
                thread::yield_now();
            }
        });
        publisher.join().expect("publisher");
        assert_eq!(event.parked(), 0);
        assert_eq!(event.generation(), 1);
    }

    #[test]
    fn notification_while_parked_wakes_the_waiter() {
        let event = Arc::new(EventCount::default());
        let waiter_event = Arc::clone(&event);
        let waiter = thread::spawn(move || {
            let observed = waiter_event.generation();
            waiter_event.wait_if_unchanged(observed);
        });
        wait_until_parked(&event, 1);
        event.notify(true);
        waiter.join().expect("waiter");
        assert_eq!(event.parked(), 0);
        assert_eq!(event.generation(), 1);
    }

    #[test]
    fn spurious_wake_returns_for_a_condition_recheck() {
        let event = Arc::new(EventCount::default());
        let waiter_event = Arc::clone(&event);
        let returned = Arc::new(AtomicBool::new(false));
        let waiter_returned = Arc::clone(&returned);
        let waiter = thread::spawn(move || {
            let observed = waiter_event.generation();
            waiter_event.wait_if_unchanged(observed);
            waiter_returned.store(true, Ordering::Release);
        });
        wait_until_parked(&event, 1);
        spurious_notify(&event);
        waiter.join().expect("waiter");
        assert!(returned.load(Ordering::Acquire));
        assert_eq!(event.generation(), 0);
    }

    #[test]
    fn repeated_wake_cycles_do_not_lose_progress() {
        const CYCLES: usize = 1_000;
        let event = Arc::new(EventCount::default());
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
            event.notify(true);
        }
        waiter.join().expect("waiter");
        assert_eq!(completed.load(Ordering::Acquire), CYCLES);
        assert_eq!(event.generation(), CYCLES as u64);
    }

    #[test]
    fn independent_events_do_not_cross_wake() {
        let first = Arc::new(EventCount::default());
        let second = Arc::new(EventCount::default());
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
        first.notify(true);
        first_waiter.join().expect("first waiter");
        assert_eq!(second.generation(), 0);
        assert_eq!(second.parked(), 1);
        second.notify(true);
        second_waiter.join().expect("second waiter");
    }

    #[test]
    fn bindings_rebind_after_the_first_event_drops() {
        let bindings = PoolProgressBindings::new();
        let first = Arc::new(PoolProgressEvent::default());
        bindings.bind(&Arc::downgrade(&first));
        assert!(bindings.additional.get().is_none());
        drop(first);

        let second = Arc::new(PoolProgressEvent::default());
        bindings.bind(&Arc::downgrade(&second));
        assert!(bindings.additional.get().is_some());
        bindings.notify();
        assert_eq!(second.channel(false).generation(), 1);
    }

    #[test]
    fn bindings_notify_each_live_event() {
        let bindings = PoolProgressBindings::new();
        let first = Arc::new(PoolProgressEvent::default());
        let second = Arc::new(PoolProgressEvent::default());
        bindings.bind(&Arc::downgrade(&first));
        assert!(bindings.additional.get().is_none());
        bindings.bind(&Arc::downgrade(&second));
        assert!(bindings.additional.get().is_some());
        bindings.notify();
        assert_eq!(first.channel(false).generation(), 1);
        assert_eq!(second.channel(false).generation(), 1);
    }

    #[test]
    fn high_volume_publication_has_no_lost_wake() {
        const NOTIFICATIONS: usize = 50_000;
        let event = Arc::new(EventCount::default());
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
            event.notify(true);
        }
        waiter.join().expect("waiter");
        assert_eq!(event.generation(), NOTIFICATIONS as u64);
        assert_eq!(published.load(Ordering::Acquire), NOTIFICATIONS);
    }
}
