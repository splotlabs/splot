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
use crate::obu::{
    ObuHeader, ParsedObu, PayloadStatus, dispatch_obu_payload, read_obu_header_from_slice,
};
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

impl<'a> ObuEnvelope<'a> {
    /// Returns the absolute byte offset where this OBU's payload begins.
    #[must_use]
    pub fn payload_offset(&self) -> ByteOffset {
        self.offset
            .saturating_add(u64::from(self.header.header_size_bytes))
    }

    /// Dispatches this OBU's payload according to its parsed `obu_type`.
    ///
    /// Structural Annex B parsing remains header-first and payload-bounded; this
    /// helper lets callers opt into the currently implemented payload syntax.
    ///
    /// # Errors
    /// Returns typed parser errors for payload syntax that is implemented but
    /// malformed.
    pub fn payload_status(&self) -> Result<PayloadStatus<'a, ParsedObu<'a>>> {
        dispatch_obu_payload(self.header, self.payload, self.payload_offset())
    }
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

    // The OBU header and payload must lie entirely within this OBU's declared
    // bytes (Annex B: `open_bitstream_unit` receives exactly `num_bytes_in_obu`
    // bytes), so parse the header from that bounded slice and never the next OBU.
    let obu_end = header_start.saturating_add(size_usize);
    let Some(obu_bytes) = input.get(header_start..obu_end) else {
        return Err(Error::ObuPayloadOutOfRange {
            offset: ByteOffset::new(header_start as u64),
            size,
            remaining,
        });
    };

    let header = read_obu_header_from_slice(obu_bytes, ByteOffset::new(header_start as u64))?;
    let payload = obu_bytes
        .get(usize::from(header.header_size_bytes)..)
        .unwrap_or(&[]);

    let envelope = ObuEnvelope {
        offset: ByteOffset::new(header_start as u64),
        size,
        header,
        payload,
    };
    Ok((envelope, obu_end))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::obu::PayloadStatus;
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
    fn envelope_payload_status_is_opt_in_and_preserves_raw_payload() {
        // SequenceHeader payload parsing is intentionally not implemented yet,
        // but the envelope still preserves the bounded raw payload bytes.
        let stream = [0x02, 0x04, 0xAB];
        let obus = parse_annex_b_obus(&stream).unwrap();
        assert_eq!(obus[0].payload, &[0xAB]);
        assert_eq!(obus[0].payload_offset(), ByteOffset::new(2));
        assert_eq!(
            obus[0].payload_status().unwrap(),
            PayloadStatus::Unimplemented {
                feature: "AV2-5.4-SEQUENCE-HEADER",
                payload: obus[0].payload,
            }
        );
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

    #[test]
    fn header_parsing_is_bounded_to_declared_obu_size() {
        // size=1 but the header byte signals an extension; its extension byte would
        // fall in the NEXT OBU. The parser must error within this OBU, not peek ahead.
        assert!(matches!(
            parse_annex_b_obus(&[0x01, 0x88, 0x01, 0x08]),
            Err(Error::UnexpectedEof { .. })
        ));
        // Same class of error when the truncated extension is at end of stream.
        assert!(matches!(
            parse_annex_b_obus(&[0x01, 0x88]),
            Err(Error::UnexpectedEof { .. })
        ));
        // A valid 2-byte extension header still parses.
        let valid = parse_annex_b_obus(&[0x02, 0x88, 0x05]).unwrap();
        assert_eq!(valid.len(), 1);
        assert!(valid[0].header.has_header_extension);
        assert_eq!(valid[0].header.extended_layer_id.get(), 5);
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
