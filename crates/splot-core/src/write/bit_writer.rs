// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A minimal, panic-free, MSB-first bit writer that is the exact inverse of
//! [`crate::bitio::BitReader`] (`ENC-BITSTREAM-WRITER`).
//!
//! Every primitive emits the bits the matching reader descriptor would consume,
//! so the foundational writer property holds for every value the writer accepts:
//!
//! ```text
//! read(write(x)) == x
//! ```
//!
//! Bits are packed most-significant-bit first, matching the `f(n)` descriptor
//! (AV2 v1.0.0 § 4.11.2, `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-2`) and every
//! reader primitive built on it. The writer never panics: values or widths the
//! corresponding reader could never have produced are rejected with a typed
//! [`WriteError`].
//!
//! `leb128()` and other byte-granular descriptors are only meaningful at
//! byte-aligned positions (the AV2 syntax guarantees this at every site); the
//! writer emits whole 8-bit groups regardless of the current bit position, exactly
//! as [`crate::bitio::BitReader::read_leb128`] consumes them.

use crate::bitio::LittleEndianValue;
use crate::write::error::{WriteError, WriteResult};

/// Writes fixed-width bit fields MSB-first, the inverse of [`crate::bitio::BitReader`].
///
/// The writer accumulates bits into an in-progress byte and flushes a completed
/// byte to an internal buffer every eight bits. [`BitWriter::into_bytes`] returns
/// the buffer, zero-padding any trailing partial byte so the result is whole bytes.
/// That zero padding is the `byte_alignment()` rule (AV2 v1.0.0 § 5.2.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-4`) — all-zero bits — and is
/// what [`BitWriter::align_to_byte`] and [`BitWriter::into_bytes`] emit.
///
/// This is **not** `trailing_bits()` (AV2 v1.0.0 § 5.2.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3`), which writes a
/// `trailing_one_bit == 1` marker *before* the zero padding. OBU payload tails that
/// end with `trailing_bits()` must therefore write that marker `1` bit first (via
/// [`BitWriter::write_bit`]) — `align_to_byte` / `into_bytes` alone would produce an
/// invalid tail. A dedicated `trailing_bits` helper lands with the OBU-header/size
/// writer; this module covers only zero-pad alignment.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    /// Completed bytes, in emission order.
    bytes: Vec<u8>,
    /// Bits accumulated into the in-progress byte, packed toward the LSB; the most
    /// significant of the `nbits` low bits is the first bit written.
    current: u8,
    /// Number of bits held in `current` (`0..=7`); `0` means no partial byte.
    nbits: u8,
}

