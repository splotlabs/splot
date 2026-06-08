// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 padding OBU syntax model (AV2 v1.0.0 § 5.16 / § 6.15).
//!
//! `padding_obu()` carries `obu_padding_byte` values whose length is *not* coded; it is
//! derived from the OBU size minus the trailing bytes. AV2 § 5.16 defines the last byte
//! of valid content as the last byte that is not equal to zero, with `trailing_bits()`
//! running from there to the payload end. This rule prevents systems that interpret
//! trailing zero bytes as continuation from dropping valid bytes, so a non-empty padding
//! payload must contain at least one non-zero byte.
//!
//! An `obuPayloadSize` of 0 is legal (no padding bytes, no trailing bits); an
//! `obuPayloadSize` of 1 is legal (no padding bytes, one byte of `trailing_bits()`), so
//! any OBU can be converted into a padding OBU in place.

use crate::bitio::BitReader;
use crate::error::{Error, PaddingErrorKind, Result};
use crate::obu::parse_trailing_bits;
use crate::span::{BitOffset, ByteOffset};

/// Parsed `padding_obu()` syntax (AV2 v1.0.0 § 5.16).
///
/// The payload is split at the last non-zero byte: `padding_len` bytes of
/// `obu_padding_byte` followed by `trailing_len` bytes of `trailing_bits()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaddingObu {
    /// Number of leading `obu_padding_byte` values (the bytes before the last non-zero
    /// byte). These may take arbitrary values (AV2 § 6.15).
    pub padding_len: usize,
    /// Number of bytes occupied by `trailing_bits()` (from the last non-zero byte
    /// through the payload end). `0` only for an empty payload.
    pub trailing_len: usize,
}

/// Parses `padding_obu()` from `payload` (AV2 v1.0.0 § 5.16 / § 6.15).
///
/// `payload` is the OBU payload (`obuPayloadSize` bytes) and `payload_offset` is its
/// absolute start offset. The parser consumes the entire payload — the leading
/// `obu_padding_byte` values and the `trailing_bits()` that begin at the last non-zero
/// byte — so the caller must NOT additionally run the generic OBU trailing-bits logic
/// for `OBU_PADDING` (that would double-consume the trailing bits).
///
/// # Errors
/// Returns [`Error::InvalidPadding`] with [`PaddingErrorKind::AllZeroPayload`] for a
/// non-empty all-zero payload, or [`PaddingErrorKind::InvalidTrailingBits`] if the bytes
/// from the last non-zero byte are not a valid `trailing_bits()` pattern.
pub fn parse_padding_obu(payload: &[u8], payload_offset: ByteOffset) -> Result<PaddingObu> {
    // AV2 § 5.16: an obuPayloadSize of 0 is legal and contains no trailing bits.
    if payload.is_empty() {
        return Ok(PaddingObu {
            padding_len: 0,
            trailing_len: 0,
        });
    }

    // AV2 § 5.16 / § 6.15: the last byte of valid content is the last non-zero byte. A
    // payload with no non-zero byte has no trailing_bits() byte, which the spec forbids.
    let Some(last_nonzero) = payload.iter().rposition(|&byte| byte != 0) else {
        return Err(Error::InvalidPadding {
            offset: payload_offset,
            bit_offset: BitOffset::from_bits(0),
            kind: PaddingErrorKind::AllZeroPayload,
        });
    };

    let padding_len = last_nonzero;
    let trailing = &payload[padding_len..];
    let trailing_len = trailing.len();
    let trailing_offset = payload_offset.saturating_add(padding_len as u64);

    // The trailing region starts at the last non-zero byte and runs to the payload end.
    // Its bytes are present (sliced from payload), so trailing_bits() never reports EOF
    // here; only a malformed pattern is possible.
    let mut reader = BitReader::new(trailing, trailing_offset);
    let nb_bits = (trailing_len as u64).saturating_mul(8);
    if let Err(error) = parse_trailing_bits(&mut reader, nb_bits) {
        let (offset, bit_offset) = match error {
            Error::InvalidTrailingBits {
                offset, bit_offset, ..
            } => (offset, bit_offset),
            // trailing_bits() over a non-empty slice can only fail with InvalidTrailingBits.
            other => return Err(other),
        };
        return Err(Error::InvalidPadding {
            offset,
            bit_offset,
            kind: PaddingErrorKind::InvalidTrailingBits,
        });
    }

    Ok(PaddingObu {
        padding_len,
        trailing_len,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn padding_empty_payload_is_valid() {
        let padding = parse_padding_obu(&[], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 0);
        assert_eq!(padding.trailing_len, 0);
    }

    #[test]
    fn padding_one_byte_trailing_only_is_valid() {
        // A single 0x80 byte is valid trailing_bits() (trailing_one_bit then zeros).
        let padding = parse_padding_obu(&[0x80], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 0);
        assert_eq!(padding.trailing_len, 1);
    }

    #[test]
    fn padding_arbitrary_bytes_before_trailing_bits_are_valid() {
        // Three arbitrary padding bytes, then a trailing-bits byte (0x80).
        let padding = parse_padding_obu(&[0xDE, 0xAD, 0xBE, 0x80], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 3);
        assert_eq!(padding.trailing_len, 1);
    }

    #[test]
    fn padding_trailing_byte_may_be_followed_by_zero_padding_bytes() {
        // The last non-zero byte is the trailing-bits byte; zero bytes after it would be
        // dropped, so the last non-zero byte (0x80) must itself be valid trailing_bits.
        let padding = parse_padding_obu(&[0xFF, 0x80], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 1);
        assert_eq!(padding.trailing_len, 1);
    }

    #[test]
    fn padding_all_zero_payload_is_rejected() {
        assert!(matches!(
            parse_padding_obu(&[0x00, 0x00, 0x00], ByteOffset::new(1)),
            Err(Error::InvalidPadding {
                kind: PaddingErrorKind::AllZeroPayload,
                ..
            })
        ));
    }

    #[test]
    fn padding_invalid_trailing_bits_is_rejected() {
        // Last non-zero byte 0x40 = 0b0100_0000: trailing_one_bit must be 1, but the
        // first bit is 0.
        assert!(matches!(
            parse_padding_obu(&[0x40], ByteOffset::new(1)),
            Err(Error::InvalidPadding {
                kind: PaddingErrorKind::InvalidTrailingBits,
                ..
            })
        ));
        // A trailing region whose later bits are not zero: 0xC0 = 0b1100_0000.
        assert!(matches!(
            parse_padding_obu(&[0xC0], ByteOffset::new(1)),
            Err(Error::InvalidPadding {
                kind: PaddingErrorKind::InvalidTrailingBits,
                ..
            })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// The padding parser must never panic on arbitrary input.
        #[test]
        fn padding_parser_never_panics(data in proptest::collection::vec(any::<u8>(), 0..128)) {
            let _ = parse_padding_obu(&data, ByteOffset::new(0));
        }
    }
}
