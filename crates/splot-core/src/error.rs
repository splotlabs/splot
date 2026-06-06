// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! Typed error model for `splot-core`.
//!
//! Library code never panics on malformed input; every failure is one of these
//! variants. Recognized-but-unmodeled functionality returns
//! [`Error::Unimplemented`] rather than `todo!()`/`unimplemented!()`.

use thiserror::Error;

use crate::span::ByteOffset;

/// Errors produced while parsing AV2 bitstreams.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A feature defined by the AV2 spec is recognized but not yet modeled.
    #[error("unimplemented AV2 feature: {feature}")]
    Unimplemented {
        /// Short, stable name of the missing feature.
        feature: &'static str,
    },

    /// The input ended before a complete syntax element could be read.
    #[error("unexpected end of input at byte {offset}: needed {needed} more byte(s)")]
    UnexpectedEof {
        /// Offset at which more data was required.
        offset: ByteOffset,
        /// Number of additional bytes required.
        needed: usize,
    },

    /// A LEB128 value violated AV2 § 4.11.6.
    #[error("invalid LEB128 at byte {offset}: {message}")]
    InvalidLeb128 {
        /// Offset of the start of the LEB128 value.
        offset: ByteOffset,
        /// Human-readable reason.
        message: String,
    },

    /// An OBU header violated AV2 § 5.2.2.
    #[error("invalid OBU header at byte {offset}: {message}")]
    InvalidObuHeader {
        /// Offset of the start of the OBU header.
        offset: ByteOffset,
        /// Human-readable reason.
        message: String,
    },

    /// A declared OBU size was structurally invalid (for example, zero).
    #[error("OBU size out of range at byte {offset}: {size}")]
    ObuSizeOutOfRange {
        /// Offset of the OBU length prefix.
        offset: ByteOffset,
        /// The offending declared size.
        size: u64,
    },

    /// A declared OBU payload extends beyond the available input.
    #[error(
        "OBU payload out of range at byte {offset}: size {size} exceeds {remaining} remaining byte(s)"
    )]
    ObuPayloadOutOfRange {
        /// Offset of the OBU (its header).
        offset: ByteOffset,
        /// Declared OBU size in bytes.
        size: u32,
        /// Bytes actually remaining in the input.
        remaining: usize,
    },
}

/// Convenience alias for results produced by `splot-core`.
pub type Result<T> = core::result::Result<T, Error>;
