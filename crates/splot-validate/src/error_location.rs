// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared helpers for locating parser errors in validator diagnostics.

use splot_core::Error;
use splot_core::span::{BitOffset, ByteOffset};

/// Returns the byte offset carried by `error`, when that error is location-aware.
pub(crate) fn error_offset(error: &Error) -> Option<ByteOffset> {
    match error {
        Error::UnexpectedEof { offset, .. }
        | Error::InvalidLeb128 { offset, .. }
        | Error::InvalidUvlc { offset, .. }
        | Error::InvalidNs { offset, .. }
        | Error::InvalidRg { offset, .. }
        | Error::InvalidObuHeader { offset, .. }
        | Error::InvalidTrailingBits { offset, .. }
        | Error::InvalidByteAlignment { offset, .. }
        | Error::InvalidSequenceHeader { offset, .. }
        | Error::InvalidLayerConfigRecord { offset, .. }
        | Error::InvalidAtlasSegment { offset, .. }
        | Error::ObuSizeOutOfRange { offset, .. }
        | Error::ObuPayloadOutOfRange { offset, .. } => Some(*offset),
        _ => None,
    }
}

/// Returns the bit offset carried by `error`, when that error is bit-location-aware.
pub(crate) fn error_bit_offset(error: &Error) -> Option<BitOffset> {
    match error {
        Error::InvalidUvlc { bit_offset, .. }
        | Error::InvalidNs { bit_offset, .. }
        | Error::InvalidRg { bit_offset, .. }
        | Error::InvalidTrailingBits { bit_offset, .. }
        | Error::InvalidByteAlignment { bit_offset, .. }
        | Error::InvalidSequenceHeader { bit_offset, .. }
        | Error::InvalidLayerConfigRecord { bit_offset, .. }
        | Error::InvalidAtlasSegment { bit_offset, .. } => Some(*bit_offset),
        _ => None,
    }
}
