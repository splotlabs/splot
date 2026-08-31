// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The owned, local Rayon worker pool ([`WorkerPool`]).
use core::num::NonZeroUsize;
use core::time::Duration;
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Weak};

use rayon::{ThreadPool, Yield};

use crate::error::ParallelError;
use crate::progress::{PoolProgressBindings, PoolProgressEvent};
use crate::thread_count::ThreadCount;

const WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
const OFF_POOL_ASSIST_PARK: Duration = Duration::from_micros(100);

std::thread_local! {
    static ON_SPLOT_WORKER: Cell<bool> = const { Cell::new(false) };
    static INSTALLED_POOL: RefCell<Weak<ThreadPool>> = const { RefCell::new(Weak::new()) };
    static INSTALLED_PROGRESS: RefCell<Weak<PoolProgressEvent>> = const { RefCell::new(Weak::new()) };
}

/// A codec context's single owned worker pool, wrapping a *local* Rayon
/// [`ThreadPool`]. The global Rayon pool is never used.
#[derive(Clone, Debug)]
pub struct WorkerPool {
    inner: Arc<ThreadPool>,
    progress: Arc<PoolProgressEvent>,
    threads: NonZeroUsize,
}

/// A borrowed scope for nonblocking tasks that are ready to run.
pub struct TaskScope<'handle, 'scope> {
    inner: &'handle rayon::ScopeFifo<'scope>,
}

impl<'scope> TaskScope<'_, 'scope> {
    /// Spawns a ready task. The task may spawn successors into the same scope.
    pub fn spawn<F>(&self, task: F)
    where
        F: for<'next> FnOnce(&TaskScope<'next, 'scope>) + Send + 'scope,
    {
        self.inner
            .spawn_fifo(move |inner| task(&TaskScope { inner }));
    }
}

impl WorkerPool {
    /// Builds a local Rayon pool for the requested [`ThreadCount`].
    ///
    /// Worker threads are named `splot-worker-{i}`. The global Rayon pool and
    /// `build_global` are never used.
    ///
    /// # Errors
    /// Returns [`ParallelError::PoolBuild`] if the Rayon pool cannot be built.
    pub fn new(thread_count: ThreadCount) -> Result<Self, ParallelError> {
        let threads = thread_count.resolve();
        let progress = Arc::new(PoolProgressEvent::new());
        let inner = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.get())
            .thread_name(|index| format!("splot-worker-{index}"))
            .start_handler(|_| ON_SPLOT_WORKER.with(|active| active.set(true)))
            .stack_size(WORKER_STACK_SIZE_BYTES)
            .build()?;
        let inner = Arc::new(inner);
        let installed = Arc::downgrade(&inner);
        let installed_progress = Arc::downgrade(&progress);
        inner.broadcast(|_| {
            INSTALLED_POOL.with(|current| current.replace(Weak::clone(&installed)));
            INSTALLED_PROGRESS.with(|current| current.replace(Weak::clone(&installed_progress)));
        });
        Ok(Self {
            inner,
            progress,
            threads,
        })
    }

    /// The resolved, non-zero worker-thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// Runs `f` inside this local pool, so any nested Rayon work uses these
    /// workers (never the global pool). Nested parallel iterators are fine;
    /// nested *pools* are not.
    ///
    /// # Examples
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use rayon::prelude::*;
    /// use splot_parallel::{ThreadCount, WorkerPool};
    ///
    /// let pool = WorkerPool::new(ThreadCount::Fixed(4.try_into()?))?;
    /// let parallel: Vec<u64> = pool.install(|| (0..8u64).into_par_iter().map(|n| n * n).collect());
    /// let sequential: Vec<u64> = (0..8u64).map(|n| n * n).collect();
    /// assert_eq!(parallel, sequential);
    /// # Ok(())
    /// # }
    /// ```
    pub fn install<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        self.progress.install_submitted();
        let progress = &self.progress;
        self.inner.install(move || {
            progress.install_started();
            f()
        })
    }
}

/// Returns whether the calling thread belongs to a splot-owned worker pool.
///
/// Unlike Rayon's worker-index query, this stays false on unrelated local or
/// global Rayon pools, so codec helpers can avoid accidentally scheduling work
/// outside their owning [`WorkerPool`].
#[must_use]
pub fn on_worker_pool() -> bool {
    ON_SPLOT_WORKER.get()
}

/// Runs ready tasks on the current splot worker pool and waits for completion.
///
/// # Errors
/// Returns [`ParallelError::NotOnWorkerPool`] outside [`WorkerPool::install`].
pub fn ready_task_scope<'scope, F, R>(f: F) -> Result<R, ParallelError>
where
    F: for<'handle> FnOnce(&TaskScope<'handle, 'scope>) -> R + Send,
    R: Send,
{
    let pool = INSTALLED_POOL
        .with(|installed| installed.borrow().upgrade())
        .ok_or(ParallelError::NotOnWorkerPool)?;
    Ok(pool.scope_fifo(|inner| f(&TaskScope { inner })))
}

