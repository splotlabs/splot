// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot ([`CompletionCell`]) for pipeline hand-off.
use core::time::Duration;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};

use crate::admission::AdmissionWaiter;
use crate::pool::{PoolAssist, assist_installed_pool};

/// How long an assisted wait parks when the pool has no job to run, before it
/// re-polls both the cell and the pool. Short enough that work arriving during
/// the park is picked up promptly, long enough not to spin a core while the
/// pool is idle.
pub(crate) const ASSIST_PARK: Duration = Duration::from_micros(100);

const ADMISSION_EMPTY: u8 = 0;
const ADMISSION_REGISTERED: u8 = 1;
const ADMISSION_SET: u8 = 2;

/// A write-once slot that a consumer can block on until the value lands.
///
/// The value is published through a [`OnceLock`], so [`CompletionCell::get`] and
/// [`CompletionCell::is_set`] are lock-free. The first admission waiter is
/// registered without a mutex; wait state is allocated lazily only when a
/// caller blocks or registers another waiter. A cell accepts exactly one value:
/// a second
/// [`CompletionCell::set`] hands the rejected value back to its caller.
///
/// # Usage contract
///
/// Only the pipeline driver thread may block in [`CompletionCell::wait`] or
/// [`CompletionCell::wait_with_pool_assist`].
/// Producer tasks running on the worker pool must never wait on any cell — a
/// worker that blocks holds its thread while the value it waits for may itself
/// need a worker. Deadlock-freedom rests on that rule plus a frame-delay depth
/// no larger than the pool width, so the frames in flight can never outnumber
/// the workers able to complete them.
#[derive(Debug)]
pub struct CompletionCell<V> {
    value: OnceLock<V>,
    admission_waiter: OnceLock<Weak<dyn AdmissionWaiter>>,
    admission_state: AtomicU8,
    wait: OnceLock<CompletionWait>,
}

/// State needed only when publication has an observer.
#[derive(Debug)]
struct CompletionWait {
    state: Mutex<CompletionState>,
    cond: Condvar,
}

/// Blocking state plus the admission waiters still to be fired.
///
/// The value is checked again while this mutex is held, so a registration
/// either sees the settled cell or is guaranteed to be fired by the
/// [`CompletionCell::set`] that settles it.
#[derive(Debug)]
struct CompletionState {
    parked_waiters: usize,
    additional_waiters: Vec<Weak<dyn AdmissionWaiter>>,
}

