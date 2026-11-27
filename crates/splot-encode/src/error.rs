// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

use splot_core::headers::sequence::CroppingWindow;
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
    #[error("chroma format {chroma:?} is not supported")]
    UnsupportedChroma {
        /// Configured chroma format.
        chroma: PixelFormat,
    },

    /// A resize denominator is outside [`crate::Config::RESIZE_DENOMINATORS`].
    #[error("resize denominator {denominator} is outside 9..=16")]
    InvalidResize {
        /// Requested denominator.
        denominator: u8,
    },

    /// The cropping window does not fit the coded frame
    /// ([`CroppingWindow::fits`]).
    #[error("cropping window {window:?} does not fit the {width}x{height} coded frame")]
    InvalidCrop {
        /// Requested window.
        window: CroppingWindow,
        /// Coded width.
        width: u32,
        /// Coded height.
        height: u32,
    },

    /// A frame-rate term is zero.
    #[error("frame rate {numerator}:{denominator} must have nonzero terms")]
    InvalidFrameRate {
        /// Numerator.
        numerator: u32,
        /// Denominator.
        denominator: u32,
    },

    /// The constant quantizer is outside `0..=MaxQ` of the bit depth.
    #[error("qp {qp} is outside 0..={max}")]
    InvalidQp {
        /// Requested `base_q_idx`.
        qp: u16,
        /// `MaxQ` of the configured bit depth (§ 6.4.1 Table 6.3).
        max: u16,
    },

    /// A lossless configuration (`qp` 0) asks for a tool that has no
    /// lossless meaning.
    #[error("lossless coding cannot use {what}")]
    LosslessWith {
        /// The refused tool or preset field.
        what: String,
    },

    /// The rate factor is outside `1..=63`.
    #[error("crf {crf} is outside 1..={max}", max = crate::rate_control::Crf::MAX)]
    InvalidCrf {
        /// Requested rate factor.
        crf: u8,
    },

    /// The key frame interval is above the longest one temporal layers allow
    /// (`PresetParams::max_tlayer_id` above 0).
    #[error("keyint {keyint} is above {max}, the longest interval with temporal layers")]
    KeyintWithTemporalLayers {
        /// Requested key frame interval.
        keyint: u32,
        /// The longest interval with temporal layers.
        max: u32,
    },

    /// [`crate::Config::cbr_padding`] is set without CBR rate control.
    #[error("CBR padding needs CBR rate control")]
    PaddingWithoutCbr,

    /// A bitrate setting is zero or too large.
    #[error("{what} {value} is outside 1..={max}")]
    InvalidBitrate {
        /// The setting.
        what: &'static str,
        /// Its value.
        value: u32,
        /// Its largest value.
        max: u32,
    },

    /// The initial CBR buffer level is above the buffer size.
    #[error("initial buffer {initial_ms} ms is above the buffer of {buffer_ms} ms")]
    InvalidInitialBuffer {
        /// The initial level in milliseconds.
        initial_ms: u32,
        /// The buffer size in milliseconds.
        buffer_ms: u32,
    },

    /// The content description does not fit the input.
    #[error("content description: {what}")]
    InvalidContentDescription {
        /// The rule the description breaks.
        what: &'static str,
    },

    /// A mastering display field is outside its § 6.16.6 range and not 0.
    #[error("mastering display {field} {value} is outside {min}..={max} (or 0 for unknown)")]
    InvalidMasteringDisplay {
        /// The offending field.
        field: &'static str,
        /// Its value.
        value: u32,
        /// Smallest non-zero valid value.
        min: u32,
        /// Largest valid value.
        max: u32,
    },

    /// A metadata payload that its syntax cannot carry.
    #[error("metadata: {what}")]
    InvalidMetadata {
        /// The rule the payload breaks.
        what: &'static str,
    },

    /// A tile layout that the frames cannot code.
    #[error("tile layout {tiles:?}: {reason}")]
    InvalidTiles {
        /// Requested layout.
        tiles: crate::config::TileLayout,
        /// The rule the layout breaks.
        reason: &'static str,
    },

    /// The speed preset is outside `0..=10`.
    #[error("preset {preset} is outside 0..={max}", max = crate::preset::Preset::MAX)]
    InvalidPreset {
        /// Requested preset.
        preset: u8,
    },

    /// A preset field override names no field or does not fit it.
    #[error(transparent)]
    Preset(#[from] crate::preset::PresetError),

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

    /// [`crate::CancelToken::cancel`] stopped the encoder.
    #[error("the encoder was cancelled")]
    Cancelled,

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
