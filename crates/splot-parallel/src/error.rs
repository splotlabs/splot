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
    /// Jobs submitted to a [`crate::AdmissionScheduler`] never became
    /// admissible: a condition source that never published (a producer that
    /// failed without publishing [`crate::WatermarkCell::FAILED`]) or a cycle in
    /// the caller's dependency graph.
    #[error(
        "{count} scheduled job(s) never became admissible (lowest order key {lowest_order_key})"
    )]
    JobsNeverAdmitted {
        /// How many submitted jobs were still waiting on a condition.
        count: usize,
        /// The lowest `order_key` among them, to locate the stalled stage.
        lowest_order_key: u64,
    },
}

/// The rejection reason from the shared `auto` / `0` / positive-integer grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AutoOrCountError {
    /// The input was empty after trimming.
    Empty,
    /// The trimmed input parsed as neither `auto` nor a `usize`.
    Invalid(String),
}

/// Parses the `auto` / `0` / positive-integer grammar shared by the policy
/// types, mapping `auto` onto the `0` alias their constructors already accept.
pub(crate) fn parse_auto_or_count(s: &str) -> Result<usize, AutoOrCountError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(AutoOrCountError::Empty);
    }
    if trimmed.eq_ignore_ascii_case("auto") {
        return Ok(0);
    }
    trimmed
        .parse::<usize>()
        .map_err(|_| AutoOrCountError::Invalid(trimmed.to_owned()))
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

impl From<AutoOrCountError> for ThreadCountParseError {
    fn from(error: AutoOrCountError) -> Self {
        match error {
            AutoOrCountError::Empty => Self::Empty,
            AutoOrCountError::Invalid(input) => Self::Invalid { input },
        }
    }
}

/// An error returned when parsing a [`crate::FrameDelay`] from a string.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameDelayParseError {
    /// The input was empty after trimming.
    #[error("frame delay must not be empty; use `auto` or a positive integer")]
    Empty,
    /// The input was neither `auto`, `0`, nor a non-negative integer.
    #[error(
        "invalid frame delay {input:?}: expected `auto`, `0` (alias for auto), or a positive integer"
    )]
    Invalid {
        /// The rejected input (trimmed).
        input: String,
    },
}

impl From<AutoOrCountError> for FrameDelayParseError {
    fn from(error: AutoOrCountError) -> Self {
        match error {
            AutoOrCountError::Empty => Self::Empty,
            AutoOrCountError::Invalid(input) => Self::Invalid { input },
        }
    }
}
