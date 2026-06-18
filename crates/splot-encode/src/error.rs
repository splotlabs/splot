// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

use splot_recon::{PlaneId, PlaneSize, ReconError};

use crate::config::{BitDepth, ChromaSubsampling};
use crate::context::{EncoderOperation, EncoderState};

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

    /// Input frame bit depth is outside the current supported subset.
    #[error("unsupported encoder input bit depth {bit_depth:?}; only 8-bit input is supported")]
    UnsupportedInputBitDepth {
        /// Requested input bit depth.
        bit_depth: BitDepth,
    },

    /// Input frame chroma layout is outside the current supported subset.
    #[error(
        "unsupported encoder input chroma layout {chroma_subsampling:?}; only YUV420 input is supported"
    )]
    UnsupportedInputChromaSubsampling {
        /// Requested input chroma layout.
        chroma_subsampling: ChromaSubsampling,
    },

    /// A required input plane is absent.
    #[error("missing required encoder input plane {plane:?}")]
    MissingInputPlane {
        /// Missing plane.
        plane: PlaneId,
    },

    /// A plane was supplied where the current format has no plane.
    #[error("unexpected encoder input plane {plane:?}")]
    UnexpectedInputPlane {
        /// Unexpected plane.
        plane: PlaneId,
    },

    /// A borrowed input plane failed geometry validation.
    #[error("invalid encoder input plane {plane:?}: {source}")]
    InputPlane {
        /// Failing plane.
        plane: PlaneId,
        /// Underlying reconstruction view validation error.
        #[source]
        source: ReconError,
    },

    /// A borrowed input plane's visible size does not match frame metadata.
    #[error("encoder input plane {plane:?} has visible size {actual:?}; expected {expected:?}")]
    InputPlaneSizeMismatch {
        /// Failing plane.
        plane: PlaneId,
        /// Expected visible size.
        expected: PlaneSize,
        /// Actual visible size.
        actual: PlaneSize,
    },

    /// Input frame dimensions do not match the encoder configuration.
    #[error(
        "encoder input frame has visible luma size {actual:?}; expected {expected_width}x{expected_height}"
    )]
    InputFrameSizeMismatch {
        /// Configured frame width in luma samples.
        expected_width: u32,
        /// Configured frame height in luma samples.
        expected_height: u32,
        /// Actual input frame visible luma size.
        actual: PlaneSize,
    },

    /// Input frame bit depth does not match the encoder configuration.
    #[error("encoder input frame bit depth {actual:?} does not match configured {expected:?}")]
    InputFrameBitDepthMismatch {
        /// Configured bit depth.
        expected: BitDepth,
        /// Actual input frame bit depth.
        actual: BitDepth,
    },

    /// Input frame chroma layout does not match the encoder configuration.
    #[error("encoder input frame chroma layout {actual:?} does not match configured {expected:?}")]
    InputFrameChromaSubsamplingMismatch {
        /// Configured chroma layout.
        expected: ChromaSubsampling,
        /// Actual input frame chroma layout.
        actual: ChromaSubsampling,
    },

    /// Chroma-size derivation failed for the input metadata.
    #[error("failed to derive encoder input chroma geometry: {source}")]
    InputChromaGeometry {
        /// Underlying chroma geometry error.
        #[source]
        source: ReconError,
    },

    /// A requested encoder residual block fell outside the input plane's visible area.
    #[error(
        "encoder residual block {block:?} for plane {plane:?} exceeds visible size {visible_size:?}"
    )]
    ResidualBlockOutOfBounds {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Visible plane size used for the bounds check.
        visible_size: PlaneSize,
    },

    /// A prediction row stride was too small for a residual block.
    #[error(
        "encoder residual prediction stride {stride_samples} for plane {plane:?} is smaller than block width {width}"
    )]
    ResidualPredictionStrideTooSmall {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Supplied prediction row stride in samples.
        stride_samples: usize,
        /// Required block width in samples.
        width: usize,
    },

    /// A prediction buffer was too small for a residual block.
    #[error(
        "encoder residual prediction for plane {plane:?} needs at least {expected} samples, got {actual}"
    )]
    ResidualPredictionLengthMismatch {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Minimum required prediction sample count.
        expected: usize,
        /// Actual supplied prediction sample count.
        actual: usize,
    },

    /// Residual prediction span arithmetic overflowed.
    #[error(
        "encoder residual prediction span overflowed for plane {plane:?}, block {block:?}, stride {stride_samples}"
    )]
    ResidualPredictionSpanOverflow {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Supplied prediction row stride in samples.
        stride_samples: usize,
    },

    /// Residual block sample-count arithmetic overflowed.
    #[error("encoder residual block sample count overflowed for plane {plane:?}, block {block:?}")]
    ResidualSampleCountOverflow {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
    },

    /// Residual block allocation failed.
    #[error("failed to allocate encoder residual storage for plane {plane:?}: {context}")]
    ResidualAllocationFailed {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// An encoder lifecycle operation is invalid in the current context state.
    #[error("encoder operation {operation:?} is invalid while the context is {state:?}")]
    State {
        /// Requested operation.
        operation: EncoderOperation,
        /// Current context state.
        state: EncoderState,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder context operations.
pub type Result<T> = core::result::Result<T, Error>;
