// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A decode-driver context scaffold that owns a worker pool but reads no bytes.

use core::num::NonZeroUsize;

use splot_parallel::{ThreadCount, WorkerPool};

use crate::error::Result;
use crate::runtime::DecodeRuntimeConfig;

/// A scaffold decode context.
///
/// It owns exactly one [`WorkerPool`] and exposes the resolved worker count,
/// but it intentionally does NOT read bitstream bytes, inspect input/output
/// paths, allocate decoded frames, or invoke any external decoder yet. Runtime
/// decode support remains unimplemented (`splot decode` still emits the stable
/// `decode/unsupported-feature` diagnostic).
#[derive(Debug)]
pub struct DecodeContext {
    runtime: DecodeRuntimeConfig,
    pool: WorkerPool,
}

impl DecodeContext {
    /// Creates a decode context and its single owned worker pool.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError::Pool`] if the worker pool cannot be built.
    pub fn new(runtime: DecodeRuntimeConfig) -> Result<Self> {
        let pool = WorkerPool::new(runtime.thread_count)?;
        Ok(Self { runtime, pool })
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &DecodeRuntimeConfig {
        &self.runtime
    }

    /// The originally requested (unresolved) thread-count policy.
    #[must_use]
    pub fn requested_threads(&self) -> ThreadCount {
        self.runtime.thread_count
    }

    /// The resolved, non-zero worker-thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.pool.threads()
    }

    /// The context's single owned worker pool.
    #[must_use]
    pub fn pool(&self) -> &WorkerPool {
        &self.pool
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_config_is_auto() {
        assert_eq!(
            DecodeRuntimeConfig::default().thread_count,
            ThreadCount::Auto
        );
    }

    #[test]
    fn context_resolves_fixed_thread_count() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
        assert_eq!(ctx.threads().get(), 4);
    }

    #[test]
    fn requested_threads_round_trips() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
        assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
    }

    #[test]
    fn zero_threads_maps_to_auto() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(0usize))).unwrap();
        assert!(ctx.threads().get() >= 1);
    }
}
