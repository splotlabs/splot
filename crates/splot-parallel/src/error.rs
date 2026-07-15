// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error types for the splot parallel-runtime primitives.

/// An error returned when constructing or using a [`crate::WorkerPool`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParallelError {
    /// The underlying Rayon thread pool could not be built.
    #[error("failed to build the splot worker pool: {source}")]
    PoolBuild {
        /// The Rayon build error.
        #[from]
        source: rayon::ThreadPoolBuildError,
    },
    /// A ready-task scope was started outside a splot worker.
    #[error("ready-task scope requires a splot worker pool")]
    NotOnWorkerPool,
}

/// An error returned when parsing a [`crate::ThreadCount`] from a string.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThreadCountParseError {
    /// The input was empty after trimming.
    #[error("thread count must not be empty; use `auto` or a positive integer")]
    Empty,
    /// The input was neither `auto`, `0`, nor a non-negative integer.
    #[error(
        "invalid thread count {input:?}: expected `auto`, `0` (alias for auto), or a positive integer"
    )]
    Invalid {
        /// The rejected input (trimmed).
        input: String,
    },
}
