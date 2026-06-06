// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed error model for `splot-core`.
//!
//! Library code never panics on malformed input; every failure is one of these
//! variants. Recognized-but-unmodeled functionality returns
//! [`Error::Unimplemented`] rather than `todo!()`/`unimplemented!()`.

use core::fmt;

use thiserror::Error;

use crate::span::{BitOffset, ByteOffset};

/// Specific ways `trailing_bits(nbBits)` can violate AV2 § 6.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailingBitsErrorKind {
    /// `trailing_bits` was asked to parse zero bits.
    Empty,
    /// The required `trailing_one_bit` was not equal to `1`.
    MissingOneBit,
    /// A `trailing_zero_bit` was not equal to `0`.
    ZeroBitNotZero,
}

impl fmt::Display for TrailingBitsErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "nbBits must be greater than zero",
            Self::MissingOneBit => "trailing_one_bit must be equal to 1",
            Self::ZeroBitNotZero => "trailing_zero_bit must be equal to 0",
        };
        f.write_str(message)
    }
}

/// Specific ways `byte_alignment()` can violate AV2 § 6.2.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteAlignmentErrorKind {
    /// A byte-alignment `zero_bit` was not equal to `0`.
    ZeroBitNotZero,
}

impl fmt::Display for ByteAlignmentErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroBitNotZero => "zero_bit must be equal to 0",
        };
        f.write_str(message)
    }
}

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

    /// A bit-read requested more bits than the reader supports for the target.
    #[error("cannot read {requested} bits (maximum is {max})")]
    BitWidthTooLarge {
        /// Number of bits requested.
        requested: u32,
        /// Maximum number of bits supported for this read.
        max: u32,
    },

    /// A byte-read requested more bytes than the reader supports for the target.
    #[error("cannot read {requested} little-endian byte(s) (maximum is {max})")]
    ByteWidthTooLarge {
        /// Number of bytes requested.
        requested: u32,
        /// Maximum number of bytes supported for this read.
        max: u32,
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

    /// A `uvlc()` descriptor violated AV2 § 4.11.3.
    #[error("invalid uvlc() at byte {offset}.{bit_offset}: {message}")]
    InvalidUvlc {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidUvlc::offset`].
        bit_offset: BitOffset,
        /// Human-readable reason.
        message: String,
    },

    /// An `ns(n)` descriptor was requested with an invalid parameter.
    #[error("invalid ns(n) at byte {offset}.{bit_offset}: {message}")]
    InvalidNs {
        /// Offset of the descriptor request.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidNs::offset`].
        bit_offset: BitOffset,
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

    /// `trailing_bits(nbBits)` violated AV2 § 6.2.3.
    #[error("invalid trailing_bits() at byte {offset}.{bit_offset}: {kind}")]
    InvalidTrailingBits {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidTrailingBits::offset`].
        bit_offset: BitOffset,
        /// Specific trailing-bits violation.
        kind: TrailingBitsErrorKind,
    },

    /// `byte_alignment()` violated AV2 § 6.2.4.
    #[error("invalid byte_alignment() at byte {offset}.{bit_offset}: {kind}")]
    InvalidByteAlignment {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidByteAlignment::offset`].
        bit_offset: BitOffset,
        /// Specific byte-alignment violation.
        kind: ByteAlignmentErrorKind,
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
