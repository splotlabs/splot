// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 Annex B length-delimited bitstream envelope parsing (AV2 v1.0.0 Annex B).
//!
//! ```text
//! bitstream() {
//!     while ( more_data_in_bitstream() ) {
//!         leb128() num_bytes_in_obu;
//!         open_bitstream_unit( num_bytes_in_obu )
//!     }
//! }
//! ```
//!
//! `num_bytes_in_obu` includes the OBU header bytes; the payload is therefore
//! `num_bytes_in_obu - header_size_bytes` (AV2 § 5.2.1 `open_bitstream_unit`).

use crate::error::{Error, Result};
use crate::leb128::read_leb128;
use crate::obu::{ObuHeader, read_obu_header};
use crate::span::ByteOffset;

/// One length-delimited OBU from an Annex B bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObuEnvelope<'a> {
    /// Absolute offset of the OBU header (i.e. just after the length prefix).
    pub offset: ByteOffset,
    /// `num_bytes_in_obu`: total OBU size in bytes (header + payload).
    pub size: u32,
    /// The parsed OBU header.
    pub header: ObuHeader,
    /// The OBU payload (`size - header_size_bytes` bytes).
    pub payload: &'a [u8],
}

/// As much of an Annex B bitstream as could be parsed: every OBU parsed before
/// the first structural error, plus that error (if any).
#[derive(Debug)]
pub struct PartialParse<'a> {
    /// OBUs parsed before the first structural error (or all of them).
    pub obus: Vec<ObuEnvelope<'a>>,
    /// The error that stopped parsing, or `None` if the whole stream parsed.
    pub error: Option<Error>,
}

/// Parses an AV2 Annex B bitstream, keeping every OBU parsed before the first
/// structural error together with that error (AV2 v1.0.0 Annex B § B.2).
///
/// Unlike [`parse_annex_b_obus`], the parseable prefix is never discarded, so a
/// validator can still run checks on the OBUs that precede a later malformed OBU.
/// The parser never panics on malformed input.
#[must_use]
pub fn parse_annex_b_obus_partial(input: &[u8]) -> PartialParse<'_> {
    let mut obus = Vec::new();
    let mut cursor: usize = 0;

    while cursor < input.len() {
        match parse_one_obu(input, cursor) {
            Ok((envelope, next)) => {
                obus.push(envelope);
                cursor = next;
            }
            Err(error) => {
                return PartialParse {
                    obus,
                    error: Some(error),
                };
            }
        }
    }

    PartialParse { obus, error: None }
}

/// Parses a complete AV2 Annex B length-delimited bitstream into OBU envelopes
/// (AV2 v1.0.0 Annex B § B.2).
///
/// # Errors
/// Returns an [`Error`] describing the first malformed length prefix, header, or
/// out-of-range size. The parser never panics on malformed input. Use
/// [`parse_annex_b_obus_partial`] to retain the OBUs parsed before the error.
pub fn parse_annex_b_obus(input: &[u8]) -> Result<Vec<ObuEnvelope<'_>>> {
    let partial = parse_annex_b_obus_partial(input);
    match partial.error {
        Some(error) => Err(error),
        None => Ok(partial.obus),
    }
}

