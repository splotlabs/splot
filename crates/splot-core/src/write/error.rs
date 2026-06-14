// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed errors for the AV2 bitstream writer (`ENC-BITSTREAM-WRITER`).
//!
//! The writer is the inverse of the [`crate::bitio::BitReader`] descriptors. A
//! [`WriteError`] is raised when a model value cannot be encoded by the requested
//! AV2 descriptor — for example a value too large for a fixed field, or a width
//! outside a descriptor's domain. These are *encoder-side* programming errors
//! (the caller asked for an impossible encoding), distinct from the parser's
//! conformance/EOF [`crate::error::Error`] variants, so the writer carries its own
//! self-contained error type and never touches the parser error model.

use thiserror::Error;

/// An AV2 bitstream-writer descriptor could not encode the requested value.
///
/// Every variant corresponds to a precondition of the matching
/// [`crate::bitio::BitReader`] descriptor: the writer rejects exactly the values
/// the reader could never have produced, so the round-trip property
/// `read(write(x)) == x` holds for every value the writer accepts.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WriteError {
    /// A fixed-width write requested more bits than the descriptor allows
    /// (`f(n)`/`su(n)`/`rg(n)` accept `n <= 32`).
    #[error("bit width {requested} exceeds the maximum of {max}")]
    BitWidthTooLarge {
        /// The requested width, in bits.
        requested: u32,
        /// The maximum width the descriptor permits, in bits.
        max: u32,
    },

    /// A little-endian write requested more bytes than the descriptor allows
    /// (`le(n) -> u64` accepts `n <= 8`).
    #[error("byte width {requested} exceeds the maximum of {max}")]
    ByteWidthTooLarge {
        /// The requested width, in bytes.
        requested: u32,
        /// The maximum width the descriptor permits, in bytes.
        max: u32,
    },

    /// A descriptor that requires a positive width was given zero (e.g. `ns(0)`).
    #[error("the {descriptor} descriptor requires a width greater than zero")]
    ZeroWidth {
        /// The AV2 descriptor name (`"ns"`).
        descriptor: &'static str,
    },

    /// A value does not fit in the requested fixed field width.
    #[error("value {value} does not fit in {width_bits} bit(s)")]
    ValueTooWide {
        /// The offending value.
        value: u64,
        /// The field width that cannot hold it, in bits.
        width_bits: u32,
    },

    /// A value lies outside the range the descriptor can encode (`su(n)` signed
    /// range, `ns(n)` `0..n`, `uvlc`/`svlc` conformance bound, or `rg(n)` whose
    /// unary prefix would not terminate within 32 bits).
    #[error("the {descriptor} descriptor cannot encode value {value}")]
    ValueOutOfRange {
        /// The AV2 descriptor name (`"su"`, `"ns"`, `"uvlc"`, `"svlc"`, `"rg"`).
        descriptor: &'static str,
        /// The offending value, widened to `i64` so both signed and unsigned
        /// descriptors share one variant.
        value: i64,
    },
}

/// Result alias for [`crate::write::BitWriter`] operations.
pub type WriteResult<T> = core::result::Result<T, WriteError>;
