// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! AV2 LEB128 unsigned-integer parsing (AV2 v1.0.0 § 4.11.6).
//!
//! LEB128 is byte-aligned, uses at most 8 bytes, and the decoded value must be
//! `<= (1 << 32) - 1`. Non-minimal encodings are allowed by the spec.

use crate::error::{Error, Result};
use crate::span::ByteOffset;

/// A decoded LEB128 value together with the number of bytes it occupied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leb128 {
    /// The decoded unsigned value (AV2 requires `<= (1 << 32) - 1`).
    pub value: u32,
    /// Number of bytes consumed (`1..=8`).
    pub bytes_read: u8,
}

/// Reads a LEB128 value from `input` starting at absolute offset `start`
/// (AV2 v1.0.0 § 4.11.6).
///
/// # Errors
/// - [`Error::UnexpectedEof`] if the input ends before a terminating byte.
/// - [`Error::InvalidLeb128`] if more than 8 bytes would be required (the MSB of
///   byte 7 is set) or the decoded value exceeds `(1 << 32) - 1`.
pub fn read_leb128(input: &[u8], start: ByteOffset) -> Result<Leb128> {
    let start_idx = usize::try_from(start.get()).map_err(|_| Error::InvalidLeb128 {
        offset: start,
        message: "start offset overflows usize".to_owned(),
    })?;

    let mut value: u64 = 0;
    for i in 0..8u8 {
        let idx = start_idx.saturating_add(usize::from(i));
        let Some(&byte) = input.get(idx) else {
            return Err(Error::UnexpectedEof {
                offset: start.saturating_add(u64::from(i)),
                needed: 1,
            });
        };
        value |= u64::from(byte & 0x7f) << (u32::from(i) * 7);
        if byte & 0x80 == 0 {
            let value = u32::try_from(value).map_err(|_| Error::InvalidLeb128 {
                offset: start,
                message: "value exceeds (1 << 32) - 1".to_owned(),
            })?;
            return Ok(Leb128 {
                value,
                bytes_read: i + 1,
            });
        }
    }

    Err(Error::InvalidLeb128 {
        offset: start.saturating_add(7),
        message: "LEB128 uses more than 8 bytes (MSB of byte 7 is set)".to_owned(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn at_start(bytes: &[u8]) -> Result<Leb128> {
        read_leb128(bytes, ByteOffset::new(0))
    }

    #[test]
    fn single_byte() {
        assert_eq!(
            at_start(&[0x00]).unwrap(),
            Leb128 {
                value: 0,
                bytes_read: 1
            }
        );
        assert_eq!(
            at_start(&[0x7f]).unwrap(),
            Leb128 {
                value: 127,
                bytes_read: 1
            }
        );
    }

    #[test]
    fn multi_byte() {
        assert_eq!(
            at_start(&[0x80, 0x01]).unwrap(),
            Leb128 {
                value: 128,
                bytes_read: 2
            }
        );
        // Classic example: 0xe5 0x8e 0x26 -> 624485.
        assert_eq!(at_start(&[0xe5, 0x8e, 0x26]).unwrap().value, 624_485);
    }

    #[test]
    fn non_minimal_encoding_is_allowed() {
        assert_eq!(
            at_start(&[0x80, 0x00]).unwrap(),
            Leb128 {
                value: 0,
                bytes_read: 2
            }
        );
    }

    #[test]
    fn more_than_eight_bytes_is_error() {
        let nine = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        assert!(matches!(at_start(&nine), Err(Error::InvalidLeb128 { .. })));
    }

    #[test]
    fn value_overflow_is_error() {
        // 0x10 << 28 == 2^32, which exceeds u32::MAX.
        let overflow = [0x80, 0x80, 0x80, 0x80, 0x10];
        assert!(matches!(
            at_start(&overflow),
            Err(Error::InvalidLeb128 { .. })
        ));
    }

    #[test]
    fn eof_is_error() {
        assert!(matches!(
            at_start(&[0x80]),
            Err(Error::UnexpectedEof { .. })
        ));
        assert!(matches!(at_start(&[]), Err(Error::UnexpectedEof { .. })));
    }

    #[test]
    fn respects_start_offset() {
        let buf = [0xff, 0x05];
        assert_eq!(
            read_leb128(&buf, ByteOffset::new(1)).unwrap(),
            Leb128 {
                value: 5,
                bytes_read: 1
            }
        );
    }
}
