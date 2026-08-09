// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

use splot_core::write::WriteError;
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

    /// Coefficient-tokenization allocation failed.
    #[error("failed to allocate encoder coefficient tokenization storage for {context}")]
    CoefficientTokenizationAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// Intra-mode emission allocation failed.
    #[error("failed to allocate encoder intra-mode emission storage for {context}")]
    IntraModeEmissionAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// Block-symbol trace allocation failed.
    #[error("failed to allocate encoder block-symbol trace storage for {context}")]
    BlockSymbolTraceAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// Writing a block-symbol trace token through the symbol encoder failed.
    #[error("encoder block-symbol trace symbol write failed for token {index}: {source}")]
    BlockSymbolTraceSymbolWrite {
        /// Zero-based index of the failing token in the trace.
        index: usize,
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Finalizing block-symbol trace symbol bytes failed.
    #[error("encoder block-symbol trace symbol encoder finalization failed: {source}")]
    BlockSymbolTraceSymbolEncodeFinish {
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// An encoder lifecycle operation is invalid in the current context state.
    #[error("encoder operation {operation:?} is invalid while the context is {state:?}")]
    State {
        /// Requested operation.
        operation: EncoderOperation,
        /// Current context state.
        state: EncoderState,
    },

    /// Assembling the minimal intra skip IVF container failed.
    #[error("encoder minimal intra skip IVF assembly failed: {source}")]
    MinimalIntraSkipIvf {
        /// Source container-assembly error from the `splot-core` writer bridge.
        #[source]
        source: splot_core::headers::frame::MinimalIntraIvfError,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder context operations.
pub type Result<T> = core::result::Result<T, Error>;
