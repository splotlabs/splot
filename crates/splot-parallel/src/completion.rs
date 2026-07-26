// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot ([`CompletionCell`]) for pipeline hand-off.
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

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
/// Only the pipeline driver thread may block in [`CompletionCell::wait`].
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
    fn cell_is_send_and_sync_for_shareable_values() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompletionCell<u32>>();
    }
}
