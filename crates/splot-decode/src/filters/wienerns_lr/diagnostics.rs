// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
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

pub(crate) fn intra_capped_seq_sb_size(sequence: &SequenceHeader) -> Result<SuperblockSize> {
    let partition = sequence
        .partition
        .as_ref()
        .ok_or(crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords)?;
    Ok(partition.seq_sb_size())
}
