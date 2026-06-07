// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A minimal, panic-free, MSB-first bit reader for AV2 fixed-width `f(n)` syntax
//! elements and direct bitstream descriptors, plus explicit stubs for the AV2
//! entropy (range) coder.

use crate::error::{ByteAlignmentErrorKind, Error, Result};
use crate::span::{BitOffset, ByteOffset};

/// Arbitrary-width value read by the AV2 `le(n)` descriptor (AV2 v1.0.0 § 4.11.5).
///
/// Bytes are stored in the same little-endian order they appear in the bitstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LittleEndianValue {
    bytes: Vec<u8>,
}

impl LittleEndianValue {
    /// Creates a little-endian value from raw descriptor bytes.
    #[must_use]
    pub fn from_le_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Returns the raw little-endian bytes.
    #[must_use]
    pub fn as_le_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of bytes read by the descriptor.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Converts this value to `u64` when it fits in eight bytes.
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        if self.bytes.len() > 8 {
            return None;
        }

        let mut value = 0u64;
        for (i, byte) in self.bytes.iter().enumerate() {
            value |= u64::from(*byte) << (i * 8);
        }
        Some(value)
    }
}

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
        BitOffset::from_bits(self.bit_pos)
    }

    /// Returns `true` if the reader is positioned on a byte boundary.
    #[must_use]
    pub const fn is_byte_aligned(&self) -> bool {
        self.bit_pos == 0
    }

    /// Returns the number of bits not yet read from this reader.
    #[must_use]
    pub fn remaining_bits(&self) -> u64 {
        let total_bits = self.data.len().saturating_mul(8);
        total_bits.saturating_sub(self.consumed_bit_count()) as u64
    }

    /// Returns the number of bits read so far from this reader (relative to its start).
    #[must_use]
    pub fn consumed_bits(&self) -> u64 {
        self.consumed_bit_count() as u64
    }

    /// Bits consumed since construction, in `usize` for offset arithmetic.
    const fn consumed_bit_count(&self) -> usize {
        self.byte_pos
            .saturating_mul(8)
            .saturating_add(self.bit_pos as usize)
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

    /// Reads an AV2 `uvlc()` descriptor (AV2 v1.0.0 § 4.11.3).
    ///
    /// AV2 differs from AV1 here: `leadingZeros >= 32` is a conformance
    /// violation, and no `(1 << 32) - 1` sentinel value is returned.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if the code is truncated, or
    /// [`Error::InvalidUvlc`] if the code has 32 or more leading zero bits.
    pub fn read_uvlc(&mut self) -> Result<u32> {
        let mut leading_zeros = 0u32;

        loop {
            let offset = self.byte_offset();
            let bit_offset = self.bit_offset();
            let done = self.read_bit()?;
            if done == 1 {
                break;
            }

            leading_zeros += 1;
            if leading_zeros >= 32 {
                return Err(Error::InvalidUvlc {
                    offset,
                    bit_offset,
                    message: "leadingZeros must be less than 32".to_owned(),
                });
            }
        }

        let suffix = self.read_bits(leading_zeros)?;
        Ok(suffix + (1u32 << leading_zeros) - 1)
    }

    /// Reads an arbitrary-width AV2 `le(n)` descriptor (AV2 v1.0.0 § 4.11.5).
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if fewer than `n` bytes remain.
    pub fn read_le(&mut self, n: u32) -> Result<LittleEndianValue> {
        let mut bytes = Vec::new();
        for _ in 0..n {
            bytes.push(self.read_bits_u8(8)?);
        }
        Ok(LittleEndianValue::from_le_bytes(bytes))
    }

    /// Reads an AV2 `le(n)` descriptor and converts it to `u64`.
    ///
    /// # Errors
    /// Returns [`Error::ByteWidthTooLarge`] if `n > 8`, or
    /// [`Error::UnexpectedEof`] if fewer than `n` bytes remain.
    pub fn read_le_u64(&mut self, n: u32) -> Result<u64> {
        if n > 8 {
            return Err(Error::ByteWidthTooLarge {
                requested: n,
                max: 8,
            });
        }

        match self.read_le(n)?.to_u64() {
            Some(value) => Ok(value),
            None => Err(Error::ByteWidthTooLarge {
                requested: n,
                max: 8,
            }),
        }
    }

    /// Reads an AV2 `ns(n)` non-symmetric integer descriptor (AV2 v1.0.0 § 4.11.8).
    ///
    /// # Errors
    /// Returns [`Error::InvalidNs`] if `n == 0`, or [`Error::UnexpectedEof`] if
    /// the encoded value is truncated.
    pub fn read_ns(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Err(Error::InvalidNs {
                offset: self.byte_offset(),
                bit_offset: self.bit_offset(),
                message: "n must be greater than 0".to_owned(),
            });
        }

        let w = u32::BITS - n.leading_zeros();
        let m = (1u64 << w) - u64::from(n);
        let v = u64::from(self.read_bits(w - 1)?);
        let value = if v < m {
            v
        } else {
            let extra_bit = u64::from(self.read_bit()?);
            (v << 1) - m + extra_bit
        };

        u32::try_from(value).map_err(|_| Error::InvalidNs {
            offset: self.byte_offset(),
            bit_offset: self.bit_offset(),
            message: "decoded value does not fit in u32".to_owned(),
        })
    }

    /// Reads an AV2 `rg(n)` Rice-Golomb descriptor (AV2 v1.0.0 § 4.11.10).
    ///
    /// The value is `(q << n) + remainder`, where `q` is the number of leading one
    /// bits before the first zero bit and `remainder` is the following `n`-bit
    /// suffix. AV2 § 4.11.10 requires the descriptor never return a value less than
    /// 0, i.e. the unary prefix must terminate (a zero bit must appear) within 32
    /// iterations.
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n > 32`, [`Error::InvalidRg`] if the
    /// unary prefix does not terminate within 32 bits or the value does not fit in a
    /// `u32`, or [`Error::UnexpectedEof`] if the code is truncated.
    pub fn read_rg(&mut self, n: u32) -> Result<u32> {
        if n > 32 {
            return Err(Error::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }

        let start_offset = self.byte_offset();
        let start_bit_offset = self.bit_offset();
        for q in 0u32..32 {
            if self.read_bit()? == 0 {
                let remainder = self.read_bits(n)?;
                // `q < 32` and `n <= 32`, so the shift is computed in u64 to avoid
                // overflow; the spec only ever uses small `n` (e.g. rg(2)), but the
                // u64 path keeps the general descriptor panic-free.
                let value = (u64::from(q) << n) + u64::from(remainder);
                return u32::try_from(value).map_err(|_| Error::InvalidRg {
                    offset: start_offset,
                    bit_offset: start_bit_offset,
                    message: "decoded value does not fit in u32".to_owned(),
                });
            }
        }

        Err(Error::InvalidRg {
            offset: start_offset,
            bit_offset: start_bit_offset,
            message: "rg(n) prefix must terminate within 32 bits".to_owned(),
        })
    }

    /// Parses AV2 `byte_alignment()` and validates all alignment bits are zero
    /// (AV2 v1.0.0 § 5.2.4 / § 6.2.4).
    ///
    /// # Errors
    /// Returns [`Error::InvalidByteAlignment`] if an alignment bit is non-zero,
    /// or [`Error::UnexpectedEof`] if the reader cannot reach a byte boundary.
    pub fn byte_align_zero(&mut self) -> Result<()> {
        while !self.is_byte_aligned() {
            let offset = self.byte_offset();
            let bit_offset = self.bit_offset();
            if self.read_bit()? != 0 {
                return Err(Error::InvalidByteAlignment {
                    offset,
                    bit_offset,
                    kind: ByteAlignmentErrorKind::ZeroBitNotZero,
                });
            }
        }
        Ok(())
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

    #[test]
    fn reports_remaining_bits() {
        let mut reader = BitReader::new(&[0xAA, 0x55], ByteOffset::new(0));
        assert_eq!(reader.remaining_bits(), 16);
        let _ = reader.read_bits(5).unwrap();
        assert_eq!(reader.remaining_bits(), 11);
        let _ = reader.read_bits(11).unwrap();
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn read_uvlc_decodes_values() {
        let mut zero = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
        assert_eq!(zero.read_uvlc().unwrap(), 0);

        let mut one = BitReader::new(&[0b0100_0000], ByteOffset::new(0));
        assert_eq!(one.read_uvlc().unwrap(), 1);

        let mut two = BitReader::new(&[0b0110_0000], ByteOffset::new(0));
        assert_eq!(two.read_uvlc().unwrap(), 2);
    }

    #[test]
    fn read_uvlc_reports_eof_and_leading_zero_bound() {
        let mut eof = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(eof.read_uvlc(), Err(Error::UnexpectedEof { .. })));

        let mut no_done_bit = BitReader::new(&[0x00], ByteOffset::new(0));
        assert!(matches!(
            no_done_bit.read_uvlc(),
            Err(Error::UnexpectedEof { .. })
        ));

        let mut too_many_zeros = BitReader::new(&[0x00, 0x00, 0x00, 0x00], ByteOffset::new(0));
        assert!(matches!(
            too_many_zeros.read_uvlc(),
            Err(Error::InvalidUvlc { .. })
        ));
    }

    #[test]
    fn read_le_decodes_little_endian_bytes() {
        let mut reader = BitReader::new(&[0x34, 0x12, 0xAB], ByteOffset::new(0));
        let first = reader.read_le(2).unwrap();
        assert_eq!(first.as_le_bytes(), &[0x34, 0x12]);
        assert_eq!(first.byte_len(), 2);
        assert_eq!(first.to_u64(), Some(0x1234));

        let second = reader.read_le(1).unwrap();
        assert_eq!(second.as_le_bytes(), &[0xAB]);
        assert_eq!(second.to_u64(), Some(0xAB));

        let mut u64_reader = BitReader::new(&[0x78, 0x56, 0x34, 0x12], ByteOffset::new(0));
        assert_eq!(u64_reader.read_le_u64(4).unwrap(), 0x1234_5678);
    }

    #[test]
    fn read_le_supports_values_wider_than_u64() {
        let data = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x10];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let value = reader.read_le(9).unwrap();
        assert_eq!(value.as_le_bytes(), &data);
        assert_eq!(value.byte_len(), 9);
        assert_eq!(value.to_u64(), None);
    }

    #[test]
    fn read_le_u64_rejects_too_many_bytes_before_consuming() {
        let mut wide = BitReader::new(&[0; 9], ByteOffset::new(0));
        assert!(matches!(
            wide.read_le_u64(9),
            Err(Error::ByteWidthTooLarge {
                requested: 9,
                max: 8
            })
        ));
        assert_eq!(wide.byte_offset(), ByteOffset::new(0));
    }

    #[test]
    fn read_le_reports_eof() {
        let mut eof = BitReader::new(&[0x34], ByteOffset::new(0));
        assert!(matches!(eof.read_le(2), Err(Error::UnexpectedEof { .. })));
    }

    #[test]
    fn read_ns_decodes_power_of_two_range() {
        let mut reader = BitReader::new(&[0b1010_0000], ByteOffset::new(0));
        assert_eq!(reader.read_ns(8).unwrap(), 5);
        assert_eq!(reader.bit_offset().get(), 3);
    }

    #[test]
    fn read_ns_decodes_non_power_of_two_range() {
        let cases = [
            (0b0000_0000, 0),
            (0b0100_0000, 1),
            (0b1000_0000, 2),
            (0b1100_0000, 3),
            (0b1110_0000, 4),
        ];
        for (bits, expected) in cases {
            let data = [bits];
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            assert_eq!(reader.read_ns(5).unwrap(), expected);
        }
    }

    #[test]
    fn read_ns_handles_single_value_and_invalid_zero_range() {
        let mut single = BitReader::new(&[0xFF], ByteOffset::new(0));
        assert_eq!(single.read_ns(1).unwrap(), 0);
        assert_eq!(single.bit_offset().get(), 0);

        let mut invalid = BitReader::new(&[0xFF], ByteOffset::new(0));
        assert!(matches!(invalid.read_ns(0), Err(Error::InvalidNs { .. })));
        assert_eq!(invalid.bit_offset().get(), 0);
    }

    #[test]
    fn read_rg_decodes_unary_prefix_and_remainder() {
        // q = 0: prefix bit 0, remainder 0b10 -> (0 << 2) + 2 = 2.
        // Bits: 0 10 00000 -> 0b0100_0000.
        let mut zero_prefix = BitReader::new(&[0b0100_0000], ByteOffset::new(0));
        assert_eq!(zero_prefix.read_rg(2).unwrap(), 2);

        // q = 1: prefix 1,0, remainder 0b11 -> (1 << 2) + 3 = 7.
        // Bits: 1 0 11 0000 -> 0b1011_0000.
        let mut one_prefix = BitReader::new(&[0b1011_0000], ByteOffset::new(0));
        assert_eq!(one_prefix.read_rg(2).unwrap(), 7);

        // value 0: prefix bit 0, remainder 0b00 -> 0b0000_0000.
        let mut value_zero = BitReader::new(&[0b0000_0000], ByteOffset::new(0));
        assert_eq!(value_zero.read_rg(2).unwrap(), 0);
    }

    #[test]
    fn read_rg_two_caps_at_127() {
        // rg(2)'s largest encodable value is (31 << 2) + 3 = 127: 31 one bits, the
        // terminating zero bit, then the 2-bit remainder 0b11. This is exactly the
        // §6.14 / Table 6.13 upper bound for ci_color_description_idc, so the
        // descriptor enforces the range structurally.
        // Bits: thirty-one 1s, 0, 1, 1 -> 0xFF 0xFF 0xFF 0xFE 0xC0.
        let mut reader = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFE, 0xC0], ByteOffset::new(0));
        assert_eq!(reader.read_rg(2).unwrap(), 127);
    }

    #[test]
    fn read_rg_rejects_non_terminating_prefix_and_reports_eof() {
        // 32 one bits with no terminating zero -> InvalidRg, not a panic.
        let mut non_terminating = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert!(matches!(
            non_terminating.read_rg(2),
            Err(Error::InvalidRg { .. })
        ));

        // Truncated before the prefix can be read.
        let mut eof = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(eof.read_rg(2), Err(Error::UnexpectedEof { .. })));
    }

    #[test]
    fn read_rg_rejects_widths_over_32() {
        let mut reader = BitReader::new(&[0x00], ByteOffset::new(0));
        assert!(matches!(
            reader.read_rg(33),
            Err(Error::BitWidthTooLarge {
                requested: 33,
                max: 32
            })
        ));
        // The guard rejects before consuming any bits.
        assert_eq!(reader.byte_offset(), ByteOffset::new(0));
    }

    #[test]
    fn byte_align_zero_accepts_zero_bits_and_rejects_one_bits() {
        let mut aligned = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
        let _ = aligned.read_bit().unwrap();
        aligned.byte_align_zero().unwrap();
        assert!(aligned.is_byte_aligned());

        let mut invalid = BitReader::new(&[0b1100_0000], ByteOffset::new(0));
        let _ = invalid.read_bit().unwrap();
        assert!(matches!(
            invalid.byte_align_zero(),
            Err(Error::InvalidByteAlignment { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Descriptor readers must never panic on arbitrary input.
        #[test]
        fn descriptor_readers_never_panic(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            n in 0u32..=64,
        ) {
            let mut uvlc_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = uvlc_reader.read_uvlc();

            let mut le_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = le_reader.read_le(n);

            let mut ns_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = ns_reader.read_ns(n);

            let mut rg_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = rg_reader.read_rg(n.min(32));

            let mut alignment_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = alignment_reader.read_bits(n.min(7));
            let _ = alignment_reader.byte_align_zero();
        }
    }
}
