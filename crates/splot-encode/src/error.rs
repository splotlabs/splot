// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

/// An error from the splot encoder API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The encoder worker pool could not be constructed.
    #[error("failed to construct the encoder worker pool: {source}")]
    Pool {
        /// The underlying parallel-runtime error.
        #[from]
        source: splot_parallel::ParallelError,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder context operations.
pub type Result<T> = core::result::Result<T, Error>;
