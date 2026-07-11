// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Non-bitstream runtime configuration for the decode driver.

use splot_parallel::ThreadCount;

/// Runtime (non-bitstream) decode knobs.
///
/// Carries only the worker-thread policy; it does not read or decode any
/// bitstream bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecodeRuntimeConfig {
    /// Worker-thread policy. Defaults to [`ThreadCount::Auto`].
    pub thread_count: ThreadCount,
}

impl DecodeRuntimeConfig {
    /// Builds a runtime config with the given thread-count policy.
    #[must_use]
    pub fn new(thread_count: ThreadCount) -> Self {
        Self { thread_count }
    }
}
