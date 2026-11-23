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
use crate::obu::parse_trailing_bits_with_reader;
use crate::span::{BitOffset, BitSpan, ByteOffset};
use crate::syntax::{
    NoopObserver, ScopeKind, SyntaxDescriptor, SyntaxErrorRef, SyntaxFieldRead, SyntaxObserver,
    SyntaxReader, SyntaxValue,
};

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
/// byte — so the caller must not additionally run the generic OBU trailing-bits logic
/// for `OBU_PADDING` (that would double-consume the trailing bits).
///
/// # Errors
/// Returns [`Error::InvalidPadding`] with [`PaddingErrorKind::AllZeroPayload`] for a
/// non-empty all-zero payload, or [`PaddingErrorKind::InvalidTrailingBits`] if the bytes
/// from the last non-zero byte are not a valid `trailing_bits()` pattern.
pub fn parse_padding_obu(payload: &[u8], payload_offset: ByteOffset) -> Result<PaddingObu> {
    let mut noop = NoopObserver;
    parse_padding_obu_impl(payload, payload_offset, &mut noop)
}

/// Parses `padding_obu()` (AV2 v1.0.0 § 5.16 / § 6.15) while reporting syntax
/// fields and bit spans to `observer`.
///
/// # Errors
/// Returns [`Error::InvalidPadding`] with [`PaddingErrorKind::AllZeroPayload`] for a
/// non-empty all-zero payload, or [`PaddingErrorKind::InvalidTrailingBits`] if the bytes
/// from the last non-zero byte are not a valid `trailing_bits()` pattern.
pub fn parse_padding_obu_observed(
    payload: &[u8],
    payload_offset: ByteOffset,
    observer: &mut dyn SyntaxObserver,
) -> Result<PaddingObu> {
    parse_padding_obu_impl(payload, payload_offset, &mut { observer })
}

pub(crate) fn parse_padding_obu_impl<O: SyntaxObserver>(
    payload: &[u8],
    payload_offset: ByteOffset,
    observer: &mut O,
) -> Result<PaddingObu> {
    observer.enter_scope(ScopeKind::Structure {
        name: "padding_obu",
        spec_section: Some("5.16"),
    });
    let result = parse_padding_obu_body(payload, payload_offset, observer);
    observer.exit_scope();
    result
}

fn parse_padding_obu_body<O: SyntaxObserver>(
    payload: &[u8],
    payload_offset: ByteOffset,
    observer: &mut O,
) -> Result<PaddingObu> {
    if payload.is_empty() {
        return Ok(PaddingObu {
            padding_len: 0,
            trailing_len: 0,
        });
    }

    let Some(last_nonzero) = payload.iter().rposition(|&byte| byte != 0) else {
        let error = Error::InvalidPadding {
            offset: payload_offset,
            bit_offset: BitOffset::from_bits(0),
            kind: PaddingErrorKind::AllZeroPayload,
        };
        observer.on_error(SyntaxErrorRef {
            bit_offset: 0,
            field: Some("obu_padding_byte"),
            error: &error,
        });
        return Err(error);
    };

    let padding_len = last_nonzero;
    if padding_len > 0 {
        observer.on_field_read(SyntaxFieldRead {
            name: "obu_padding_byte",
            descriptor: SyntaxDescriptor::Bytes(padding_len),
            value: SyntaxValue::Bytes { len: padding_len },
            span: BitSpan::new(0, (padding_len as u64).saturating_mul(8)),
        });
    }

    let trailing = &payload[padding_len..];
    let trailing_len = trailing.len();
    let trailing_offset = payload_offset.saturating_add(padding_len as u64);
    let origin_abs_bit = payload_offset.get().saturating_mul(8);

    let mut reader = BitReader::new(trailing, trailing_offset);
    let nb_bits = (trailing_len as u64).saturating_mul(8);
    let mut sr = SyntaxReader::with_origin(&mut reader, origin_abs_bit, observer);
    if let Err(error) = parse_trailing_bits_with_reader(&mut sr, nb_bits) {
        let (offset, bit_offset) = match error {
            Error::InvalidTrailingBits {
                offset, bit_offset, ..
            } => (offset, bit_offset),
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
        let padding = parse_padding_obu(&[0x80], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 0);
        assert_eq!(padding.trailing_len, 1);
    }

    #[test]
    fn padding_arbitrary_bytes_before_trailing_bits_are_valid() {
        let padding = parse_padding_obu(&[0xDE, 0xAD, 0xBE, 0x80], ByteOffset::new(1)).unwrap();
        assert_eq!(padding.padding_len, 3);
        assert_eq!(padding.trailing_len, 1);
    }

    #[test]
    fn padding_trailing_byte_may_be_followed_by_zero_padding_bytes() {
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
        assert!(matches!(
            parse_padding_obu(&[0x40], ByteOffset::new(1)),
            Err(Error::InvalidPadding {
                kind: PaddingErrorKind::InvalidTrailingBits,
                ..
            })
        ));
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
