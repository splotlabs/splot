// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private minimal AV2 header-intent planning for future encoder writer handoff.
//!
//! This module advances `ENC-MINIMAL-HEADER-PLAN`. It is intentionally not
//! re-exported from the crate root, does not own a bit writer, and does not
//! produce [`crate::Packet`] values. The records are a bounded, typed bridge from
//! the current encoder configuration and first accepted frame metadata toward
//! future AV2 §5.4, §5.18, §5.19, and §5.20.1 writer integration.

#![allow(dead_code)]

use splot_recon::PlaneSize;

use crate::config::{BitDepth, ChromaSubsampling, EncoderConfig};
use crate::frame::{FrameId, FrameInfo};
use crate::syntax_ir::TileIndex;

type HeaderPlanResult<T> = core::result::Result<T, HeaderPlanError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HeaderPlanError {
    #[error("encoder config {field} must be non-zero")]
    ZeroDimension { field: &'static str },

    #[error("encoder config {field} {value} cannot be represented as a plane dimension")]
    DimensionOutOfRange { field: &'static str, value: u64 },

    #[error("encoder config dimensions cannot form a coded luma size")]
    InvalidCodedLumaSize,

    #[error("minimal header planning does not yet support {bit_depth:?} input")]
    UnsupportedBitDepth { bit_depth: BitDepth },

    #[error("minimal header planning does not yet support {chroma_subsampling:?} input")]
    UnsupportedChromaSubsampling {
        chroma_subsampling: ChromaSubsampling,
    },

    #[error("frame {frame:?} visible luma size {actual:?} does not match config {expected:?}")]
    FrameSizeMismatch {
        frame: FrameId,
        expected: PlaneSize,
        actual: PlaneSize,
    },

    #[error("frame {frame:?} bit depth {actual:?} does not match config {expected:?}")]
    FrameBitDepthMismatch {
        frame: FrameId,
        expected: BitDepth,
        actual: BitDepth,
    },

    #[error("frame {frame:?} chroma subsampling {actual:?} does not match config {expected:?}")]
    FrameChromaSubsamplingMismatch {
        frame: FrameId,
        expected: ChromaSubsampling,
        actual: ChromaSubsampling,
    },

    #[error("minimal header planning only supports the first tile group")]
    UnsupportedTileGroupContinuation,

    #[error(
        "minimal header planning only supports single tile 0, got {first_tile:?}..{last_tile:?}"
    )]
    UnsupportedTileRange {
        first_tile: TileIndex,
        last_tile: TileIndex,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MinimalHeaderPlan {
    sequence: SequenceHeaderIntent,
    frame: FrameHeaderIntent,
    tile_group: TileGroupHeaderIntent,
}

impl MinimalHeaderPlan {
    pub(crate) fn new(config: &EncoderConfig, frame_info: FrameInfo) -> HeaderPlanResult<Self> {
        let coded_luma_size = config_coded_luma_size(config)?;
        validate_frame_compatibility(coded_luma_size, config, frame_info)?;
        validate_supported_subset(config)?;

        Ok(Self {
            sequence: SequenceHeaderIntent {
                coded_luma_size,
                bit_depth: config.bit_depth,
                chroma_subsampling: config.chroma_subsampling,
            },
            frame: FrameHeaderIntent {
                source_frame: frame_info.id(),
                visible_luma_size: frame_info.visible_luma_size(),
                bit_depth: frame_info.bit_depth(),
                chroma_subsampling: frame_info.chroma_subsampling(),
                kind: FrameHeaderIntentKind::FirstFrame,
            },
            tile_group: TileGroupHeaderIntent::new(TileIndex::new(0), TileIndex::new(0), true)?,
        })
    }

    pub(crate) const fn sequence(&self) -> &SequenceHeaderIntent {
        &self.sequence
    }

    pub(crate) const fn frame(&self) -> &FrameHeaderIntent {
        &self.frame
    }

    pub(crate) const fn tile_group(&self) -> &TileGroupHeaderIntent {
        &self.tile_group
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SequenceHeaderIntent {
    coded_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
}

impl SequenceHeaderIntent {
    pub(crate) const fn coded_luma_size(&self) -> PlaneSize {
        self.coded_luma_size
    }

    pub(crate) const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    pub(crate) const fn chroma_subsampling(&self) -> ChromaSubsampling {
        self.chroma_subsampling
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameHeaderIntent {
    source_frame: FrameId,
    visible_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
    kind: FrameHeaderIntentKind,
}

impl FrameHeaderIntent {
    pub(crate) const fn source_frame(&self) -> FrameId {
        self.source_frame
    }

    pub(crate) const fn visible_luma_size(&self) -> PlaneSize {
        self.visible_luma_size
    }

    pub(crate) const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    pub(crate) const fn chroma_subsampling(&self) -> ChromaSubsampling {
        self.chroma_subsampling
    }

    pub(crate) const fn kind(&self) -> FrameHeaderIntentKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameHeaderIntentKind {
    FirstFrame,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileGroupHeaderIntent {
    first_tile: TileIndex,
    last_tile: TileIndex,
    is_first_tile_group: bool,
}

impl TileGroupHeaderIntent {
    fn new(
        first_tile: TileIndex,
        last_tile: TileIndex,
        is_first_tile_group: bool,
    ) -> HeaderPlanResult<Self> {
        if !is_first_tile_group {
            return Err(HeaderPlanError::UnsupportedTileGroupContinuation);
        }
        if first_tile.get() != 0 || last_tile.get() != 0 || first_tile != last_tile {
            return Err(HeaderPlanError::UnsupportedTileRange {
                first_tile,
                last_tile,
            });
        }

        Ok(Self {
            first_tile,
            last_tile,
            is_first_tile_group,
        })
    }

    pub(crate) const fn first_tile(&self) -> TileIndex {
        self.first_tile
    }

    pub(crate) const fn last_tile(&self) -> TileIndex {
        self.last_tile
    }

    pub(crate) const fn is_first_tile_group(&self) -> bool {
        self.is_first_tile_group
    }
}

fn config_coded_luma_size(config: &EncoderConfig) -> HeaderPlanResult<PlaneSize> {
    if config.width == 0 {
        return Err(HeaderPlanError::ZeroDimension { field: "width" });
    }
    if config.height == 0 {
        return Err(HeaderPlanError::ZeroDimension { field: "height" });
    }

    let width =
        usize::try_from(config.width).map_err(|_| HeaderPlanError::DimensionOutOfRange {
            field: "width",
            value: u64::from(config.width),
        })?;
    let height =
        usize::try_from(config.height).map_err(|_| HeaderPlanError::DimensionOutOfRange {
            field: "height",
            value: u64::from(config.height),
        })?;
    PlaneSize::new(width, height).map_err(|_| HeaderPlanError::InvalidCodedLumaSize)
}

fn validate_frame_compatibility(
    coded_luma_size: PlaneSize,
    config: &EncoderConfig,
    frame_info: FrameInfo,
) -> HeaderPlanResult<()> {
    // TODO(spec: ENC-MINIMAL-HEADER-PLAN): revisit coded-vs-visible luma size
    if frame_info.visible_luma_size() != coded_luma_size {
        return Err(HeaderPlanError::FrameSizeMismatch {
            frame: frame_info.id(),
            expected: coded_luma_size,
            actual: frame_info.visible_luma_size(),
        });
    }
    if frame_info.bit_depth() != config.bit_depth {
        return Err(HeaderPlanError::FrameBitDepthMismatch {
            frame: frame_info.id(),
            expected: config.bit_depth,
            actual: frame_info.bit_depth(),
        });
    }
    if frame_info.chroma_subsampling() != config.chroma_subsampling {
        return Err(HeaderPlanError::FrameChromaSubsamplingMismatch {
            frame: frame_info.id(),
            expected: config.chroma_subsampling,
            actual: frame_info.chroma_subsampling(),
        });
    }

    Ok(())
}

fn validate_supported_subset(config: &EncoderConfig) -> HeaderPlanResult<()> {
    if config.bit_depth != BitDepth::Eight {
        return Err(HeaderPlanError::UnsupportedBitDepth {
            bit_depth: config.bit_depth,
        });
    }
    if config.chroma_subsampling != ChromaSubsampling::Yuv420 {
        return Err(HeaderPlanError::UnsupportedChromaSubsampling {
            chroma_subsampling: config.chroma_subsampling,
        });
    }

    Ok(())
}

#[cfg(test)]
#[path = "header_plan_tests.rs"]
mod tests;
