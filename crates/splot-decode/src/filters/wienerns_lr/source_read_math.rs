// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_recon::PlaneId;

use crate::error::DecodeError;

use super::{LR_MI_SIZE, unsupported_feature_at};

pub(crate) fn source_read_coordinate_add(
    value: isize,
    delta: isize,
    context: &'static str,
) -> crate::error::Result<isize> {
    value
        .checked_add(delta)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}

pub(crate) fn scale_chroma_source_coordinate(
    value: isize,
    subsampling: usize,
    context: &'static str,
) -> crate::error::Result<isize> {
    match subsampling {
        0 => Ok(value),
        1 => value
            .checked_mul(2)
            .ok_or_else(|| source_read_arithmetic_overflow(context)),
        _ => Err(source_read_arithmetic_overflow(context)),
    }
}

pub(crate) fn clip_source_read_coordinate(
    value: isize,
    minimum: usize,
    maximum: usize,
    context: &'static str,
) -> crate::error::Result<usize> {
    let minimum = isize::try_from(minimum).map_err(|_| source_read_arithmetic_overflow(context))?;
    let maximum = isize::try_from(maximum).map_err(|_| source_read_arithmetic_overflow(context))?;
    if minimum > maximum {
        return Err(source_read_arithmetic_overflow(context));
    }
    usize::try_from(value.clamp(minimum, maximum))
        .map_err(|_| source_read_arithmetic_overflow(context))
}

pub(crate) fn mi_to_luma_start(mi: usize, context: &'static str) -> crate::error::Result<usize> {
    mi.checked_mul(LR_MI_SIZE)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}

pub(crate) fn mi_to_luma_end(mi_end: usize, context: &'static str) -> crate::error::Result<usize> {
    mi_to_luma_start(mi_end, context)?
        .checked_sub(1)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}

pub(crate) fn usize_to_source_coordinate(
    value: usize,
    context: &'static str,
) -> crate::error::Result<isize> {
    isize::try_from(value).map_err(|_| source_read_arithmetic_overflow(context))
}

pub(crate) const fn chroma_subsampling(chroma_format: ChromaFormatIdc) -> (u8, u8) {
    match chroma_format {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (1, 1),
        ChromaFormatIdc::Yuv444 => (0, 0),
        ChromaFormatIdc::Yuv422 => (1, 0),
    }
}

pub(crate) fn wienerns_lr_source_plane(
    plane: usize,
    chroma_format: ChromaFormatIdc,
    offset: ByteOffset,
) -> crate::error::Result<PlaneId> {
    let (rule_id, message) = match plane {
        0 => return Ok(PlaneId::Y),
        1 if chroma_format != ChromaFormatIdc::Monochrome => return Ok(PlaneId::U),
        2 if chroma_format != ChromaFormatIdc::Monochrome => return Ok(PlaneId::V),
        1 | 2 => (
            "unsupported_wienerns_lr_source_chroma_plane",
            "minimal-tier reached a Wiener NS LR source-read request for a chroma plane in a monochrome sequence",
        ),
        _ => (
            "unsupported_wienerns_lr_source_plane",
            "minimal-tier reached a Wiener NS LR source-read request for an unsupported plane index",
        ),
    };

    Err(unsupported_feature_at(rule_id, offset, message, "7.20.2"))
}

pub(crate) fn source_read_arithmetic_overflow(context: &'static str) -> DecodeError {
    DecodeError::Reconstruction {
        source: splot_recon::ReconError::ArithmeticOverflow { context },
    }
}
