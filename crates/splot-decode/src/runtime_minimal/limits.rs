// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Limit-budget helpers for the minimal runtime.

use splot_core::headers::frame::FrameSize;
use splot_core::headers::sequence::ChromaFormatIdc;

use crate::error::Result;
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimitOp};

pub(super) struct DecodedFrameByteBudget {
    pub(super) luma_samples: u64,
    pub(super) chroma_samples: u64,
    pub(super) decoded_bytes: u64,
}

pub(super) struct DecodedFrameStorageBudget {
    pub(super) luma_samples: u64,
    pub(super) chroma_samples_per_plane: u64,
    pub(super) decoded_bytes: u64,
}

pub(super) fn decoded_frame_byte_budget(
    frame_size: FrameSize,
    bytes_per_sample: u64,
) -> Result<DecodedFrameByteBudget> {
    let budget =
        decoded_frame_storage_budget(frame_size, ChromaFormatIdc::Yuv420, bytes_per_sample)?;
    Ok(DecodedFrameByteBudget {
        luma_samples: budget.luma_samples,
        chroma_samples: budget.chroma_samples_per_plane,
        decoded_bytes: budget.decoded_bytes,
    })
}

pub(super) fn decoded_frame_storage_budget(
    frame_size: FrameSize,
    chroma_format: ChromaFormatIdc,
    bytes_per_sample: u64,
) -> Result<DecodedFrameStorageBudget> {
    let width = u64::from(frame_size.width);
    let height = u64::from(frame_size.height);
    let sample_limit = DecodeLimitName::MaxLumaSamplesPerFrame;
    let byte_limit = DecodeLimitName::MaxDecodedFrameBytes;
    let luma_samples = checked_mul(sample_limit, width, height)?;
    let (chroma_width, chroma_height, chroma_plane_count) = match chroma_format {
        ChromaFormatIdc::Monochrome => (0, 0, 0),
        ChromaFormatIdc::Yuv420 => ((width + 1) >> 1, (height + 1) >> 1, 2),
        ChromaFormatIdc::Yuv422 => ((width + 1) >> 1, height, 2),
        ChromaFormatIdc::Yuv444 => (width, height, 2),
    };
    let chroma_samples_per_plane = checked_mul(sample_limit, chroma_width, chroma_height)?;
    let chroma_samples = checked_mul(sample_limit, chroma_samples_per_plane, chroma_plane_count)?;
    let decoded_samples = checked_add(byte_limit, luma_samples, chroma_samples)?;
    let decoded_bytes = checked_mul(byte_limit, decoded_samples, bytes_per_sample)?;
    Ok(DecodedFrameStorageBudget {
        luma_samples,
        chroma_samples_per_plane,
        decoded_bytes,
    })
}

pub(super) fn checked_add(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_add(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Add,
            left,
            right,
        })
}

pub(super) fn checked_mul(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_mul(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Mul,
            left,
            right,
        })
}
