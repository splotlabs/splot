// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot decode driver scaffold.

/// An error from the splot decode driver scaffold.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The decode worker pool could not be constructed.
    #[error("failed to construct the decode worker pool: {source}")]
    Pool {
        /// The underlying parallel-runtime error.
        #[from]
        source: splot_parallel::ParallelError,
    },
}

/// A specialized [`Result`](core::result::Result) for decode context operations.
pub type Result<T> = core::result::Result<T, DecodeError>;
