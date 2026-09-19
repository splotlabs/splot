// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot for pipeline hand-off.
use parking_lot::{Condvar, Mutex};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::admission::{Waiter, WeakWaiter};
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
    first_waiter: OnceLock<WeakWaiter>,
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
    additional_waiters: Vec<WeakWaiter>,
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

    /// Borrows the value exclusively while no reader can access this cell.
    pub fn get_mut(&mut self) -> Option<&mut V> {
        self.value.get_mut()
    }

    /// Clears a retired publication while exclusive access excludes every reader.
    pub fn reset(&mut self) {
        self.value.take();
        self.first_waiter.take();
        *self.admission_state.get_mut() = EMPTY;
        if let Some(wait) = self.wait.get_mut() {
            let state = wait.state.get_mut();
            state.parked = 0;
            state.additional_waiters.clear();
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

    /// Publishes a value and wakes every observer.
    ///
    /// # Errors
    /// Returns the given value unchanged if the cell was already set.
    pub fn set(&self, value: V) -> Result<(), V> {
        self.value.set(value)?;
        notify_bound_pool_progress(&self.progress);
        if self.admission_state.swap(SET, Ordering::AcqRel) == REGISTERED
            && let Some(waiter) = self.first_waiter.get().and_then(WeakWaiter::upgrade)
        {
            waiter.satisfy();
        }
        let Some(wait) = self.wait.get() else {
            return Ok(());
        };
        let mut state = wait.state.lock();
        if state.parked != 0 {
            wait.cond.notify_all();
        }
        for waiter in state.additional_waiters.drain(..) {
            if let Some(waiter) = waiter.upgrade() {
                waiter.satisfy();
            }
        }
        Ok(())
    }

    pub(crate) fn register_waiter(&self, waiter: Waiter) -> bool {
        bind_installed_pool_progress(&self.progress);
        if self.is_set() {
            return false;
        }
        let weak = waiter.downgrade();
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
        let mut state = wait.state.lock();
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

    /// Takes the completed value, or `None` when nothing was ever published.
    #[must_use]
    pub fn into_inner(self) -> Option<V> {
        self.value.into_inner()
    }

    /// Reports whether the cell is set.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.value.get().is_some()
    }

    /// Runs pool jobs while the pipeline driver waits for the value.
    #[must_use]
    pub fn wait_with_pool_assist(&self) -> &V {
        self.wait_with_assist(|| false)
    }

    /// Runs ready work supplied by the driver before assisting or parking its pool.
    #[must_use]
    pub fn wait_with_assist(&self, mut assist: impl FnMut() -> bool) -> &V {
        bind_installed_pool_progress(&self.progress);
        loop {
            let progress = pool_progress_snapshot();
            if let Some(value) = self.value.get() {
                return value;
            }
            if assist() {
                continue;
            }
            match assist_installed_pool_or_wait(&progress) {
                PoolAssist::Executed | PoolAssist::Idle => {}
                PoolAssist::OffPool => {
                    let wait = self.wait_state();
                    let mut state = wait.state.lock();
                    while self.value.get().is_none() {
                        state.parked += 1;
                        wait.cond.wait(&mut state);
                        state.parked -= 1;
                    }
                }
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
    use std::sync::Arc;

    use super::*;
    use crate::admission::{AdmissionScheduler, Condition};
    use crate::pool::{WorkerPool, ready_task_scope};
    use crate::thread_count::ThreadCount;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn reset_detaches_previous_admission_waiters() {
        let mut cell = CompletionCell::new();
        let visits = AtomicUsize::new(0);
        let old: AdmissionScheduler<'_, crate::NoTask> = AdmissionScheduler::new();
        let new: AdmissionScheduler<'_, crate::NoTask> = AdmissionScheduler::new();
        WorkerPool::new(ThreadCount::from(12usize))
            .unwrap()
            .install(|| {
                ready_task_scope(|scope| {
                    for index in 0..12 {
                        old.submit(
                            scope,
                            index,
                            &[Condition::completion(&cell)],
                            crate::Job::Boxed(Box::new(|_| {
                                visits.fetch_add(100, Ordering::Relaxed);
                            })),
                        );
                    }
                    cell.reset();
                    assert_eq!(cell.get(), None);
                    new.submit(
                        scope,
                        0,
                        &[Condition::completion(&cell)],
                        crate::Job::Boxed(Box::new(|_| {
                            visits.fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                    cell.set(7).unwrap();
                    old.admit_ready(scope);
                    new.admit_ready(scope);
                })
                .unwrap();
            });
        assert_eq!(visits.load(Ordering::Relaxed), 1);
        assert!(old.finish().is_err());
        new.finish().unwrap();
        cell.reset();
        assert_eq!(cell.set(8), Ok(()));
    }

    #[test]
    fn cell_is_write_once() {
        let cell = CompletionCell::new();
        assert!(!cell.is_set());
        assert_eq!(cell.set(7), Ok(()));
        assert_eq!(cell.get(), Some(&7));
        assert_eq!(cell.set(8), Err(8));
    }

    #[test]
    fn set_wakes_multiple_admission_and_blocking_waiters() {
        let cell = Arc::new(CompletionCell::new());
        let visits: Vec<_> = (0..2).map(|_| AtomicUsize::new(0)).collect();
        let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> = AdmissionScheduler::new();
        let pool = WorkerPool::new(ThreadCount::Fixed(2.try_into().unwrap())).unwrap();
        pool.install(|| {
            ready_task_scope(|scope| {
                for visit in &visits {
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::completion(cell.as_ref())],
                        crate::admission::Job::Boxed(Box::new(move |_| {
                            visit.fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                }
            })
            .unwrap();
        });
        let blocked: Vec<_> = (0..2)
            .map(|_| {
                let cell = Arc::clone(&cell);
                std::thread::spawn(move || *cell.wait_with_pool_assist())
            })
            .collect();
        cell.set(11).unwrap();
        pool.install(|| {
            ready_task_scope(|scope| scheduler.admit_ready(scope)).unwrap();
        });
        assert!(
            visits
                .iter()
                .all(|visit| visit.load(Ordering::Acquire) == 1)
        );
        for waiter in blocked {
            assert_eq!(waiter.join().unwrap(), 11);
        }
        scheduler.finish().unwrap();
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
        let pool = WorkerPool::new(ThreadCount::Fixed(2.try_into().unwrap())).unwrap();
        for _ in 0..128 {
            let cell = Arc::new(CompletionCell::new());
            let ran = AtomicUsize::new(0);
            let scheduler: AdmissionScheduler<'_, crate::admission::NoTask> =
                AdmissionScheduler::new();
            let setting = Arc::clone(&cell);
            let setter = std::thread::spawn(move || setting.set(()).unwrap());
            pool.install(|| {
                ready_task_scope(|scope| {
                    scheduler.submit(
                        scope,
                        0,
                        &[Condition::completion(cell.as_ref())],
                        crate::admission::Job::Boxed(Box::new(|_| {
                            ran.fetch_add(1, Ordering::Relaxed);
                        })),
                    );
                    setter.join().unwrap();
                    scheduler.admit_ready(scope);
                })
                .unwrap();
            });
            assert_eq!(ran.load(Ordering::Acquire), 1);
            scheduler.finish().unwrap();
        }
    }
}
