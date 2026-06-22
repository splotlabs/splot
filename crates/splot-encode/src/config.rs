// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder configuration: bitstream-affecting settings only.
//!
//! Runtime knobs such as thread count are passed to [`crate::Context::new`]
//! instead of living here, so that this type describes *what* is encoded rather
//! than *how fast*.

/// Placeholder sample bit depth.
// TODO(spec: AV2-5.4-SEQUENCE-HEADER): confirm the exact set of permitted bit depths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BitDepth {
    /// 8 bits per sample.
    #[default]
    Eight,
    /// 10 bits per sample.
    Ten,
    /// 12 bits per sample.
    Twelve,
}

/// Placeholder chroma subsampling.
// TODO(spec: AV2-5.4-SEQUENCE-HEADER): map to the sequence-header color configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ChromaSubsampling {
    /// Monochrome (no chroma planes).
    Monochrome,
    /// 4:2:0.
    #[default]
    Yuv420,
    /// 4:2:2.
    Yuv422,
    /// 4:4:4.
    Yuv444,
}

/// The default fixed quantizer index (`base_q_idx`): 80, the value the AVM- and
/// dav2d-validated `syn-flat-intra-64x64-q80` fixture muxes at, which the minimal
/// intra path has always emitted.
pub const DEFAULT_QP: u8 = 80;

/// Bitstream-affecting encoder configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncoderConfig {
    /// Frame width in luma samples.
    pub width: u32,
    /// Frame height in luma samples.
    pub height: u32,
    /// Sample bit depth (placeholder).
    pub bit_depth: BitDepth,
    /// Chroma subsampling (placeholder).
    pub chroma_subsampling: ChromaSubsampling,
    /// The fixed quantizer index used for every frame: the frame-header `base_q_idx`
    /// (AV2 § 5.18.2) and — once the lossy coefficient path is wired into
    /// `receive_packet` — the quantizer's qindex, both read from this single field so
    /// the two cannot diverge. The minimal encoder is constant-QP; per-frame and
    /// per-block rate control are a later phase behind a `RateController` seam.
    pub qp: u8,
    // TODO(spec: AV2-5.4-SEQUENCE-HEADER): profile, level, color/transfer characteristics, and more.
}

impl EncoderConfig {
    /// Creates a configuration with the given dimensions, default color format, and the
    /// [`DEFAULT_QP`] fixed quantizer.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bit_depth: BitDepth::default(),
            chroma_subsampling: ChromaSubsampling::default(),
            qp: DEFAULT_QP,
        }
    }
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self::new(0, 0)
    }
}