impl<V> CompletionCell<V> {
    /// Creates an empty completion cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            admission_waiter: OnceLock::new(),
            admission_state: AtomicU8::new(ADMISSION_EMPTY),
            wait: OnceLock::new(),
        }
    }

    /// Creates a cell that already holds `value`.
    #[must_use]
    pub fn completed(value: V) -> Self {
        Self {
            value: OnceLock::from(value),
            admission_waiter: OnceLock::new(),
            admission_state: AtomicU8::new(ADMISSION_SET),
            wait: OnceLock::new(),
        }
    }

    /// Consumes the cell, returning the published value if there is one.
    #[must_use]
    pub fn into_inner(self) -> Option<V> {
        self.value.into_inner()
    }

    /// Publishes `value` and wakes every waiter.
    ///
    /// # Errors
    /// Returns `value` unchanged if the cell already holds one; an existing
    /// value is never overwritten.
    pub fn set(&self, value: V) -> Result<(), V> {
        self.value.set(value)?;
        if self.admission_state.swap(ADMISSION_SET, Ordering::AcqRel) == ADMISSION_REGISTERED
            && let Some(waiter) = self.admission_waiter.get().and_then(Weak::upgrade)
        {
            waiter.satisfy();
        }
        let Some(wait) = self.wait.get() else {
            return Ok(());
        };
        let (has_parked_waiters, additional_waiters) = {
            let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
            (
                state.parked_waiters != 0,
                core::mem::take(&mut state.additional_waiters),
            )
        };
        if has_parked_waiters {
            wait.cond.notify_all();
        }
        for waiter in additional_waiters {
            if let Some(waiter) = waiter.upgrade() {
                waiter.satisfy();
            }
        }
        Ok(())
    }

    /// Registers `waiter` to fire when the value is published.
    ///
    /// Returns `false` when the cell is already set; the waiter is then not
    /// stored and never called, so the caller must treat the condition as
    /// satisfied itself. Returning `true` promises exactly one later
    /// [`AdmissionWaiter::satisfy`] call while the caller retains another
    /// strong reference to `waiter`.
    pub fn register_waiter(&self, waiter: Arc<dyn AdmissionWaiter>) -> bool {
        if self.is_set() {
            return false;
        }
        let weak_waiter = Arc::downgrade(&waiter);
        drop(waiter);
        let Err(waiter) = self.admission_waiter.set(weak_waiter) else {
            return self
                .admission_state
                .compare_exchange(
                    ADMISSION_EMPTY,
                    ADMISSION_REGISTERED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok();
        };
        let wait = self.wait_state();
        let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_set() {
            return false;
        }
        state.additional_waiters.push(waiter);
        true
    }

    /// Returns the value if it has been published, without blocking.
    #[must_use]
    pub fn get(&self) -> Option<&V> {
        self.value.get()
    }

    /// Whether the value has been published, without blocking.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.value.get().is_some()
    }

    /// Blocks until the value is published, then returns it.
    ///
    /// Reserved for the pipeline driver thread: see the type-level usage
    /// contract. A worker-pool task that calls this can deadlock the pool.
    #[must_use]
    pub fn wait(&self) -> &V {
        loop {
            if let Some(value) = self.value.get() {
                return value;
            }
            let wait = self.wait_state();
            let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
            while self.value.get().is_none() {
                state.parked_waiters += 1;
                state = wait
                    .cond
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                state.parked_waiters -= 1;
            }
        }
    }

    /// Blocks until the value is published, running pool jobs while it waits.
    ///
    /// Like [`CompletionCell::wait`] this is reserved for the pipeline driver
    /// thread, and it additionally donates the wait to the pool: instead of
    /// parking idle while the producer's own tasks queue up behind a narrower
    /// pool, the caller runs those tasks itself. Off an installed pool it
    /// degrades to [`CompletionCell::wait`].
    ///
    /// # Reentrancy contract
    ///
    /// Each assist step runs one *arbitrary* pool job to completion, which can
    /// take milliseconds, can nest further pool work, and can publish other
    /// cells. The caller must therefore hold no lock, no thread-local scope
    /// guard, and no borrow that such a job could need, and no pool job may
    /// itself wait on a cell — the type-level usage contract already forbids
    /// that, and it is what keeps the assist deadlock-free.
    #[must_use]
    pub fn wait_with_pool_assist(&self) -> &V {
        loop {
            if let Some(value) = self.value.get() {
                return value;
            }
            match assist_installed_pool() {
                PoolAssist::Executed => (),
                PoolAssist::Idle => self.park_briefly(),
                PoolAssist::OffPool => return self.wait(),
            }
        }
    }

    /// Parks for at most [`ASSIST_PARK`], returning early when the value lands.
    fn park_briefly(&self) {
        if self.is_set() {
            return;
        }
        let wait = self.wait_state();
        let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_set() {
            return;
        }
        state.parked_waiters += 1;
        let (mut state, _) = wait
            .cond
            .wait_timeout(state, ASSIST_PARK)
            .unwrap_or_else(PoisonError::into_inner);
        state.parked_waiters -= 1;
    }

    fn wait_state(&self) -> &CompletionWait {
        self.wait.get_or_init(|| CompletionWait {
            state: Mutex::new(CompletionState {
                parked_waiters: 0,
                additional_waiters: Vec::new(),
            }),
            cond: Condvar::new(),
        })
    }
}