/// One pool-progress generation captured before a driver checks its condition.
///
/// Pass the snapshot to [`assist_pool_or_park`] only after checking the waited
/// condition. A publication between the snapshot and the wait advances the
/// generation, so the wait is skipped instead of losing the notification.
pub struct PoolProgressSnapshot {
    progress: Option<Arc<PoolProgressEvent>>,
    generation: u64,
}

/// Captures the current decoder pool's progress generation.
///
/// The pipeline driver must call this before checking the condition it may
/// wait for. Outside a splot-owned pool the returned snapshot preserves the
/// legacy off-pool fallback in [`assist_pool_or_park`].
#[must_use]
pub fn pool_progress_snapshot() -> PoolProgressSnapshot {
    let progress = INSTALLED_PROGRESS.with(|installed| installed.borrow().upgrade());
    let generation = progress.as_deref().map_or(0, PoolProgressEvent::generation);
    PoolProgressSnapshot {
        progress,
        generation,
    }
}

/// What one [`assist_installed_pool`] attempt did.
pub(crate) enum PoolAssist {
    /// One pending pool job ran to completion on the calling thread.
    Executed,
    /// The installed pool had no job available to run.
    Idle,
    /// The caller is not a worker of an installed pool, so nothing ran.
    OffPool,
}

/// Runs at most one pending job of the calling thread's installed pool.
///
/// The job is taken from the pool the caller belongs to, never Rayon's global
/// registry: a caller outside [`WorkerPool::install`] gets
/// [`PoolAssist::OffPool`] and runs nothing. The executed job is arbitrary and
/// may itself nest further pool work, so callers must be reentrancy-safe.
pub(crate) fn assist_installed_pool() -> PoolAssist {
    let Some(pool) = INSTALLED_POOL.with(|installed| installed.borrow().upgrade()) else {
        return PoolAssist::OffPool;
    };
    match pool.yield_now() {
        Some(Yield::Executed) => PoolAssist::Executed,
        Some(Yield::Idle) => PoolAssist::Idle,
        None => PoolAssist::OffPool,
    }
}

pub(crate) fn assist_installed_pool_or_wait(snapshot: &PoolProgressSnapshot) -> PoolAssist {
    let assisted = assist_installed_pool();
    match assisted {
        PoolAssist::Idle => {
            if let Some(progress) = snapshot.progress.as_deref() {
                if progress.has_pending_install() {
                    std::thread::yield_now();
                } else {
                    progress.wait_if_unchanged(snapshot.generation);
                }
            }
        }
        PoolAssist::Executed | PoolAssist::OffPool => {}
    }
    assisted
}

pub(crate) fn notify_installed_pool_progress() {
    if let Some(progress) = INSTALLED_PROGRESS.with(|installed| installed.borrow().upgrade()) {
        progress.notify();
    }
}

pub(crate) fn bind_installed_pool_progress(bound: &PoolProgressBindings) {
    INSTALLED_PROGRESS.with(|installed| bound.bind(&installed.borrow()));
}

pub(crate) fn notify_bound_pool_progress(bound: &PoolProgressBindings) {
    bind_installed_pool_progress(bound);
    bound.notify();
}

/// Runs one pending job of the driver's installed pool, or waits for its next
/// progress event when the pool has nothing to run.
///
/// Like [`crate::CompletionCell::wait_with_pool_assist`] this is reserved for
/// the pipeline driver thread and carries the same reentrancy contract: the
/// executed job is arbitrary, so the caller must hold no lock, no thread-local
/// scope guard, and no borrow such a job could need. A driver waiting on
/// something no completion cell publishes — a reference frame's row watermark,
/// say — captures [`pool_progress_snapshot`], tests its own condition, and then
/// passes the snapshot here. Watermark, completion, and ready-task publication
/// advance that generation before notifying the pool-scoped condition variable.
/// The next loop iteration re-tests both the condition and the pool.
pub fn assist_pool_or_park(snapshot: &PoolProgressSnapshot) {
    match assist_installed_pool_or_wait(snapshot) {
        PoolAssist::Executed | PoolAssist::Idle => (),
        PoolAssist::OffPool => std::thread::sleep(OFF_POOL_ASSIST_PARK),
    }
}

/// Runs one pending job of the driver's installed pool, reporting whether one
/// ran, and never parks when the pool has nothing queued.
///
/// This is the nonblocking half of [`assist_pool_or_park`] and carries the same
/// reentrancy contract: the executed job is arbitrary, so the caller must hold
/// no lock, no thread-local scope guard, and no borrow such a job could need.
/// It suits a driver that has work of its own to fall back on: `false` means
/// the pool had nothing to run, so the driver's own work is what is left to do.
#[must_use]
pub fn assist_pool_once() -> bool {
    matches!(assist_installed_pool(), PoolAssist::Executed)
}

