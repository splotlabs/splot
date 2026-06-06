// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! Byte- and bit-offset newtypes shared by the parser and the validator.
//!
//! These keep bare integers off public boundaries so that offsets, lengths, and
//! bit positions cannot be confused with one another.

use core::fmt;

use serde::{Deserialize, Serialize};

/// A zero-based offset into the bitstream, measured in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ByteOffset(u64);

impl ByteOffset {
    /// Creates a [`ByteOffset`] from a raw byte count.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw byte offset.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns this offset advanced by `delta` bytes, saturating at [`u64::MAX`].
    #[must_use]
    pub const fn saturating_add(self, delta: u64) -> Self {
        Self(self.0.saturating_add(delta))
    }
}

impl fmt::Display for ByteOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for ByteOffset {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A bit position within a byte, `0..=7`, counted MSB-first to match the AV2
/// `f(n)` descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BitOffset(u8);

impl BitOffset {
    /// Creates a [`BitOffset`]. Values are expected to be in `0..=7`.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw bit position (`0..=7`, MSB-first).
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl fmt::Display for BitOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A contiguous span of bytes in the bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteSpan {
    /// First byte of the span.
    pub start: ByteOffset,
    /// Length of the span in bytes.
    pub len: u64,
}

impl ByteSpan {
    /// Creates a [`ByteSpan`] from a start offset and length.
    #[must_use]
    pub const fn new(start: ByteOffset, len: u64) -> Self {
        Self { start, len }
    }

    /// Returns the first byte offset past the end of the span (saturating).
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.start.saturating_add(self.len)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn byte_offset_round_trips() {
        let offset = ByteOffset::new(42);
        assert_eq!(offset.get(), 42);
        assert_eq!(ByteOffset::from(42), offset);
        assert_eq!(offset.saturating_add(8).get(), 50);
        assert_eq!(ByteOffset::new(u64::MAX).saturating_add(1).get(), u64::MAX);
    }

    #[test]
    fn byte_offsets_order() {
        assert!(ByteOffset::new(1) < ByteOffset::new(2));
    }

    #[test]
    fn byte_span_end_saturates() {
        assert_eq!(
            ByteSpan::new(ByteOffset::new(4), 6).end(),
            ByteOffset::new(10)
        );
        assert_eq!(
            ByteSpan::new(ByteOffset::new(u64::MAX), 1).end(),
            ByteOffset::new(u64::MAX)
        );
    }

    #[test]
    fn offsets_display() {
        assert_eq!(ByteOffset::new(7).to_string(), "7");
        assert_eq!(BitOffset::new(3).to_string(), "3");
    }
}
