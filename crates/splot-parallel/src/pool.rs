// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The owned, local Rayon worker pool ([`WorkerPool`]) for data-parallel
//! loops outside the decoder.

use core::num::NonZeroUsize;
use std::sync::Arc;

use rayon::ThreadPool;

use crate::error::ParallelError;
use crate::thread_count::ThreadCount;

/// The stack each worker thread gets: recursive partition walks need more
/// than Rayon's default.
pub(crate) const WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// An owned Rayon pool sized by a [`ThreadCount`].
#[derive(Clone, Debug)]
pub struct WorkerPool {
    inner: Arc<ThreadPool>,
    threads: NonZeroUsize,
}

impl WorkerPool {
    /// Builds the pool with `thread_count` resolved against the machine.
    ///
    /// # Errors
    /// Returns [`ParallelError::PoolBuild`] when Rayon cannot start the pool.
    pub fn new(thread_count: ThreadCount) -> Result<Self, ParallelError> {
        let threads = thread_count.resolve();
        let inner = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.get())
            .thread_name(|index| format!("splot-worker-{index}"))
            .stack_size(WORKER_STACK_SIZE_BYTES)
            .build()?;
        Ok(Self {
            inner: Arc::new(inner),
            threads,
        })
    }

    /// The resolved worker count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// Runs `f` on the pool, so parallel iterators inside it use this pool.
    pub fn install<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        self.inner.install(f)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::prelude::*;

    #[test]
    fn install_runs_parallel_iterators_on_the_owned_pool() {
        let pool = WorkerPool::new(ThreadCount::from(3usize)).unwrap();
        assert_eq!(pool.threads().get(), 3);
        let sum: usize = pool.install(|| (0..1000usize).into_par_iter().sum());
        assert_eq!(sum, 499_500);
    }
}