/// Returns whether the calling thread is a worker of an installed pool that
/// has more than one thread.
///
/// Codec helpers that are usually driven inside [`WorkerPool::install`] but are
/// also callable directly (for example from tests) use this to take their
/// parallel path only when the context's workers are actually in scope and
/// parallelism can help, so a bare call never falls back to Rayon's global
/// pool and a one-thread pool never pays work-splitting overhead.
#[must_use]
pub fn on_multiworker_pool() -> bool {
    on_worker_pool() && rayon::current_num_threads() > 1
}

/// The worker count of the installed pool the caller runs on (`1` when the
/// caller is not a pool worker).
///
/// Parallel stages use this to choose a work-unit grain that yields a few
/// units per worker instead of thousands of tiny tasks.
///
/// A retention bound built on this scales with the pool only for calls made on
/// a worker; a caller outside the pool reads `1` and so gets the bound's floor.
#[must_use]
pub fn current_pool_width() -> usize {
    if rayon::current_thread_index().is_some() {
        rayon::current_num_threads()
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use rayon::prelude::*;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn fixed_pool_has_requested_threads() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(4))).unwrap();
        assert_eq!(pool.threads().get(), 4);
    }

    #[test]
    fn auto_pool_has_at_least_one_thread() {
        let pool = WorkerPool::new(ThreadCount::Auto).unwrap();
        assert!(pool.threads().get() >= 1);
    }

    #[test]
    fn install_runs_closure() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        assert_eq!(pool.install(|| 21 * 2), 42);
    }

    #[test]
    fn on_multiworker_pool_tracks_install_scope_and_width() {
        assert!(!on_multiworker_pool());
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        assert!(pool.install(on_multiworker_pool));
        let single = WorkerPool::new(ThreadCount::Fixed(nz(1))).unwrap();
        assert!(!single.install(on_multiworker_pool));
    }

    #[test]
    fn on_worker_pool_rejects_unrelated_rayon_pool() {
        assert!(!on_worker_pool());
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(1))).unwrap();
        assert!(pool.install(on_worker_pool));
        let unrelated = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        assert!(!unrelated.install(on_worker_pool));
    }

    #[test]
    fn ready_task_scope_is_available_on_every_owned_worker() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        let results = pool.inner.broadcast(|_| ready_task_scope(|_| ()));
        assert!(results.into_iter().all(|result| result.is_ok()));
    }

    #[test]
    fn install_runs_work_on_local_pool() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        let all_on_pool = pool.install(|| {
            (0..256u64)
                .into_par_iter()
                .map(|_| {
                    let indexed = rayon::current_thread_index().is_some();
                    let named = std::thread::current()
                        .name()
                        .is_some_and(|name| name.starts_with("splot-worker-"));
                    indexed && named
                })
                .all(|on_pool| on_pool)
        });
        assert!(all_on_pool);
    }

    #[test]
    fn ready_tasks_borrow_mutable_state_and_complete() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        let mut values = [1, 2, 3, 4];
        let result = pool.install(|| {
            ready_task_scope(|scope| {
                for value in &mut values {
                    scope.spawn(move |_| *value *= 2);
                }
                42
            })
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(values, [2, 4, 6, 8]);
    }

    fn spawn_successors<'scope>(
        scope: &TaskScope<'_, 'scope>,
        visits: &'scope AtomicUsize,
        remaining: usize,
    ) {
        scope.spawn(move |scope| {
            visits.fetch_add(1, Ordering::Relaxed);
            if remaining > 0 {
                spawn_successors(scope, visits, remaining - 1);
            }
        });
    }

    #[test]
    fn ready_tasks_spawn_successors_on_owned_pool() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        let visits = AtomicUsize::new(0);
        let all_on_pool = AtomicBool::new(true);
        pool.install(|| {
            ready_task_scope(|scope| {
                spawn_successors(scope, &visits, 7);
                scope.spawn(|_| {
                    all_on_pool.store(on_worker_pool(), Ordering::Relaxed);
                });
            })
            .unwrap();
        });
        assert_eq!(visits.load(Ordering::Relaxed), 8);
        assert!(all_on_pool.load(Ordering::Relaxed));
    }

    #[test]
    fn pool_assist_runs_a_queued_job_on_the_calling_worker() {
        assert!(matches!(assist_installed_pool(), PoolAssist::OffPool));
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(1))).unwrap();
        let ran = AtomicBool::new(false);
        let (idle, executed) = pool.install(|| {
            let idle = matches!(assist_installed_pool(), PoolAssist::Idle);
            let executed = ready_task_scope(|scope| {
                scope.spawn(|_| ran.store(true, Ordering::Relaxed));
                matches!(assist_installed_pool(), PoolAssist::Executed)
            })
            .unwrap();
            (idle, executed)
        });
        assert!(idle, "an empty pool has no job to assist with");
        assert!(executed, "a queued job must run on the assisting worker");
        assert!(ran.load(Ordering::Relaxed));
    }

    #[test]
    fn ready_task_scope_rejects_unowned_thread() {
        assert!(matches!(
            ready_task_scope(|_| ()),
            Err(ParallelError::NotOnWorkerPool)
        ));
    }
}
