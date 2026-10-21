// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

use splot_core::write::WriteError;
use splot_recon::{BitDepth, PixelFormat};

/// An error from the splot encoder API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Frame dimensions are zero or larger than AV2 can signal.
    #[error("frame size {width}x{height} is outside 1..={max} per dimension", max = crate::config::MAX_FRAME_DIMENSION)]
    InvalidDimensions {
        /// Configured width.
        width: u32,
        /// Configured height.
        height: u32,
    },

    /// The chroma format is not supported.
    #[error("chroma format {chroma:?} is not supported; only 4:2:0 is")]
    UnsupportedChroma {
        /// Configured chroma format.
        chroma: PixelFormat,
    },

    /// A frame-rate term is zero.
    #[error("frame rate {numerator}:{denominator} must have nonzero terms")]
    InvalidFrameRate {
        /// Numerator.
        numerator: u32,
        /// Denominator.
        denominator: u32,
    },

    /// The constant quantizer is outside `1..=255`.
    #[error("qp {qp} is outside 1..=255")]
    InvalidQp {
        /// Requested `base_q_idx`.
        qp: u8,
    },

    /// The speed preset is outside `0..=10`.
    #[error("preset {preset} is outside 0..={max}", max = crate::preset::Preset::MAX)]
    InvalidPreset {
        /// Requested preset.
        preset: u8,
    },

    /// Input samples have a different bit depth than the configuration.
    #[error("input bit depth {actual:?} does not match the configured {expected:?}")]
    BitDepthMismatch {
        /// Configured bit depth.
        expected: BitDepth,
        /// Input bit depth.
        actual: BitDepth,
    },

    /// An input plane has the wrong number of samples.
    #[error("input plane {plane} needs {expected} samples, got {actual}")]
    PlaneSizeMismatch {
        /// Plane index: 0 = Y, 1 = U, 2 = V.
        plane: usize,
        /// Samples needed.
        expected: usize,
        /// Samples given.
        actual: usize,
    },

    /// An input plane stride is smaller than its width.
    #[error("input plane {plane} stride {stride} is smaller than its width {width}")]
    InvalidStride {
        /// Plane index: 0 = Y, 1 = U, 2 = V.
        plane: usize,
        /// Given stride.
        stride: usize,
        /// Plane width.
        width: usize,
    },

    /// An input sample exceeds the configured bit depth.
    #[error("input plane {plane} has sample {sample} above the maximum {max}")]
    SampleOutOfRange {
        /// Plane index: 0 = Y, 1 = U, 2 = V.
        plane: usize,
        /// Offending sample.
        sample: u16,
        /// Largest allowed sample.
        max: u16,
    },

    /// A frame was built for a different configuration than the encoder's.
    #[error("frame plane sizes or bit depth do not match the encoder configuration")]
    FrameConfigMismatch,

    /// A frame was sent after `flush`.
    #[error("send_frame was called after flush")]
    SendAfterFlush,

    /// An earlier coding error stopped the encoder.
    #[error("the encoder stopped after an earlier error")]
    Failed,

    /// A frame coder returned reconstruction planes of the wrong size.
    #[error("the frame coder returned reconstruction planes of the wrong size")]
    ReconSizeMismatch,

    /// The requested feature is not implemented.
    #[error("{what} is not supported yet")]
    Unsupported {
        /// The missing feature.
        what: &'static str,
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

    /// Assembling the minimal intra skip IVF container failed.
    #[error("encoder minimal intra skip IVF assembly failed: {source}")]
    MinimalIntraSkipIvf {
        /// Source container-assembly error from the `splot-core` writer bridge.
        #[source]
        source: splot_core::headers::frame::MinimalIntraIvfError,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder operations.
pub type Result<T> = core::result::Result<T, Error>;
