// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;

use crate::error::{DecodeError, Result};

pub(crate) fn selectable_missing_quantization_error() -> DecodeError {
    crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into()
}

pub(crate) fn selectable_symbol_read_error(
    offset: ByteOffset,
    spec_section: &'static str,
) -> DecodeError {
    crate::pipeline::malformed_tile_payload(
        offset,
        spec_section,
        "selectable transform-record syntax read failed",
    )
}

/// The frame `SbSize` in 4x4 units (§ 5.18.2): intra frames cap 256x256 at 128x128.
pub(crate) fn frame_sb_size4(sequence: &SequenceHeader, frame_is_intra: bool) -> Result<usize> {
    let partition = sequence
        .partition
        .as_ref()
        .ok_or(crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords)?;
    Ok(crate::tile::frame_superblock_h4(
        partition.seq_sb_size(),
        frame_is_intra,
    ))
}
