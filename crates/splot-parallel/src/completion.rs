// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot ([`CompletionCell`]) for pipeline hand-off.
use core::time::Duration;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use crate::pool::{PoolAssist, assist_installed_pool};

/// How long an assisted wait parks when the pool has no job to run, before it
/// re-polls both the cell and the pool. Short enough that work arriving during
/// the park is picked up promptly, long enough not to spin a core while the
/// pool is idle.
pub(crate) const ASSIST_PARK: Duration = Duration::from_micros(100);

/// A write-once slot that a consumer can block on until the value lands.
///
/// The value is published through a [`OnceLock`], so [`CompletionCell::get`] and
/// [`CompletionCell::is_set`] are lock-free; the mutex and condition variable
/// exist only to park a blocked [`CompletionCell::wait`] caller instead of
/// spinning. A cell accepts exactly one value: a second
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
    done: Mutex<bool>,
    cond: Condvar,
}

impl<V> CompletionCell<V> {
    /// Creates an empty completion cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            done: Mutex::new(false),
            cond: Condvar::new(),
        }
    }

    /// Creates a cell that already holds `value`.
    #[must_use]
    pub fn completed(value: V) -> Self {
        Self {
            value: OnceLock::from(value),
            done: Mutex::new(true),
            cond: Condvar::new(),
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
        let mut done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        *done = true;
        drop(done);
        self.cond.notify_all();
        Ok(())
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
            let mut done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
            while !*done {
                done = self.cond.wait(done).unwrap_or_else(PoisonError::into_inner);
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
        let done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        if !*done {
            drop(
                self.cond
                    .wait_timeout(done, ASSIST_PARK)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
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

    #[test]
    fn cell_is_send_and_sync_for_shareable_values() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompletionCell<u32>>();
    }
}