impl BitWriter {
    /// Creates an empty bit writer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            nbits: 0,
        }
    }

    /// Creates an empty bit writer with room for at least `bytes` completed bytes.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            current: 0,
            nbits: 0,
        }
    }

    /// Returns `true` if the writer is positioned on a byte boundary.
    #[must_use]
    pub const fn is_byte_aligned(&self) -> bool {
        self.nbits == 0
    }

    /// Returns the number of bits written so far.
    #[must_use]
    pub fn bit_len(&self) -> u64 {
        (self.bytes.len() as u64)
            .saturating_mul(8)
            .saturating_add(u64::from(self.nbits))
    }

    /// Writes a single bit (MSB-first).
    ///
    /// # Errors
    /// Returns [`WriteError::ValueTooWide`] if `bit` is not `0` or `1`.
    pub fn write_bit(&mut self, bit: u8) -> WriteResult<()> {
        if bit > 1 {
            return Err(WriteError::ValueTooWide {
                value: u64::from(bit),
                width_bits: 1,
            });
        }
        self.current = (self.current << 1) | bit;
        self.nbits += 1;
        if self.nbits == 8 {
            self.bytes.push(self.current);
            self.current = 0;
            self.nbits = 0;
        }
        Ok(())
    }

    /// Writes a boolean flag as a single bit, the inverse of
    /// [`crate::bitio::BitReader::read_flag`] and the boolean spelling of
    /// [`BitWriter::write_bit`] for the AV2 `f(1)` flag idiom.
    ///
    /// `true` emits a `1` bit and `false` emits a `0` bit.
    ///
    /// # Errors
    /// Never fails (the encoded bit is always `0` or `1`); returns [`WriteResult`]
    /// for symmetry with the other primitives.
    pub fn write_flag(&mut self, flag: bool) -> WriteResult<()> {
        self.write_bit(u8::from(flag))
    }

    /// Writes the low `n` bits of `value` (MSB-first), the inverse of
    /// [`crate::bitio::BitReader::read_bits`].
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] if `n > 32`, or
    /// [`WriteError::ValueTooWide`] if `value` does not fit in `n` bits.
    pub fn write_bits(&mut self, value: u32, n: u32) -> WriteResult<()> {
        if n > 32 {
            return Err(WriteError::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }
        if n < 32 && value >= (1u32 << n) {
            return Err(WriteError::ValueTooWide {
                value: u64::from(value),
                width_bits: n,
            });
        }
        for i in (0..n).rev() {
            self.write_bit(((value >> i) & 1) as u8)?;
        }
        Ok(())
    }

    /// Writes the low `n` bits of a `u8` value (MSB-first), the inverse of
    /// [`crate::bitio::BitReader::read_bits_u8`].
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] if `n > 8`, or
    /// [`WriteError::ValueTooWide`] if `value` does not fit in `n` bits.
    pub fn write_bits_u8(&mut self, value: u8, n: u32) -> WriteResult<()> {
        if n > 8 {
            return Err(WriteError::BitWidthTooLarge {
                requested: n,
                max: 8,
            });
        }
        self.write_bits(u32::from(value), n)
    }

    /// Writes an AV2 `su(n)` signed integer descriptor (AV2 v1.0.0 § 4.11.7,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-7`), the
    /// inverse of [`crate::bitio::BitReader::read_su`].
    ///
    /// The value is encoded as its `n`-bit two's-complement pattern, MSB-first.
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] if `n == 0` or `n > 32`, or
    /// [`WriteError::ValueOutOfRange`] if `value` is outside the signed range
    /// `-(2^(n-1)) ..= 2^(n-1) - 1`.
    pub fn write_su(&mut self, value: i32, n: u32) -> WriteResult<()> {
        if n == 0 || n > 32 {
            return Err(WriteError::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }
        let sign_mask = 1i64 << (n - 1);
        let lo = -sign_mask;
        let hi = sign_mask - 1;
        let value = i64::from(value);
        if value < lo || value > hi {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "su",
                value,
            });
        }
        let field_mask: i64 = if n == 32 {
            i64::from(u32::MAX)
        } else {
            (1i64 << n) - 1
        };
        let coded = (value & field_mask) as u32;
        self.write_bits(coded, n)
    }

    /// Writes an AV2 `uvlc()` descriptor (AV2 v1.0.0 § 4.11.3,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-3`), the inverse of
    /// [`crate::bitio::BitReader::read_uvlc`].
    ///
    /// # Errors
    /// Returns [`WriteError::ValueOutOfRange`] if `value == u32::MAX`, which would
    /// require 32 leading zero bits — an AV2 conformance violation the reader never
    /// produces.
    pub fn write_uvlc(&mut self, value: u32) -> WriteResult<()> {
        let m = u64::from(value) + 1;
        if m > u64::from(u32::MAX) {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(value),
            });
        }
        let m = m as u32;
        let leading_zeros = m.ilog2();
        self.write_bits(0, leading_zeros)?;
        self.write_bit(1)?;
        let suffix = m - (1u32 << leading_zeros);
        self.write_bits(suffix, leading_zeros)
    }

    /// Writes an AV2 `svlc()` signed variable-length descriptor (AV2 v1.0.0
    /// § 4.11.4, `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-4`), the inverse of
    /// [`crate::bitio::BitReader::read_svlc`].
    ///
    /// # Errors
    /// Returns [`WriteError::ValueOutOfRange`] if `value == i32::MIN`, whose
    /// magnitude exceeds the `uvlc()` conformance bound.
    pub fn write_svlc(&mut self, value: i32) -> WriteResult<()> {
        let v = if value > 0 {
            2 * i64::from(value) - 1
        } else {
            -2 * i64::from(value)
        };
        if v < 0 || v > i64::from(u32::MAX) - 1 {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "svlc",
                value: i64::from(value),
            });
        }
        self.write_uvlc(v as u32)
    }

    /// Writes raw little-endian bytes for an AV2 `le(n)` descriptor (AV2 v1.0.0
    /// § 4.11.5, `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-5`), the inverse of
    /// [`crate::bitio::BitReader::read_le`]. The `n` of the descriptor is the slice
    /// length.
    ///
    /// # Errors
    /// Never fails for whole bytes; returns [`WriteResult`] for symmetry with the
    /// other primitives.
    pub fn write_le(&mut self, bytes: &[u8]) -> WriteResult<()> {
        for &byte in bytes {
            self.write_bits_u8(byte, 8)?;
        }
        Ok(())
    }

    /// Writes a [`LittleEndianValue`] produced by [`crate::bitio::BitReader::read_le`].
    ///
    /// # Errors
    /// Propagates [`WriteError`] from [`BitWriter::write_le`] (never fails today).
    pub fn write_le_value(&mut self, value: &LittleEndianValue) -> WriteResult<()> {
        self.write_le(value.as_le_bytes())
    }

    /// Writes an AV2 `le(n)` descriptor from a `u64` (AV2 v1.0.0 § 4.11.5,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-5`), the inverse of
    /// [`crate::bitio::BitReader::read_le_u64`].
    ///
    /// # Errors
    /// Returns [`WriteError::ByteWidthTooLarge`] if `n > 8`, or
    /// [`WriteError::ValueTooWide`] if `value` does not fit in `n` bytes.
    pub fn write_le_u64(&mut self, value: u64, n: u32) -> WriteResult<()> {
        if n > 8 {
            return Err(WriteError::ByteWidthTooLarge {
                requested: n,
                max: 8,
            });
        }
        if n < 8 {
            let width_bits = 8 * n;
            if value >= (1u64 << width_bits) {
                return Err(WriteError::ValueTooWide { value, width_bits });
            }
        }
        for i in 0..n {
            let byte = ((value >> (8 * i)) & 0xff) as u8;
            self.write_bits_u8(byte, 8)?;
        }
        Ok(())
    }

    /// Writes an AV2 `leb128()` descriptor (AV2 v1.0.0 § 4.11.6,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-6`) in its canonical
    /// minimal-length form, the inverse of [`crate::bitio::BitReader::read_leb128`].
    ///
    /// Every `u32` is encodable (in one to five bytes), so this never fails. The
    /// reader accepts non-minimal encodings too, so byte-exact round-tripping is
    /// only guaranteed for canonically-encoded inputs; `read(write(x)) == x` always
    /// holds.
    ///
    /// # Errors
    /// Never fails; returns [`WriteResult`] for symmetry with the other primitives.
    pub fn write_leb128(&mut self, value: u32) -> WriteResult<()> {
        let mut remaining = value;
        loop {
            let mut byte = (remaining & 0x7f) as u8;
            remaining >>= 7;
            if remaining != 0 {
                byte |= 0x80;
            }
            self.write_bits_u8(byte, 8)?;
            if remaining == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Writes an AV2 `ns(n)` non-symmetric integer descriptor (AV2 v1.0.0 § 4.11.8,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-8`), the inverse of
    /// [`crate::bitio::BitReader::read_ns`].
    ///
    /// # Errors
    /// Returns [`WriteError::ZeroWidth`] if `n == 0`, or
    /// [`WriteError::ValueOutOfRange`] if `value >= n` (the descriptor encodes
    /// `0 ..= n - 1`).
    #[allow(clippy::many_single_char_names)]
    pub fn write_ns(&mut self, value: u32, n: u32) -> WriteResult<()> {
        if n == 0 {
            return Err(WriteError::ZeroWidth { descriptor: "ns" });
        }
        if value >= n {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: i64::from(value),
            });
        }
        let w = u32::BITS - n.leading_zeros();
        let m = (1u64 << w) - u64::from(n);
        let value = u64::from(value);
        if value < m {
            self.write_bits(value as u32, w - 1)
        } else {
            let t = value + m;
            let v = (t >> 1) as u32;
            let extra = (t & 1) as u8;
            self.write_bits(v, w - 1)?;
            self.write_bit(extra)
        }
    }

    /// Writes an AV2 `tu(mx)` truncated-unary descriptor (AV2 v1.0.0 § 4.11.9,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-9`), the inverse of the `read_tu`
    /// reader helper: `value` `1`-bits followed by a terminating `0`-bit, except that the
    /// final `0` is omitted when `value == mx` (the all-ones form, the descriptor's maximum).
    ///
    /// # Errors
    /// Returns [`WriteError::ValueOutOfRange`] if `value > mx` (the descriptor encodes
    /// `0 ..= mx`), checked before any bit is written.
    pub fn write_tu(&mut self, value: u32, mx: u32) -> WriteResult<()> {
        if value > mx {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "tu",
                value: i64::from(value),
            });
        }
        for _ in 0..value {
            self.write_bit(1)?;
        }
        if value < mx {
            self.write_bit(0)?;
        }
        Ok(())
    }

    /// Writes an AV2 `rg(n)` Rice-Golomb descriptor (AV2 v1.0.0 § 4.11.10,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-10`), the inverse of
    /// [`crate::bitio::BitReader::read_rg`]: a unary quotient prefix of
    /// `value >> n` one bits, a terminating zero bit, then an `n`-bit remainder.
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] if `n > 32`, or
    /// [`WriteError::ValueOutOfRange`] if `value >> n >= 32` (the reader's unary
    /// prefix must terminate within 32 bits).
    pub fn write_rg(&mut self, value: u32, n: u32) -> WriteResult<()> {
        if n > 32 {
            return Err(WriteError::BitWidthTooLarge {
                requested: n,
                max: 32,
            });
        }
        let (quotient, remainder) = if n == 32 {
            (0u32, value)
        } else {
            (value >> n, value & ((1u32 << n) - 1))
        };
        if quotient >= 32 {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "rg",
                value: i64::from(value),
            });
        }
        for _ in 0..quotient {
            self.write_bit(1)?;
        }
        self.write_bit(0)?;
        self.write_bits(remainder, n)
    }

    /// Pads the in-progress byte to the next byte boundary with zero bits, the
    /// inverse of [`crate::bitio::BitReader::byte_align_zero`] (AV2 v1.0.0 § 5.2.4 /
    /// § 6.2.4, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-4`). This is
    /// `byte_alignment()`, not `trailing_bits()` (§ 5.2.3) — see the type-level docs.
    /// A no-op when already byte-aligned.
    pub fn align_to_byte(&mut self) {
        if self.nbits != 0 {
            let pad = 8 - u32::from(self.nbits);
            self.bytes.push(self.current << pad);
            self.current = 0;
            self.nbits = 0;
        }
    }

    /// Writes AV2 `trailing_bits(nbBits)` (AV2 v1.0.0 § 5.2.3,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3`): a single
    /// `trailing_one_bit == 1` followed by `nb_bits - 1` `trailing_zero_bit == 0`
    /// bits, the inverse of [`crate::obu::parse_trailing_bits`].
    ///
    /// Unlike [`BitWriter::align_to_byte`] — which emits `byte_alignment()`
    /// (§ 5.2.4, all-zero padding) — `trailing_bits()` writes the leading `1` marker
    /// bit first; the two are not interchangeable for an OBU payload tail.
    ///
    /// # Errors
    /// Returns [`WriteError::EmptyTrailingBits`] if `nb_bits == 0` (the parser
    /// rejects an empty trailing-bits field).
    pub fn write_trailing_bits(&mut self, nb_bits: u64) -> WriteResult<()> {
        if nb_bits == 0 {
            return Err(WriteError::EmptyTrailingBits);
        }
        self.write_bit(1)?;
        for _ in 1..nb_bits {
            self.write_bit(0)?;
        }
        Ok(())
    }

    /// Appends every bit `other` holds — its completed bytes and any in-progress partial
    /// byte — onto this writer, MSB-first, preserving the exact bit sequence regardless of
    /// either writer's current alignment.
    ///
    /// This is the "commit" step of the scratch-writer pattern: a composing writer drafts a
    /// whole structure into a local [`BitWriter`], and on full success appends that draft to
    /// the caller's writer. When a draft step fails the caller's writer is never touched, so
    /// the composition is reject-before-write as a whole even though its sub-writers each
    /// validate independently.
    ///
    /// # Errors
    /// Propagates [`WriteError`] from the underlying [`BitWriter::write_bit`] (never fails
    /// for a `0`/`1` bit, so this returns `Ok` for any well-formed `other`).
    pub fn append(&mut self, other: &BitWriter) -> WriteResult<()> {
        for &byte in &other.bytes {
            self.write_bits_u8(byte, 8)?;
        }
        for i in (0..other.nbits).rev() {
            self.write_bit((other.current >> i) & 1)?;
        }
        Ok(())
    }

    /// Consumes the writer and returns the written bytes, zero-padding any trailing
    /// partial byte (see the type-level note on alignment).
    #[must_use]
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.bytes
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::span::ByteOffset;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    #[test]
    fn writes_msb_first_matching_the_reader_doctest() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        writer.write_bits(0b011, 3).unwrap();
        writer.write_bits(0b0010, 4).unwrap();
        assert!(writer.is_byte_aligned());
        let bytes = writer.into_bytes();
        assert_eq!(bytes, vec![0b1011_0010]);
    }

    #[test]
    fn write_flag_emits_single_bit_and_round_trips() {
        let mut writer = BitWriter::new();
        writer.write_flag(true).unwrap();
        writer.write_flag(false).unwrap();
        assert_eq!(writer.bit_len(), 2);
        let bytes = writer.into_bytes();
        assert_eq!(bytes, vec![0b1000_0000]);
        let mut w2 = BitWriter::new();
        for f in [true, false, true] {
            w2.write_flag(f).unwrap();
        }
        let w2_bytes = w2.into_bytes();
        let mut r = reader(&w2_bytes);
        assert!(r.read_flag().unwrap());
        assert!(!r.read_flag().unwrap());
        assert!(r.read_flag().unwrap());
    }

    #[test]
    fn write_bit_rejects_values_above_one() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_bit(2),
            Err(WriteError::ValueTooWide {
                value: 2,
                width_bits: 1
            })
        ));
    }

    #[test]
    fn write_bits_rejects_wide_widths_and_values() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_bits(0, 33),
            Err(WriteError::BitWidthTooLarge {
                requested: 33,
                max: 32
            })
        ));
        assert!(matches!(
            writer.write_bits(0b1_0000, 4),
            Err(WriteError::ValueTooWide {
                value: 16,
                width_bits: 4
            })
        ));
        assert!(writer.write_bits(0, 0).is_ok());
        assert!(matches!(
            writer.write_bits(1, 0),
            Err(WriteError::ValueTooWide { .. })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn align_to_byte_zero_pads_the_partial_byte() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        assert!(!writer.is_byte_aligned());
        assert_eq!(writer.bit_len(), 1);
        writer.align_to_byte();
        assert!(writer.is_byte_aligned());
        assert_eq!(writer.into_bytes(), vec![0b1000_0000]);
    }

    #[test]
    fn write_leb128_emits_canonical_encodings() {
        for (value, expected) in [
            (0u32, vec![0x00]),
            (127, vec![0x7F]),
            (128, vec![0x80, 0x01]),
            (624_485, vec![0xE5, 0x8E, 0x26]),
            (u32::MAX, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        ] {
            let mut writer = BitWriter::new();
            writer.write_leb128(value).unwrap();
            assert_eq!(writer.into_bytes(), expected, "leb128({value})");
        }
    }

    #[test]
    fn write_su_rejects_out_of_range_and_invalid_widths() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_su(8, 4),
            Err(WriteError::ValueOutOfRange {
                descriptor: "su",
                value: 8
            })
        ));
        assert!(matches!(
            writer.write_su(-9, 4),
            Err(WriteError::ValueOutOfRange { .. })
        ));
        assert!(matches!(
            writer.write_su(0, 0),
            Err(WriteError::BitWidthTooLarge { .. })
        ));
        assert!(matches!(
            writer.write_su(0, 33),
            Err(WriteError::BitWidthTooLarge { .. })
        ));
    }

    #[test]
    fn write_uvlc_rejects_the_unencodable_max() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_uvlc(u32::MAX),
            Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                ..
            })
        ));
        assert!(writer.write_uvlc(u32::MAX - 1).is_ok());
    }

    #[test]
    fn write_svlc_rejects_i32_min() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_svlc(i32::MIN),
            Err(WriteError::ValueOutOfRange {
                descriptor: "svlc",
                ..
            })
        ));
        assert!(writer.write_svlc(i32::MIN + 1).is_ok());
    }

    #[test]
    fn write_le_u64_rejects_too_many_bytes_and_too_wide_values() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_le_u64(0, 9),
            Err(WriteError::ByteWidthTooLarge {
                requested: 9,
                max: 8
            })
        ));
        assert!(matches!(
            writer.write_le_u64(0x1_00, 1),
            Err(WriteError::ValueTooWide {
                value: 256,
                width_bits: 8
            })
        ));
    }

    #[test]
    fn write_ns_rejects_zero_width_and_out_of_range() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_ns(0, 0),
            Err(WriteError::ZeroWidth { descriptor: "ns" })
        ));
        assert!(matches!(
            writer.write_ns(5, 5),
            Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: 5
            })
        ));
    }

    #[test]
    fn write_rg_rejects_wide_widths_and_non_terminating_quotients() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_rg(0, 33),
            Err(WriteError::BitWidthTooLarge {
                requested: 33,
                max: 32
            })
        ));
        assert!(matches!(
            writer.write_rg(128, 2),
            Err(WriteError::ValueOutOfRange {
                descriptor: "rg",
                value: 128
            })
        ));
        let mut max = BitWriter::new();
        max.write_rg(127, 2).unwrap();
        assert_eq!(max.into_bytes(), vec![0xFF, 0xFF, 0xFF, 0xFE, 0xC0]);
    }

    #[test]
    fn ns_round_trips_a_non_power_of_two_range() {
        for value in 0u32..5 {
            let mut writer = BitWriter::new();
            writer.write_ns(value, 5).unwrap();
            let bytes = writer.into_bytes();
            assert_eq!(reader(&bytes).read_ns(5).unwrap(), value, "ns({value}, 5)");
        }
    }

    /// Decodes a `tu(mx)` value MSB-first, mirroring `read_tu` in
    /// `crate::headers::frame::restoration` (private there), so the writer can be
    /// round-tripped here without re-exporting it.
    fn read_tu(r: &mut BitReader<'_>, mx: u32) -> u32 {
        for idx in 0..mx {
            if r.read_bit().unwrap() == 0 {
                return idx;
            }
        }
        mx
    }

    #[test]
    fn write_tu_round_trips_and_rejects_out_of_range() {
        for mx in [0u32, 1, 3, 7, 31] {
            for value in 0..=mx {
                let mut writer = BitWriter::new();
                writer.write_tu(value, mx).unwrap();
                let bytes = writer.into_bytes();
                assert_eq!(read_tu(&mut reader(&bytes), mx), value, "tu({value}, {mx})");
            }
        }
        let mut max = BitWriter::new();
        max.write_tu(7, 7).unwrap();
        assert_eq!(max.bit_len(), 7);
        let mut writer = BitWriter::new();
        assert!(matches!(
            writer.write_tu(8, 7),
            Err(WriteError::ValueOutOfRange {
                descriptor: "tu",
                value: 8
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn append_preserves_bits_across_alignment() {
        let mut scratch = BitWriter::new();
        scratch.write_bit(1).unwrap();
        scratch.write_bits(0b010, 3).unwrap();
        scratch.write_bits(0b1, 1).unwrap(); // 5 bits total, partial byte

        let mut dest = BitWriter::new();
        dest.write_bits(0b11, 2).unwrap(); // dest starts mid-byte (2 bits)
        dest.append(&scratch).unwrap();

        let bytes = dest.into_bytes();
        let mut r = reader(&bytes);
        assert_eq!(r.read_bits(2).unwrap(), 0b11);
        assert_eq!(r.read_bit().unwrap(), 1);
        assert_eq!(r.read_bits(3).unwrap(), 0b010);
        assert_eq!(r.read_bit().unwrap(), 1);
    }

    #[test]
    fn append_empty_scratch_is_a_noop() {
        let mut dest = BitWriter::new();
        dest.write_bits(0b101, 3).unwrap();
        let before = dest.bit_len();
        dest.append(&BitWriter::new()).unwrap();
        assert_eq!(dest.bit_len(), before);
    }

    #[test]
    fn trailing_bits_round_trip_and_reject_empty() {
        assert!(matches!(
            BitWriter::new().write_trailing_bits(0),
            Err(WriteError::EmptyTrailingBits)
        ));
        for nb in 1u64..=16 {
            let mut writer = BitWriter::new();
            writer.write_trailing_bits(nb).unwrap();
            let bytes = writer.into_bytes();
            let mut r = reader(&bytes);
            crate::obu::parse_trailing_bits(&mut r, nb).unwrap();
        }
        let mut one = BitWriter::new();
        one.write_trailing_bits(1).unwrap();
        assert_eq!(one.into_bytes(), vec![0b1000_0000]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn read_back(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    /// Decodes a `tu(mx)` value MSB-first, mirroring the private `read_tu` reader helper.
    fn read_tu(r: &mut BitReader<'_>, mx: u32) -> u32 {
        for idx in 0..mx {
            if r.read_bit().unwrap() == 0 {
                return idx;
            }
        }
        mx
    }

    proptest! {
        /// `f(n)`: `read_bits(write_bits(x, n)) == x` for every valid value.
        #[test]
        fn roundtrip_bits(raw in any::<u32>(), n in 0u32..=32) {
            let value = if n == 32 { raw } else { raw & ((1u32 << n) - 1) };
            let mut writer = BitWriter::new();
            writer.write_bits(value, n).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_bits(n).unwrap(), value);
        }

        /// `su(n)`: round-trips across the full signed range, including `n == 32`.
        #[test]
        fn roundtrip_su(raw in any::<i64>(), n in 1u32..=32) {
            let sign_mask = 1i64 << (n - 1);
            let lo = -sign_mask;
            let hi = sign_mask - 1;
            let span = hi - lo + 1;
            let value = (lo + raw.rem_euclid(span)) as i32;
            let mut writer = BitWriter::new();
            writer.write_su(value, n).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_su(n).unwrap(), value);
        }

        /// `uvlc()`: round-trips every encodable value (all but `u32::MAX`).
        #[test]
        fn roundtrip_uvlc(value in 0u32..=(u32::MAX - 1)) {
            let mut writer = BitWriter::new();
            writer.write_uvlc(value).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_uvlc().unwrap(), value);
        }

        /// `svlc()`: round-trips every encodable value (all but `i32::MIN`).
        #[test]
        fn roundtrip_svlc(value in (i32::MIN + 1)..=i32::MAX) {
            let mut writer = BitWriter::new();
            writer.write_svlc(value).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_svlc().unwrap(), value);
        }

        /// `le(n) -> u64`: round-trips every value that fits in `n` bytes.
        #[test]
        fn roundtrip_le_u64(raw in any::<u64>(), n in 0u32..=8) {
            let value = if n == 8 { raw } else { raw & ((1u64 << (8 * n)) - 1) };
            let mut writer = BitWriter::new();
            writer.write_le_u64(value, n).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_le_u64(n).unwrap(), value);
        }

        /// `leb128()`: round-trips every `u32`.
        #[test]
        fn roundtrip_leb128(value in any::<u32>()) {
            let mut writer = BitWriter::new();
            writer.write_leb128(value).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_leb128().unwrap(), value);
        }

        /// `ns(n)`: round-trips values in `0..n` across the full width domain,
        /// including the high `w == 32` paths where `n` has its top bit set
        /// (`n >= 2^31`, up to `u32::MAX`).
        #[test]
        fn roundtrip_ns(
            n in prop_oneof![1u32..=100_000, (1u32 << 31)..=u32::MAX],
            frac in 0u32..=u32::MAX,
        ) {
            let value = ((u64::from(frac) * u64::from(n)) >> 32) as u32;
            let value = value.min(n - 1);
            let mut writer = BitWriter::new();
            writer.write_ns(value, n).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_ns(n).unwrap(), value);
        }

        /// `rg(n)`: round-trips across the full width domain `0..=32`, exercising the
        /// widths above 20 and the `n == 32` special branch (quotient always 0).
        #[test]
        fn roundtrip_rg(quotient in 0u32..32, remainder_bits in any::<u32>(), n in 0u32..=32) {
            let remainder = match n {
                0 => 0,
                32 => remainder_bits,
                _ => remainder_bits & ((1u32 << n) - 1),
            };
            let value = (u64::from(quotient) << n)
                .checked_add(u64::from(remainder))
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(remainder);
            let mut writer = BitWriter::new();
            writer.write_rg(value, n).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_back(&bytes).read_rg(n).unwrap(), value);
        }

        /// `tu(mx)`: `read_tu(write_tu(x, mx)) == x` for every value in `0..=mx` across a
        /// range of maxima (including `mx == 0` and the all-ones terminal value `mx`).
        #[test]
        fn roundtrip_tu(mx in 0u32..=64, frac in 0u32..=u32::MAX) {
            let value = ((u64::from(frac) * (u64::from(mx) + 1)) >> 32) as u32;
            let value = value.min(mx);
            let mut writer = BitWriter::new();
            writer.write_tu(value, mx).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_tu(&mut read_back(&bytes), mx), value);
        }

        /// `trailing_bits(nbBits)`: a `1` marker then zeros reparses cleanly for any
        /// width, the inverse of `crate::obu::parse_trailing_bits`.
        #[test]
        fn roundtrip_trailing_bits(nb in 1u64..=512) {
            let mut writer = BitWriter::new();
            writer.write_trailing_bits(nb).unwrap();
            let bytes = writer.into_bytes();
            let mut r = read_back(&bytes);
            prop_assert!(crate::obu::parse_trailing_bits(&mut r, nb).is_ok());
        }

        /// `append`: concatenating two writers reads back as the two bit sequences in order,
        /// regardless of either writer's alignment.
        #[test]
        fn roundtrip_append(
            a_bits in proptest::collection::vec(0u8..=1, 0..40),
            b_bits in proptest::collection::vec(0u8..=1, 0..40),
        ) {
            let mut dest = BitWriter::new();
            for &b in &a_bits {
                dest.write_bit(b).unwrap();
            }
            let mut scratch = BitWriter::new();
            for &b in &b_bits {
                scratch.write_bit(b).unwrap();
            }
            dest.append(&scratch).unwrap();
            let bytes = dest.into_bytes();
            let mut r = read_back(&bytes);
            for &expected in a_bits.iter().chain(b_bits.iter()) {
                prop_assert_eq!(r.read_bit().unwrap(), expected);
            }
        }

        /// The writer never panics, whatever the value/width — it returns `Result`.
        #[test]
        fn writer_never_panics(value in any::<u32>(), signed in any::<i32>(), n in 0u32..=64) {
            let mut writer = BitWriter::new();
            let _ = writer.write_bits(value, n);
            let _ = writer.write_bits_u8(value as u8, n);
            let _ = writer.write_su(signed, n);
            let _ = writer.write_uvlc(value);
            let _ = writer.write_svlc(signed);
            let _ = writer.write_le_u64(u64::from(value), n);
            let _ = writer.write_leb128(value);
            let _ = writer.write_ns(value, n);
            let _ = writer.write_rg(value, n.min(32));
            let tu_value = value % 130;
            let _ = writer.write_tu(tu_value, 64);
            writer.align_to_byte();
        }
    }
}
