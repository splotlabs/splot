// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A minimal, panic-free, MSB-first bit reader for AV2 fixed-width `f(n)` syntax
//! elements and direct bitstream descriptors.
//!
//! The working AV2 § 8.2 entropy primitives live in [`crate::symbol`] and
//! [`crate::symbol_encoder`]. This module keeps only the historical
//! `RangeDecoder` stub and `RangeEncoder` compatibility wrapper.

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

/// Returns the eight bytes at `byte_index` as a big-endian window, reading
/// bytes past the end of `data` as zero padding.
#[inline]
pub(crate) fn be_window(data: &[u8], byte_index: usize) -> u64 {
    if let Some(bytes) = byte_index
        .checked_add(8)
        .and_then(|end| data.get(byte_index..end))
        && let Ok(bytes) = <[u8; 8]>::try_from(bytes)
    {
        return u64::from_be_bytes(bytes);
    }
    let mut window = 0u64;
    let tail = data.get(byte_index..).unwrap_or_default();
    for (offset, &byte) in tail.iter().take(8).enumerate() {
        window |= u64::from(byte) << (56 - offset * 8);
    }
    window
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

    /// Reads a single bit as a boolean flag, matching the AV2 `f(1)` flag idiom.
    ///
    /// Returns `true` when the bit is `1` and `false` when it is `0`; this is the
    /// boolean spelling of [`Self::read_bit`] used for every `*_flag` syntax element.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn read_flag(&mut self) -> Result<bool> {
        Ok(self.read_bit()? != 0)
    }

    /// Reads `n` bits (MSB-first) into a `u32`.
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n > 32`, or [`Error::UnexpectedEof`]
    /// if fewer than `n` bits remain (the reader is then positioned at end of
    /// input, matching the per-bit consumption it replaces).
    pub fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n > 32 {
            return Err(Error::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }
        if n == 0 {
            return Ok(0);
        }
        let start = self.consumed_bit_count();
        let within_payload = start
            .checked_add(n as usize)
            .is_some_and(|end| end <= self.data.len().saturating_mul(8));
        if !within_payload {
            self.byte_pos = self.data.len();
            self.bit_pos = 0;
            return Err(Error::UnexpectedEof {
                offset: self.byte_offset(),
                needed: 1,
            });
        }
        let window = be_window(self.data, self.byte_pos);
        let value = (window >> (64 - u32::from(self.bit_pos) - n)) as u32 & (u32::MAX >> (32 - n));
        let consumed = start + n as usize;
        self.byte_pos = consumed / 8;
        self.bit_pos = (consumed % 8) as u8;
        Ok(value)
    }

    /// Reads an AV2 `f(n)` field, treating `n == 0` as reading no bits (value `0`).
    ///
    /// This mirrors the AV2 convention that an `f(0)` syntax element is absent and
    /// consumes no bits; for `n >= 1` it delegates to [`read_bits`](Self::read_bits).
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n > 32`, or [`Error::UnexpectedEof`]
    /// if fewer than `n` bits remain. `n == 0` never errors.
    pub(crate) fn read_f(&mut self, n: u32) -> Result<u32> {
        self.read_bits(n)
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
        Ok(self.read_bits(n)? as u8)
    }

    /// Reads an AV2 `su(n)` signed integer descriptor (AV2 v1.0.0 § 4.11.7).
    ///
    /// The value is the `n`-bit unsigned field read MSB-first, sign-extended:
    /// `signMask = 1 << (n - 1)`; if the sign bit is set, `value -= 2 * signMask`.
    /// The bottom `n` bits of the result equal the coded unsigned value.
    ///
    /// # Errors
    /// Returns [`Error::BitWidthTooLarge`] if `n == 0` or `n > 32`, or
    /// [`Error::UnexpectedEof`] if fewer than `n` bits remain.
    pub fn read_su(&mut self, n: u32) -> Result<i32> {
        if n == 0 || n > 32 {
            return Err(Error::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }

        let value = self.read_bits(n)?;
        let sign_mask = 1u32 << (n - 1);
        Ok(if value & sign_mask == 0 {
            value as i32
        } else {
            value.wrapping_sub(sign_mask.wrapping_mul(2)) as i32
        })
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

    /// Reads an AV2 `svlc()` signed variable-length descriptor (AV2 v1.0.0 § 4.11.4).
    ///
    /// The unsigned [`Self::read_uvlc`] value `v` maps to a signed integer via
    /// `half = (v + 1) >> 1`; the result is `half` when `v` is odd and `-half` when
    /// `v` is even, so `0 -> 0`, `1 -> 1`, `2 -> -1`, `3 -> 2`, `4 -> -2`.
    ///
    /// # Errors
    /// Propagates [`Error::UnexpectedEof`] or [`Error::InvalidUvlc`] from
    /// [`Self::read_uvlc`] (the only failure paths; `svlc()` reads no further bits).
    pub fn read_svlc(&mut self) -> Result<i32> {
        let value = self.read_uvlc()?;
        let half = ((value + 1) >> 1) as i32;
        if value & 1 == 1 { Ok(half) } else { Ok(-half) }
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

        let mut value = 0;
        for i in 0..n {
            value |= u64::from(self.read_bits_u8(8)?) << (i * 8);
        }
        Ok(value)
    }

    /// Reads an AV2 `leb128()` descriptor (AV2 v1.0.0 § 4.11.6).
    ///
    /// The value is encoded in up to eight little-endian groups of seven bits, each
    /// group preceded by a continuation bit (the most-significant bit of the byte):
    /// a set continuation bit means another group follows. AV2 requires the decoded
    /// value to fit in `u32` and to use at most eight bytes.
    ///
    /// `leb128()` only appears at byte-aligned positions in the AV2 syntax; this
    /// reader consumes whole 8-bit groups via [`Self::read_bits_u8`] regardless of the
    /// current bit position, so it never panics, but its value is only meaningful when
    /// the reader is byte-aligned (the spec guarantees this at every `leb128()` site).
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if the code is truncated, or
    /// [`Error::InvalidLeb128`] if it uses more than eight bytes or the decoded value
    /// exceeds `(1 << 32) - 1`.
    pub fn read_leb128(&mut self) -> Result<u32> {
        let start_offset = self.byte_offset();
        let mut value: u64 = 0;
        for i in 0..8u32 {
            let byte = self.read_bits_u8(8)?;
            value |= u64::from(byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                return u32::try_from(value).map_err(|_| Error::InvalidLeb128 {
                    offset: start_offset,
                    message: "value exceeds (1 << 32) - 1".to_owned(),
                });
            }
        }

        Err(Error::InvalidLeb128 {
            offset: start_offset,
            message: "LEB128 uses more than 8 bytes (MSB of the 8th byte, index 7, is set)"
                .to_owned(),
        })
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
        let m = if w == u32::BITS {
            u32::MAX - n + 1
        } else {
            (1u32 << w) - n
        };
        let v = self.read_bits(w - 1)?;
        let value = if v < m {
            v
        } else {
            let extra_bit = u32::from(self.read_bit()?);
            (v << 1) - m + extra_bit
        };
        Ok(value)
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
                let overflow = || Error::InvalidRg {
                    offset: start_offset,
                    bit_offset: start_bit_offset,
                    message: "decoded value does not fit in u32".to_owned(),
                };
                let base = if q == 0 {
                    0
                } else if n == u32::BITS {
                    return Err(overflow());
                } else {
                    q.checked_mul(1u32 << n).ok_or_else(overflow)?
                };
                return base.checked_add(remainder).ok_or_else(overflow);
            }
        }

        Err(Error::InvalidRg {
            offset: start_offset,
            bit_offset: start_bit_offset,
            message: "rg(n) prefix must terminate within 32 bits".to_owned(),
        })
    }

    /// Splits off a sub-reader over the next `n` bytes and advances this reader past
    /// them.
    ///
    /// The returned reader is bounded to exactly `n` bytes, so any descriptor that
    /// reads past `n` bytes returns [`Error::UnexpectedEof`]. This is how a length-
    /// bounded payload — `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1) —
    /// prevents child syntax from overreading its declared size: the parent advances by
    /// exactly `n` bytes regardless of how much of the sub-reader the child consumes
    /// (the trailing `metadata_unit_remaining_bit` bits, § 6.16.1, are skipped).
    ///
    /// The reader must be byte-aligned. Every AV2 site that bounds a payload by a byte
    /// count (`metadata_unit`, `lcr_global_payload`, ...) is byte-aligned because its
    /// preceding syntax is whole bytes, so this precondition holds for all inputs; it is
    /// asserted in debug builds.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if fewer than `n` bytes remain.
    pub fn take_bytes(&mut self, n: usize) -> Result<BitReader<'a>> {
        debug_assert_eq!(
            self.bit_pos, 0,
            "take_bytes requires the reader to be byte-aligned"
        );
        let slice = self
            .byte_pos
            .checked_add(n)
            .and_then(|end| self.data.get(self.byte_pos..end));
        let Some(slice) = slice else {
            let available = self.data.len().saturating_sub(self.byte_pos);
            return Err(Error::UnexpectedEof {
                offset: self.byte_offset(),
                needed: n.saturating_sub(available),
            });
        };
        let base = self.byte_offset();
        self.byte_pos = self.byte_pos.saturating_add(n);
        Ok(BitReader::new(slice, base))
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
            if self.read_flag()? {
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

/// Compatibility wrapper for the AV2 § 8.2 symbol/range encoder.
///
/// New code should use [`crate::symbol_encoder::SymbolEncoder`] directly; this
/// type preserves the older `bitio::RangeEncoder` name that predated the
/// dedicated symbol encoder module.
#[derive(Debug)]
pub struct RangeEncoder {
    inner: crate::symbol_encoder::SymbolEncoder,
}

impl RangeEncoder {
    /// Creates a range encoder with default configuration.
    ///
    /// # Errors
    /// This compatibility constructor is currently infallible.
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: crate::symbol_encoder::SymbolEncoder::new(),
        })
    }

    /// Creates a range encoder with explicit symbol encoder configuration.
    #[must_use]
    pub fn with_config(config: crate::symbol_encoder::SymbolEncoderConfig) -> Self {
        Self {
            inner: crate::symbol_encoder::SymbolEncoder::with_config(config),
        }
    }

    /// Consumes this compatibility wrapper and returns the underlying symbol encoder.
    #[must_use]
    pub fn into_symbol_encoder(self) -> crate::symbol_encoder::SymbolEncoder {
        self.inner
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
    fn read_flag_maps_bit_to_bool() {
        let mut reader = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
        assert!(reader.read_flag().unwrap());
        assert!(!reader.read_flag().unwrap());
        for _ in 0..6 {
            assert!(!reader.read_flag().unwrap());
        }
        assert!(matches!(
            reader.read_flag(),
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
    fn range_decoder_stub_is_unimplemented_and_encoder_wrapper_constructs() {
        assert!(matches!(
            RangeDecoder::new(),
            Err(Error::Unimplemented { .. })
        ));
        assert!(RangeEncoder::new().is_ok());
    }

    #[test]
    fn read_bits_crosses_bytes_from_unaligned_positions() {
        let data = [0b1011_0010, 0b0111_0101, 0b1100_1110, 0b0001_1011];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert_eq!(reader.read_bits(3).unwrap(), 0b101);
        assert_eq!(reader.read_bits(13).unwrap(), 0b1_0010_0111_0101);
        assert_eq!(reader.read_bits(16).unwrap(), 0b1100_1110_0001_1011);
        assert!(matches!(
            reader.read_bits(1),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn read_bits_past_end_consumes_remaining_and_reports_end_offset() {
        let data = [0xAB];
        let mut reader = BitReader::new(&data, ByteOffset::new(4));
        let _ = reader.read_bits(6).unwrap();
        assert!(matches!(
            reader.read_bits(5),
            Err(Error::UnexpectedEof { offset, needed: 1 }) if offset == ByteOffset::new(5)
        ));
        assert_eq!(reader.remaining_bits(), 0);
        assert!(reader.is_byte_aligned());
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
    fn read_svlc_maps_uvlc_to_signed_values() {
        let cases: [(&[u8], i32); 5] = [
            (&[0b1000_0000], 0),
            (&[0b0100_0000], 1),
            (&[0b0110_0000], -1),
            (&[0b0010_0000], 2),
            (&[0b0010_1000], -2),
        ];
        for (data, expected) in cases {
            let mut reader = BitReader::new(data, ByteOffset::new(0));
            assert_eq!(reader.read_svlc().unwrap(), expected);
        }
    }

    #[test]
    fn read_svlc_reports_eof_and_leading_zero_bound() {
        let mut eof = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(eof.read_svlc(), Err(Error::UnexpectedEof { .. })));

        let mut too_many_zeros = BitReader::new(&[0x00, 0x00, 0x00, 0x00], ByteOffset::new(0));
        assert!(matches!(
            too_many_zeros.read_svlc(),
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
    fn read_leb128_decodes_single_and_multi_byte() {
        let mut zero = BitReader::new(&[0x00], ByteOffset::new(0));
        assert_eq!(zero.read_leb128().unwrap(), 0);
        assert_eq!(zero.byte_offset(), ByteOffset::new(1));

        let mut max1 = BitReader::new(&[0x7F], ByteOffset::new(0));
        assert_eq!(max1.read_leb128().unwrap(), 127);

        let mut two = BitReader::new(&[0x80, 0x01], ByteOffset::new(0));
        assert_eq!(two.read_leb128().unwrap(), 128);
        assert_eq!(two.byte_offset(), ByteOffset::new(2));

        let mut example = BitReader::new(&[0xE5, 0x8E, 0x26], ByteOffset::new(0));
        assert_eq!(example.read_leb128().unwrap(), 624_485);
    }

    #[test]
    fn read_leb128_reports_eof_and_overflow() {
        let mut eof = BitReader::new(&[0x80], ByteOffset::new(0));
        assert!(matches!(
            eof.read_leb128(),
            Err(Error::UnexpectedEof { .. })
        ));

        let mut too_long = BitReader::new(&[0x80; 9], ByteOffset::new(0));
        assert!(matches!(
            too_long.read_leb128(),
            Err(Error::InvalidLeb128 { .. })
        ));

        let mut overflow = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F], ByteOffset::new(0));
        assert!(matches!(
            overflow.read_leb128(),
            Err(Error::InvalidLeb128 { .. })
        ));
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
        let mut zero_prefix = BitReader::new(&[0b0100_0000], ByteOffset::new(0));
        assert_eq!(zero_prefix.read_rg(2).unwrap(), 2);

        let mut one_prefix = BitReader::new(&[0b1011_0000], ByteOffset::new(0));
        assert_eq!(one_prefix.read_rg(2).unwrap(), 7);

        let mut value_zero = BitReader::new(&[0b0000_0000], ByteOffset::new(0));
        assert_eq!(value_zero.read_rg(2).unwrap(), 0);
    }

    #[test]
    fn read_rg_two_caps_at_127() {
        let mut reader = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFE, 0xC0], ByteOffset::new(0));
        assert_eq!(reader.read_rg(2).unwrap(), 127);
    }

    #[test]
    fn read_rg_rejects_non_terminating_prefix_and_reports_eof() {
        let mut non_terminating = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert!(matches!(
            non_terminating.read_rg(2),
            Err(Error::InvalidRg { .. })
        ));

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
        assert_eq!(reader.byte_offset(), ByteOffset::new(0));
    }

    #[test]
    fn read_su_decodes_single_bit_sign() {
        let mut zero = BitReader::new(&[0b0000_0000], ByteOffset::new(0));
        assert_eq!(zero.read_su(1).unwrap(), 0);

        let mut neg_one = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
        assert_eq!(neg_one.read_su(1).unwrap(), -1);
    }

    #[test]
    fn read_su_decodes_positive_and_negative_multi_bit() {
        let mut positive = BitReader::new(&[0b0101_0000], ByteOffset::new(0));
        assert_eq!(positive.read_su(4).unwrap(), 5);

        let mut negative = BitReader::new(&[0b1011_0000], ByteOffset::new(0));
        assert_eq!(negative.read_su(4).unwrap(), -5);

        let mut min10 = BitReader::new(&[0b1000_0000, 0b0000_0000], ByteOffset::new(0));
        assert_eq!(min10.read_su(10).unwrap(), -512);

        let mut max10 = BitReader::new(&[0b0111_1111, 0b1100_0000], ByteOffset::new(0));
        assert_eq!(max10.read_su(10).unwrap(), 511);
    }

    #[test]
    fn read_su_handles_full_width_boundary() {
        let mut min32 = BitReader::new(&[0x80, 0x00, 0x00, 0x00], ByteOffset::new(0));
        assert_eq!(min32.read_su(32).unwrap(), i32::MIN);

        let mut max32 = BitReader::new(&[0x7F, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert_eq!(max32.read_su(32).unwrap(), i32::MAX);

        let mut neg_one = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert_eq!(neg_one.read_su(32).unwrap(), -1);
    }

    #[test]
    fn read_su_reports_eof_and_rejects_invalid_widths() {
        let mut eof = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(eof.read_su(4), Err(Error::UnexpectedEof { .. })));

        let mut zero_width = BitReader::new(&[0xFF], ByteOffset::new(0));
        assert!(matches!(
            zero_width.read_su(0),
            Err(Error::BitWidthTooLarge { .. })
        ));
        assert_eq!(zero_width.byte_offset(), ByteOffset::new(0));

        let mut wide = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF], ByteOffset::new(0));
        assert!(matches!(
            wide.read_su(33),
            Err(Error::BitWidthTooLarge { .. })
        ));
        assert_eq!(wide.byte_offset(), ByteOffset::new(0));
    }

    #[test]
    fn take_bytes_bounds_a_sub_reader_and_advances_the_parent() {
        let data = [0x11, 0x22, 0x33, 0x44];
        let mut reader = BitReader::new(&data, ByteOffset::new(10));
        let mut sub = reader.take_bytes(2).unwrap();
        assert_eq!(reader.byte_offset(), ByteOffset::new(12));
        assert_eq!(sub.byte_offset(), ByteOffset::new(10));
        assert_eq!(sub.read_bits_u8(8).unwrap(), 0x11);
        assert_eq!(sub.read_bits_u8(8).unwrap(), 0x22);
        assert!(matches!(
            sub.read_bits_u8(8),
            Err(Error::UnexpectedEof { .. })
        ));
        assert_eq!(reader.read_bits_u8(8).unwrap(), 0x33);
    }

    #[test]
    fn take_bytes_zero_yields_empty_sub_reader() {
        let data = [0xAB];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut sub = reader.take_bytes(0).unwrap();
        assert_eq!(sub.remaining_bits(), 0);
        assert!(matches!(sub.read_bit(), Err(Error::UnexpectedEof { .. })));
        assert_eq!(reader.read_bits_u8(8).unwrap(), 0xAB);
    }

    #[test]
    fn take_bytes_reports_eof_when_too_few_bytes_remain() {
        let data = [0x01, 0x02];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            reader.take_bytes(3),
            Err(Error::UnexpectedEof { .. })
        ));
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
        /// Windowed multi-bit reads must match the bit-by-bit reference exactly,
        /// including consumed position and end-of-input behavior.
        #[test]
        fn read_bits_matches_per_bit_reference(
            data in proptest::collection::vec(any::<u8>(), 0..12),
            widths in proptest::collection::vec(0u32..=32, 1..8),
        ) {
            let mut chunked = BitReader::new(&data, ByteOffset::new(0));
            let mut reference = BitReader::new(&data, ByteOffset::new(0));
            for n in widths {
                let expected = (|| {
                    let mut value = 0u32;
                    for _ in 0..n {
                        value = (value << 1) | u32::from(reference.read_bit()?);
                    }
                    Ok::<u32, Error>(value)
                })();
                let actual = chunked.read_bits(n);
                match (expected, actual) {
                    (Ok(expected), Ok(actual)) => prop_assert_eq!(expected, actual),
                    (Err(_), Err(_)) => {}
                    (expected, actual) => {
                        return Err(TestCaseError::fail(format!(
                            "mismatch: reference {expected:?} vs chunked {actual:?}"
                        )));
                    }
                }
                prop_assert_eq!(reference.consumed_bits(), chunked.consumed_bits());
            }
        }

        /// Descriptor readers must never panic on arbitrary input.
        #[test]
        fn descriptor_readers_never_panic(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            n in 0u32..=64,
        ) {
            let mut uvlc_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = uvlc_reader.read_uvlc();

            let mut svlc_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = svlc_reader.read_svlc();

            let mut leb128_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = leb128_reader.read_leb128();

            let mut le_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = le_reader.read_le(n);

            let mut ns_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = ns_reader.read_ns(n);

            let mut rg_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = rg_reader.read_rg(n.min(32));

            let mut su_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = su_reader.read_su(n.min(32));

            let mut alignment_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = alignment_reader.read_bits(n.min(7));
            let _ = alignment_reader.byte_align_zero();
        }
    }
}
