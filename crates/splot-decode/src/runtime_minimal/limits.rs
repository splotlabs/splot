// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Limit-budget helpers for the minimal runtime.

use splot_core::headers::frame::FrameSize;

use crate::error::Result;
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimitOp};

pub(super) struct DecodedFrameByteBudget {
    pub(super) luma_samples: u64,
    pub(super) chroma_samples: u64,
    pub(super) decoded_bytes: u64,
}

pub(super) fn decoded_frame_byte_budget(frame_size: FrameSize) -> Result<DecodedFrameByteBudget> {
    let luma_samples = checked_mul(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        u64::from(frame_size.width),
        u64::from(frame_size.height),
    )?;
    // AV2 §5.3.2 4:2:0 chroma plane size uses `(dimension + subsamplingX) >> 1`
    // rounding. Equivalent to `dimension / 2` for the admitted even (multiple-of-64)
    // sizes, but written spec-faithfully so a future size relaxation stays correct.
    let chroma_width = (u64::from(frame_size.width) + 1) >> 1;
    let chroma_height = (u64::from(frame_size.height) + 1) >> 1;
    let chroma_samples = checked_mul(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        chroma_width,
        chroma_height,
    )?;
    let decoded_bytes = checked_add(
        DecodeLimitName::MaxDecodedFrameBytes,
        luma_samples,
        checked_mul(DecodeLimitName::MaxDecodedFrameBytes, chroma_samples, 2)?,
    )?;
    Ok(DecodedFrameByteBudget {
        luma_samples,
        chroma_samples,
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

fn checked_mul(
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
