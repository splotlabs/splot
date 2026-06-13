// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Non-bitstream runtime configuration for the encoder.

use splot_parallel::ThreadCount;

/// Runtime (non-bitstream) encoder knobs.
///
/// This is deliberately separate from [`crate::EncoderConfig`], which holds
/// only bitstream-affecting settings. Thread count must never influence
/// bitstream output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncoderRuntimeConfig {
    /// Worker-thread policy. Defaults to [`ThreadCount::Auto`].
    pub thread_count: ThreadCount,
}

impl EncoderRuntimeConfig {
    /// Builds a runtime config with the given thread-count policy.
    #[must_use]
    pub fn new(thread_count: ThreadCount) -> Self {
        Self { thread_count }
    }
}