/// Parses a single OBU starting at absolute byte `cursor`, returning the envelope
/// and the cursor position immediately after it.
fn parse_one_obu(input: &[u8], cursor: usize) -> Result<(ObuEnvelope<'_>, usize)> {
    let prefix = read_leb128(input, ByteOffset::new(cursor as u64))?;
    let header_start = cursor.saturating_add(usize::from(prefix.bytes_read));
    let size = prefix.value;

    if size == 0 {
        return Err(Error::ObuSizeOutOfRange {
            offset: ByteOffset::new(cursor as u64),
            size: 0,
        });
    }

    let size_usize = size as usize;
    let remaining = input.len().saturating_sub(header_start);
    if size_usize > remaining {
        return Err(Error::ObuPayloadOutOfRange {
            offset: ByteOffset::new(header_start as u64),
            size,
            remaining,
        });
    }

    let header = read_obu_header(input, ByteOffset::new(header_start as u64))?;
    let header_len = usize::from(header.header_size_bytes);
    if header_len > size_usize {
        return Err(Error::InvalidObuHeader {
            offset: ByteOffset::new(header_start as u64),
            message: "OBU header is larger than the declared OBU size".to_owned(),
        });
    }

    let payload_start = header_start.saturating_add(header_len);
    let payload_end = header_start.saturating_add(size_usize);
    let Some(payload) = input.get(payload_start..payload_end) else {
        return Err(Error::ObuPayloadOutOfRange {
            offset: ByteOffset::new(header_start as u64),
            size,
            remaining,
        });
    };

    let envelope = ObuEnvelope {
        offset: ByteOffset::new(header_start as u64),
        size,
        header,
        payload,
    };
    Ok((envelope, header_start.saturating_add(size_usize)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::{GLOBAL_XLAYER_ID, ObuType};

    #[test]
    fn parses_temporal_delimiter() {
        // size=1, header=0x08 (type=2 TemporalDelimiter, no extension, no payload).
        let stream = [0x01, 0x08];
        let obus = parse_annex_b_obus(&stream).unwrap();
        assert_eq!(obus.len(), 1);
        assert_eq!(obus[0].size, 1);
        assert_eq!(obus[0].offset, ByteOffset::new(1));
        assert_eq!(obus[0].header.obu_type, ObuType::TemporalDelimiter);
        assert_eq!(obus[0].header.extended_layer_id, GLOBAL_XLAYER_ID);
        assert!(obus[0].payload.is_empty());
    }

    #[test]
    fn parses_two_obus_with_payload() {
        // TD (size 1), then SequenceHeader (size 2: header 0x04 + 1 payload byte 0xAB).
        let stream = [0x01, 0x08, 0x02, 0x04, 0xAB];
        let obus = parse_annex_b_obus(&stream).unwrap();
        assert_eq!(obus.len(), 2);
        assert_eq!(obus[1].header.obu_type, ObuType::SequenceHeader);
        assert_eq!(obus[1].size, 2);
        assert_eq!(obus[1].payload, &[0xAB]);
    }

    #[test]
    fn zero_length_is_error() {
        assert!(matches!(
            parse_annex_b_obus(&[0x00]),
            Err(Error::ObuSizeOutOfRange { .. })
        ));
    }

    #[test]
    fn size_exceeding_input_is_error() {
        assert!(matches!(
            parse_annex_b_obus(&[0x05, 0x08]),
            Err(Error::ObuPayloadOutOfRange { .. })
        ));
    }

    #[test]
    fn truncated_length_is_error() {
        assert!(matches!(
            parse_annex_b_obus(&[0x80]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn empty_input_yields_no_obus() {
        assert!(parse_annex_b_obus(&[]).unwrap().is_empty());
    }

    #[test]
    fn partial_parse_keeps_prefix_and_reports_error() {
        // OBU #0 parses (TemporalDelimiter with extension); OBU #1 is truncated
        // (declares 5 bytes but only 1 is present).
        let stream = [0x02, 0x88, 0x05, 0x05, 0x08];
        let partial = parse_annex_b_obus_partial(&stream);
        assert_eq!(partial.obus.len(), 1);
        assert_eq!(partial.obus[0].header.obu_type, ObuType::TemporalDelimiter);
        assert!(matches!(
            partial.error,
            Some(Error::ObuPayloadOutOfRange { .. })
        ));
        // The strict wrapper still surfaces the error.
        assert!(parse_annex_b_obus(&stream).is_err());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::leb128::read_leb128;
    use crate::obu::read_obu_header;
    use proptest::prelude::*;

    proptest! {
        /// The parsers must never panic on arbitrary input.
        #[test]
        fn parsers_never_panic(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = read_leb128(&data, ByteOffset::new(0));
            let _ = read_obu_header(&data, ByteOffset::new(0));
            let _ = parse_annex_b_obus(&data);
        }
    }
}
