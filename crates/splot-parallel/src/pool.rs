// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The owned, local Rayon worker pool ([`WorkerPool`]).
use core::num::NonZeroUsize;
use std::sync::Arc;

use rayon::ThreadPool;

use crate::error::ParallelError;
use crate::thread_count::ThreadCount;

const WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// A codec context's single owned worker pool, wrapping a *local* Rayon
/// [`ThreadPool`]. The global Rayon pool is never used.
#[derive(Clone, Debug)]
pub struct WorkerPool {
    inner: Arc<ThreadPool>,
    requested: ThreadCount,
    threads: NonZeroUsize,
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
        let inner = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.get())
            .thread_name(|index| format!("splot-worker-{index}"))
            .stack_size(WORKER_STACK_SIZE_BYTES)
            .build()?;
        Ok(Self {
            inner: Arc::new(inner),
            requested: thread_count,
            threads,
        })
    }

    /// The resolved, non-zero worker-thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// The originally requested (unresolved) [`ThreadCount`].
    #[must_use]
    pub fn requested(&self) -> ThreadCount {
        self.requested
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
        self.inner.install(f)
    }
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
    rayon::current_thread_index().is_some() && rayon::current_num_threads() > 1
}

/// Returns the index of the current Rayon worker when called inside an
/// installed worker-pool scope.
///
/// This is for diagnostics and attribution only. It does not spawn work and
/// must not be used to select codec semantics.
#[must_use]
pub fn current_worker_index() -> Option<usize> {
    rayon::current_thread_index()
}

/// Returns the number of workers in the current installed pool scope.
///
/// This is for diagnostics and attribution only. Outside a Rayon worker scope
/// it returns `None`.
#[must_use]
pub fn current_worker_count() -> Option<NonZeroUsize> {
    rayon::current_thread_index()?;
    NonZeroUsize::new(rayon::current_num_threads())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
    fn requested_round_trips() {
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        assert_eq!(pool.requested(), ThreadCount::Fixed(nz(2)));

        let auto = WorkerPool::new(ThreadCount::Auto).unwrap();
        assert_eq!(auto.requested(), ThreadCount::Auto);
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
    fn current_worker_metadata_tracks_install_scope() {
        assert_eq!(current_worker_index(), None);
        assert_eq!(current_worker_count(), None);
        let pool = WorkerPool::new(ThreadCount::Fixed(nz(2))).unwrap();
        let (index, count) = pool.install(|| (current_worker_index(), current_worker_count()));
        assert!(index.is_some_and(|index| index < 2));
        assert_eq!(count, Some(nz(2)));
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
}
