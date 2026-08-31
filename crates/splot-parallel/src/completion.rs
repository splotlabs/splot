// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot for pipeline hand-off.
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};

use crate::admission::AdmissionWaiter;
use crate::pool::{
    PoolAssist, assist_installed_pool_or_wait, bind_installed_pool_progress,
    notify_bound_pool_progress, pool_progress_snapshot,
};
use crate::progress::PoolProgressBindings;

const EMPTY: u8 = 0;
const REGISTERED: u8 = 1;
const SET: u8 = 2;

/// A write-once slot that a pipeline driver can wait on.
///
/// Pool jobs must never wait: a worker could block while its producer needs
/// that same worker. Drivers may use [`CompletionCell::wait_with_pool_assist`]
/// to execute pool work until the value arrives.
#[derive(Debug)]
pub struct CompletionCell<V> {
    value: OnceLock<V>,
    progress: PoolProgressBindings,
    first_waiter: OnceLock<Weak<dyn AdmissionWaiter>>,
    admission_state: AtomicU8,
    wait: OnceLock<CompletionWait>,
}

#[derive(Debug)]
struct CompletionWait {
    state: Mutex<WaitState>,
    cond: Condvar,
}

#[derive(Debug)]
struct WaitState {
    parked: usize,
    additional_waiters: Vec<Weak<dyn AdmissionWaiter>>,
}

impl<V> CompletionCell<V> {
    /// Creates an empty cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            progress: PoolProgressBindings::new(),
            first_waiter: OnceLock::new(),
            admission_state: AtomicU8::new(EMPTY),
            wait: OnceLock::new(),
        }
    }

    /// Creates an already completed cell.
    #[must_use]
    pub fn completed(value: V) -> Self {
        Self {
            value: OnceLock::from(value),
            progress: PoolProgressBindings::new(),
            first_waiter: OnceLock::new(),
            admission_state: AtomicU8::new(SET),
            wait: OnceLock::new(),
        }
    }

    /// Consumes the cell and returns its value, if set.
    #[must_use]
    pub fn into_inner(self) -> Option<V> {
        self.value.into_inner()
    }

    /// Publishes a value and wakes every observer.
    ///
    /// # Errors
    /// Returns the given value unchanged if the cell was already set.
    pub fn set(&self, value: V) -> Result<(), V> {
        self.value.set(value)?;
        notify_bound_pool_progress(&self.progress);
        if self.admission_state.swap(SET, Ordering::AcqRel) == REGISTERED
            && let Some(waiter) = self.first_waiter.get().and_then(Weak::upgrade)
        {
            waiter.satisfy();
        }
        let Some(wait) = self.wait.get() else {
            return Ok(());
        };
        let (parked, waiters) = {
            let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
            (
                state.parked != 0,
                core::mem::take(&mut state.additional_waiters),
            )
        };
        if parked {
            wait.cond.notify_all();
        }
        for waiter in waiters {
            if let Some(waiter) = waiter.upgrade() {
                waiter.satisfy();
            }
        }
        Ok(())
    }

    pub(crate) fn register_waiter(&self, waiter: Arc<dyn AdmissionWaiter>) -> bool {
        bind_installed_pool_progress(&self.progress);
        if self.is_set() {
            return false;
        }
        let weak = Arc::downgrade(&waiter);
        drop(waiter);
        let weak = match self.first_waiter.set(weak) {
            Ok(()) => {
                return self
                    .admission_state
                    .compare_exchange(EMPTY, REGISTERED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok();
            }
            Err(weak) => weak,
        };
        let wait = self.wait_state();
        let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_set() {
            return false;
        }
        state.additional_waiters.push(weak);
        true
    }

    /// Returns the value without blocking.
    #[must_use]
    pub fn get(&self) -> Option<&V> {
        self.value.get()
    }

    /// Reports whether the cell is set.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.value.get().is_some()
    }

    /// Blocks the calling pipeline driver until the value arrives.
    #[must_use]
    pub fn wait(&self) -> &V {
        loop {
            if let Some(value) = self.value.get() {
                return value;
            }
            let wait = self.wait_state();
            let mut state = wait.state.lock().unwrap_or_else(PoisonError::into_inner);
            while self.value.get().is_none() {
                state.parked += 1;
                state = wait
                    .cond
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                state.parked -= 1;
            }
        }
    }

    /// Runs pool jobs while the pipeline driver waits for the value.
    #[must_use]
    pub fn wait_with_pool_assist(&self) -> &V {
        bind_installed_pool_progress(&self.progress);
        loop {
            let progress = pool_progress_snapshot();
            if let Some(value) = self.value.get() {
                return value;
            }
            match assist_installed_pool_or_wait(&progress) {
                PoolAssist::Executed | PoolAssist::Idle => {}
                PoolAssist::OffPool => return self.wait(),
            }
        }
    }

    fn wait_state(&self) -> &CompletionWait {
        self.wait.get_or_init(|| CompletionWait {
            state: Mutex::new(WaitState {
                parked: 0,
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
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::pool::{WorkerPool, ready_task_scope};
    use crate::thread_count::ThreadCount;
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug, Default)]
    struct Count(AtomicUsize);

    impl AdmissionWaiter for Count {
        fn satisfy(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    fn cell_is_write_once() {
        let cell = CompletionCell::new();
        assert!(!cell.is_set());
        assert_eq!(cell.set(7), Ok(()));
        assert_eq!(cell.get(), Some(&7));
        assert_eq!(cell.wait(), &7);
        assert_eq!(cell.set(8), Err(8));
        assert_eq!(cell.into_inner(), Some(7));
    }

    #[test]
    fn set_wakes_multiple_admission_and_blocking_waiters() {
        let cell = Arc::new(CompletionCell::new());
        let first = Arc::new(Count::default());
        let second = Arc::new(Count::default());
        assert!(cell.register_waiter(first.clone()));
        assert!(cell.register_waiter(second.clone()));
        let blocked: Vec<_> = (0..2)
            .map(|_| {
                let cell = Arc::clone(&cell);
                std::thread::spawn(move || *cell.wait())
            })
            .collect();
        cell.set(11).unwrap();
        assert_eq!(first.0.load(Ordering::Acquire), 1);
        assert_eq!(second.0.load(Ordering::Acquire), 1);
        for waiter in blocked {
            assert_eq!(waiter.join().unwrap(), 11);
        }
    }

    #[test]
    fn assisted_wait_runs_the_only_workers_producer() {
        let pool = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
        let cell = CompletionCell::new();
        let value = pool.install(|| {
            ready_task_scope(|scope| {
                scope.spawn(|_| {
                    cell.set(23).unwrap();
                });
                *cell.wait_with_pool_assist()
            })
            .unwrap()
        });
        assert_eq!(value, 23);
    }

    #[test]
    fn registration_racing_set_is_rejected_or_fired_once() {
        for _ in 0..128 {
            let cell = Arc::new(CompletionCell::new());
            let waiter = Arc::new(Count::default());
            let setting = Arc::clone(&cell);
            let setter = std::thread::spawn(move || setting.set(()).unwrap());
            let registered = cell.register_waiter(waiter.clone());
            setter.join().unwrap();
            assert_eq!(waiter.0.load(Ordering::Acquire), usize::from(registered));
        }
    }
}
