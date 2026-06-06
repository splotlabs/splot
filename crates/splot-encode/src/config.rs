// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder configuration: bitstream-affecting settings only.
//!
//! Runtime knobs such as thread count are passed to [`crate::Context::new`]
//! instead of living here, so that this type describes *what* is encoded rather
//! than *how fast*.

/// Placeholder sample bit depth.
// TODO(spec): confirm the exact set of AV2-permitted bit depths.
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
// TODO(spec): map to the AV2 sequence-header color configuration.
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
    // TODO(spec): profile, level, color/transfer characteristics, and more.
}

impl EncoderConfig {
    /// Creates a configuration with the given dimensions and default color format.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bit_depth: BitDepth::default(),
            chroma_subsampling: ChromaSubsampling::default(),
        }
    }
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self::new(0, 0)
    }
}