impl<V> Default for CompletionCell<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn set_publishes_to_get_and_wait() {
        let cell = CompletionCell::new();
        assert_eq!(cell.set(7u32), Ok(()));
        assert_eq!(cell.get(), Some(&7));
        assert_eq!(cell.wait(), &7);
        assert!(cell.wait.get().is_none());
    }

    #[test]
    fn second_set_returns_the_rejected_value() {
        let cell = CompletionCell::new();
        assert_eq!(cell.set(1u32), Ok(()));
        assert_eq!(cell.set(2u32), Err(2));
        assert_eq!(cell.get(), Some(&1));
    }

    #[test]
    fn is_set_transitions_once() {
        let cell = CompletionCell::new();
        assert!(!cell.is_set());
        assert_eq!(cell.get(), None);
        assert_eq!(cell.set("done"), Ok(()));
        assert!(cell.is_set());
    }

    #[test]
    fn default_is_empty() {
        let cell = CompletionCell::<u8>::default();
        assert!(!cell.is_set());
    }

    #[test]
    fn completed_starts_settled_and_rejects_a_later_set() {
        let cell = CompletionCell::completed(5u32);
        assert!(cell.is_set());
        assert_eq!(cell.get(), Some(&5));
        assert_eq!(cell.wait(), &5);
        assert_eq!(cell.set(6), Err(6));
    }

    #[test]
    fn into_inner_yields_the_published_value_only() {
        assert_eq!(CompletionCell::completed(3u32).into_inner(), Some(3));
        assert_eq!(CompletionCell::<u32>::new().into_inner(), None);
    }

    #[test]
    fn wait_blocks_until_another_thread_sets() {
        let cell = Arc::new(CompletionCell::new());
        let entered = Arc::new(AtomicBool::new(false));

        let waiter_cell = Arc::clone(&cell);
        let waiter_entered = Arc::clone(&entered);
        let waiter = std::thread::spawn(move || {
            waiter_entered.store(true, Ordering::SeqCst);
            *waiter_cell.wait()
        });

        while !entered.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        assert!(!cell.is_set(), "the waiter must not observe a value yet");
        assert_eq!(cell.set(99u32), Ok(()));

        assert_eq!(waiter.join().unwrap(), 99);
    }

    #[test]
    fn set_wakes_every_blocked_waiter() {
        let cell = Arc::new(CompletionCell::new());
        let waiters = (0..2)
            .map(|_| {
                let cell = Arc::clone(&cell);
                std::thread::spawn(move || *cell.wait())
            })
            .collect::<Vec<_>>();

        while cell.wait.get().is_none_or(|wait| {
            wait.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .parked_waiters
                != 2
        }) {
            std::thread::yield_now();
        }
        assert_eq!(cell.set(99u32), Ok(()));

        for waiter in waiters {
            assert_eq!(waiter.join().unwrap(), 99);
        }
    }

    #[test]
    fn assisted_wait_off_pool_blocks_like_plain_wait() {
        let cell = Arc::new(CompletionCell::new());
        let waiter_cell = Arc::clone(&cell);
        let waiter = std::thread::spawn(move || *waiter_cell.wait_with_pool_assist());
        assert_eq!(cell.set(11u32), Ok(()));
        assert_eq!(waiter.join().unwrap(), 11);
    }

    #[test]
    fn assisted_wait_runs_the_producer_task_on_the_waiting_worker() {
        use crate::pool::{WorkerPool, ready_task_scope};
        use crate::thread_count::ThreadCount;

        let pool = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
        let cell = CompletionCell::new();
        let observed = pool.install(|| {
            ready_task_scope(|scope| {
                let cell = &cell;
                scope.spawn(move |_| {
                    let _ = cell.set(23u32);
                });
                *cell.wait_with_pool_assist()
            })
            .unwrap()
        });
        assert_eq!(observed, 23, "the only worker must run the task it awaits");
    }

    /// A waiter that records how often it was satisfied.
    #[derive(Debug, Default)]
    struct CountingWaiter(std::sync::atomic::AtomicUsize);

    impl AdmissionWaiter for CountingWaiter {
        fn satisfy(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    fn a_registered_waiter_fires_once_on_set() {
        let cell = CompletionCell::new();
        let waiter = Arc::new(CountingWaiter::default());
        assert!(cell.register_waiter(waiter.clone()));
        assert!(cell.admission_waiter.get().is_some());
        assert!(cell.wait.get().is_none());
        assert_eq!(waiter.0.load(Ordering::Acquire), 0);
        assert_eq!(cell.set(1u32), Ok(()));
        assert_eq!(waiter.0.load(Ordering::Acquire), 1);
        assert_eq!(cell.set(2u32), Err(2), "a rejected set fires nothing");
        assert_eq!(waiter.0.load(Ordering::Acquire), 1);
    }

    #[test]
    fn additional_registered_waiters_use_the_locked_fallback() {
        let cell = CompletionCell::new();
        let first = Arc::new(CountingWaiter::default());
        let second = Arc::new(CountingWaiter::default());
        assert!(cell.register_waiter(first.clone()));
        assert!(cell.register_waiter(second.clone()));
        assert_eq!(
            cell.wait
                .get()
                .unwrap()
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .additional_waiters
                .len(),
            1
        );

        assert_eq!(cell.set(1u32), Ok(()));
        assert_eq!(first.0.load(Ordering::Acquire), 1);
        assert_eq!(second.0.load(Ordering::Acquire), 1);
    }

    #[test]
    fn registration_racing_publication_fires_exactly_once() {
        for _ in 0..128 {
            let cell = Arc::new(CompletionCell::new());
            let waiter = Arc::new(CountingWaiter::default());
            let registering_cell = Arc::clone(&cell);
            let registering_waiter = Arc::clone(&waiter);
            let registration = std::thread::spawn(move || {
                if !registering_cell.register_waiter(registering_waiter.clone()) {
                    registering_waiter.satisfy();
                }
            });
            assert_eq!(cell.set(1u32), Ok(()));
            registration.join().unwrap();
            assert_eq!(waiter.0.load(Ordering::Acquire), 1);
        }
    }

    #[test]
    fn registering_on_a_settled_cell_reports_the_condition_holds() {
        let waiter = Arc::new(CountingWaiter::default());
        let settled = CompletionCell::completed(3u32);
        assert!(!settled.register_waiter(waiter.clone()));

        let cell = CompletionCell::new();
        assert_eq!(cell.set(4u32), Ok(()));
        assert!(!cell.register_waiter(waiter.clone()));
        assert_eq!(waiter.0.load(Ordering::Acquire), 0);
    }

    #[test]
    fn cell_is_send_and_sync_for_shareable_values() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompletionCell<u32>>();
    }
}
