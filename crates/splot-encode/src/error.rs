// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

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

    /// The rate factor is outside `1..=63`.
    #[error("crf {crf} is outside 1..={max}", max = crate::rate_control::Crf::MAX)]
    InvalidCrf {
        /// Requested rate factor.
        crf: u8,
    },

    /// A bitrate setting is zero.
    #[error("{what} must be positive, got {value}")]
    InvalidBitrate {
        /// The setting.
        what: &'static str,
        /// Its value.
        value: u32,
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

    /// A frame-coding stage failed.
    #[error("frame coding failed at {stage}: {source}")]
    Coding {
        /// The failing stage.
        stage: &'static str,
        /// The stage error.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// A frame header decodes to another `OrderHint` than the planned one.
    #[error("the decoder derives OrderHint {derived:?}, not the planned {planned}")]
    OrderHintMismatch {
        /// The planned `OrderHint`.
        planned: u32,
        /// The `OrderHint` the parsed header derives.
        derived: Option<u32>,
    },

    /// The requested feature is not implemented.
    #[error("{what} is not supported yet")]
    Unsupported {
        /// The missing feature.
        what: &'static str,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder operations.
pub type Result<T> = core::result::Result<T, Error>;
