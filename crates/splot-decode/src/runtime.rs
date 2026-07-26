// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Non-bitstream runtime configuration for the decode driver.

use splot_parallel::{FrameDelay, ThreadCount};

/// Runtime (non-bitstream) decode knobs.
///
/// Carries the worker-thread policy and the frame-pipelining depth; it does not
/// read or decode any bitstream bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecodeRuntimeConfig {
    /// Worker-thread policy. Defaults to [`ThreadCount::Auto`].
    pub thread_count: ThreadCount,
    /// Frame-pipelining depth policy. Defaults to [`FrameDelay::Auto`], which
    /// resolves to the pool's worker-thread count. A resolved depth of one
    /// decodes serially.
    pub frame_delay: FrameDelay,
}

impl DecodeRuntimeConfig {
    /// Builds a runtime config with the given thread-count policy.
    #[must_use]
    pub fn new(thread_count: ThreadCount) -> Self {
        Self {
            thread_count,
            frame_delay: FrameDelay::Auto,
        }
    }

    /// Returns the config with the given frame-pipelining depth policy.
    #[must_use]
    pub fn with_frame_delay(mut self, frame_delay: FrameDelay) -> Self {
        self.frame_delay = frame_delay;
        self
    }
}
