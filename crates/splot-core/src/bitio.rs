// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A minimal, panic-free, MSB-first bit reader for AV2 fixed-width `f(n)` syntax
//! elements, plus explicit stubs for the AV2 entropy (range) coder.

use crate::error::{Error, Result};
use crate::span::{BitOffset, ByteOffset};

/// Reads fixed-width bit fields MSB-first, matching the AV2 `f(n)` descriptor.
///
/// The reader borrows a byte slice and tracks an absolute [`ByteOffset`] base so
/// that errors and diagnostics report positions relative to the whole bitstream.
#[derive(Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    base: ByteOffset,
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    /// Creates a bit reader over `data`, whose first byte is at absolute `base`.
    #[must_use]
    pub const fn new(data: &'a [u8], base: ByteOffset) -> Self {
        Self {
            data,
            base,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// Returns the absolute byte offset of the next bit to be read.
    #[must_use]
    pub const fn byte_offset(&self) -> ByteOffset {
        self.base.saturating_add(self.byte_pos as u64)
    }

    /// Returns the bit position (`0..=7`, MSB-first) within the current byte.
    #[must_use]
    pub const fn bit_offset(&self) -> BitOffset {
        BitOffset::new(self.bit_pos)
    }

    /// Returns `true` if the reader is positioned on a byte boundary.
    #[must_use]
    pub const fn is_byte_aligned(&self) -> bool {
        self.bit_pos == 0
    }

    /// Reads a single bit (MSB-first).
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn read_bit(&mut self) -> Result<u8> {
        let Some(&byte) = self.data.get(self.byte_pos) else {
            return Err(Error::UnexpectedEof {
                offset: self.byte_offset(),
                needed: 1,
            });
        };
        let bit = (byte >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Ok(bit)
    }

    /// Reads `n` bits (MSB-first) into a `u32`.
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n > 32`, or [`Error::UnexpectedEof`]
    /// if fewer than `n` bits remain.
    pub fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n > 32 {
            return Err(Error::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Ok(value)
    }

    /// Reads `n` bits (MSB-first) into a `u8`.
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n > 8`, or [`Error::UnexpectedEof`]
    /// if fewer than `n` bits remain.
    pub fn read_bits_u8(&mut self, n: u32) -> Result<u8> {
        if n > 8 {
            return Err(Error::BitWidthTooLarge {
                requested: n,
                max: 8,
            });
        }
        // `n <= 8` guarantees the value fits in a `u8` without truncation.
        Ok(self.read_bits(n)? as u8)
    }
}

/// Stub for the AV2 range (arithmetic) decoder. Entropy coding is not yet modeled.
#[derive(Debug)]
#[non_exhaustive]
pub struct RangeDecoder;

impl RangeDecoder {
    /// Creating a range decoder is not yet supported.
    ///
    /// # Errors
    /// Always returns [`Error::Unimplemented`].
    pub fn new() -> Result<Self> {
        Err(Error::Unimplemented {
            feature: "AV2 entropy coding",
        })
    }
}

/// Stub for the AV2 range (arithmetic) encoder. Entropy coding is not yet modeled.
#[derive(Debug)]
#[non_exhaustive]
pub struct RangeEncoder;

impl RangeEncoder {
    /// Creating a range encoder is not yet supported.
    ///
    /// # Errors
    /// Always returns [`Error::Unimplemented`].
    pub fn new() -> Result<Self> {
        Err(Error::Unimplemented {
            feature: "AV2 entropy coding",
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_msb_first() {
        let mut reader = BitReader::new(&[0b1011_0010], ByteOffset::new(0));
        assert_eq!(reader.read_bit().unwrap(), 1);
        assert_eq!(reader.read_bits(3).unwrap(), 0b011);
        assert_eq!(reader.read_bits(4).unwrap(), 0b0010);
        assert!(matches!(
            reader.read_bit(),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn tracks_offsets() {
        let mut reader = BitReader::new(&[0xFF, 0x00], ByteOffset::new(10));
        assert_eq!(reader.byte_offset(), ByteOffset::new(10));
        assert!(reader.is_byte_aligned());
        let _ = reader.read_bits(8).unwrap();
        assert_eq!(reader.byte_offset(), ByteOffset::new(11));
        assert!(reader.is_byte_aligned());
        let _ = reader.read_bit().unwrap();
        assert_eq!(reader.bit_offset().get(), 1);
        assert!(!reader.is_byte_aligned());
    }

    #[test]
    fn entropy_coder_is_unimplemented() {
        assert!(matches!(
            RangeDecoder::new(),
            Err(Error::Unimplemented { .. })
        ));
        assert!(matches!(
            RangeEncoder::new(),
            Err(Error::Unimplemented { .. })
        ));
    }

    #[test]
    fn read_bits_rejects_widths_over_32() {
        let mut reader = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert!(matches!(
            reader.read_bits(33),
            Err(Error::BitWidthTooLarge {
                requested: 33,
                max: 32
            })
        ));
        // The guard rejects before consuming any bits.
        assert_eq!(reader.byte_offset(), ByteOffset::new(0));
    }

    #[test]
    fn read_bits_u8_rejects_wide_reads() {
        let mut reader = BitReader::new(&[0xFF, 0xFF], ByteOffset::new(0));
        assert!(matches!(
            reader.read_bits_u8(9),
            Err(Error::BitWidthTooLarge {
                requested: 9,
                max: 8
            })
        ));
    }
}
