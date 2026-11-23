// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

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
    /// Creates a [`BitOffset`] from a value already known to be `0..=7` (for
    /// example, a bit position tracked by the bit reader).
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Creates a [`BitOffset`], returning `None` if `value > 7`.
    #[must_use]
    pub const fn try_new(value: u8) -> Option<Self> {
        if value <= 7 { Some(Self(value)) } else { None }
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

/// A contiguous span of bits, measured relative to the start of an observed
/// bitstream unit (such as an OBU payload or OBU header).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BitSpan {
    /// Zero-based starting bit index from the start of the observed unit.
    pub start_bit: u64,
    /// Length of the span in bits.
    pub bit_len: u64,
}

impl BitSpan {
    /// Creates a [`BitSpan`] from a starting bit index and bit length.
    #[must_use]
    pub const fn new(start_bit: u64, bit_len: u64) -> Self {
        Self { start_bit, bit_len }
    }

    /// Returns the first bit index past the end of the span (saturating).
    #[must_use]
    pub const fn end_bit(self) -> u64 {
        self.start_bit.saturating_add(self.bit_len)
    }

    /// Returns `true` when the span covers zero bits.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bit_len == 0
    }

    /// Returns the byte offset of [`Self::start_bit`], relative to the observed unit.
    #[must_use]
    pub const fn start_byte_offset(self) -> ByteOffset {
        ByteOffset::new(self.start_bit / 8)
    }

    /// Returns the MSB-first bit offset (`0..=7`) of [`Self::start_bit`] within its byte.
    #[must_use]
    pub const fn start_bit_offset(self) -> BitOffset {
        BitOffset::from_bits((self.start_bit % 8) as u8)
    }

    /// Returns the minimal [`ByteSpan`] enclosing every byte touched by this bit span.
    #[must_use]
    pub const fn enclosing_byte_span(self) -> ByteSpan {
        let start_byte = self.start_bit / 8;
        if self.bit_len == 0 {
            return ByteSpan::new(ByteOffset::new(start_byte), 0);
        }
        let end_byte = self.end_bit().div_ceil(8);
        ByteSpan::new(
            ByteOffset::new(start_byte),
            end_byte.saturating_sub(start_byte),
        )
    }
}

#[cfg(test)]
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
        assert_eq!(BitOffset::from_bits(3).to_string(), "3");
    }

    #[test]
    fn bit_offset_try_new_enforces_range() {
        assert_eq!(BitOffset::try_new(7).map(BitOffset::get), Some(7));
        assert!(BitOffset::try_new(8).is_none());
    }

    #[test]
    fn bit_span_offsets_and_enclosing_bytes() {
        let span = BitSpan::new(5, 6);
        assert_eq!(span.end_bit(), 11);
        assert!(!span.is_empty());
        assert_eq!(span.start_byte_offset(), ByteOffset::new(0));
        assert_eq!(span.start_bit_offset(), BitOffset::from_bits(5));
        assert_eq!(
            span.enclosing_byte_span(),
            ByteSpan::new(ByteOffset::new(0), 2)
        );

        let empty = BitSpan::new(16, 0);
        assert!(empty.is_empty());
        assert_eq!(
            empty.enclosing_byte_span(),
            ByteSpan::new(ByteOffset::new(2), 0)
        );
    }
}
