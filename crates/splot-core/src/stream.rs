// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Container-aware AV2 bitstream input parsing.
//!
//! Raw inputs are parsed as Annex B. Inputs beginning with `DKIF` are parsed as
//! IVF containers (`AV2-IVF-CONTAINER`) whose frame payloads contain Annex B OBUs.

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::annexb::{ObuEnvelope, PartialParse, parse_annex_b_obus_partial_at};
use crate::ivf::{IvfError, IvfFrame, IvfHeader, is_ivf, parse_ivf_partial};

/// Detected input container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BitstreamFormat {
    /// Raw AV2 Annex B length-delimited OBUs.
    AnnexB,
    /// IVF `DKIF` container whose frame payloads are AV2 Annex B.
    Ivf,
}

/// Parsed input in either supported container format.
#[derive(Debug)]
pub enum ParsedBitstream<'a> {
    /// Raw Annex B parse.
    AnnexB(PartialParse<'a>),
    /// IVF container parse.
    Ivf(ParsedIvfBitstream<'a>),
}

impl<'a> ParsedBitstream<'a> {
    /// Returns the detected input format.
    #[must_use]
    pub const fn format(&self) -> BitstreamFormat {
        match self {
            Self::AnnexB(_) => BitstreamFormat::AnnexB,
            Self::Ivf(_) => BitstreamFormat::Ivf,
        }
    }
}

/// Parsed IVF container with each frame payload parsed as Annex B.
#[derive(Debug)]
pub struct ParsedIvfBitstream<'a> {
    /// Parsed IVF header, if available.
    pub header: Option<IvfHeader>,
    /// Parsed IVF frames and their Annex B payload parse results.
    pub frames: Vec<ParsedIvfFrame<'a>>,
    /// Container-level parse error, if frame parsing stopped early.
    pub error: Option<IvfError>,
}

/// One IVF frame plus the result of parsing its payload as Annex B.
#[derive(Debug)]
pub struct ParsedIvfFrame<'a> {
    /// IVF frame record.
    pub frame: IvfFrame<'a>,
    /// OBUs parsed from this frame payload before any payload parse error.
    pub obus: Vec<ObuEnvelope<'a>>,
    /// Annex B parse error in this frame payload, if any.
    pub error: Option<Error>,
}

/// Parses `input` as IVF when it starts with `DKIF`, otherwise as raw Annex B.
#[must_use]
pub fn parse_bitstream_partial(input: &[u8]) -> ParsedBitstream<'_> {
    if is_ivf(input) {
        return ParsedBitstream::Ivf(parse_ivf_bitstream_partial(input));
    }
    ParsedBitstream::AnnexB(parse_annex_b_obus_partial_at(
        input,
        crate::span::ByteOffset::new(0),
    ))
}

fn parse_ivf_bitstream_partial(input: &[u8]) -> ParsedIvfBitstream<'_> {
    let parsed = parse_ivf_partial(input);
    let mut frames = Vec::with_capacity(parsed.frames.len());
    for frame in parsed.frames {
        let payload = parse_annex_b_obus_partial_at(frame.payload, frame.payload_offset);
        frames.push(ParsedIvfFrame {
            frame,
            obus: payload.obus,
            error: payload.error,
        });
    }

    ParsedIvfBitstream {
        header: parsed.header,
        frames,
        error: parsed.error,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
    use crate::span::ByteOffset;
    use crate::types::ObuType;

    fn ivf_with_frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_ivf_header(&mut bytes, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1)).unwrap();
        write_ivf_frame(&mut bytes, 0, payload).unwrap();
        bytes
    }

    #[test]
    fn detects_raw_annex_b() {
        let parsed = parse_bitstream_partial(&[0x01, 0x08]);
        assert!(matches!(parsed, ParsedBitstream::AnnexB(_)));
        let ParsedBitstream::AnnexB(raw) = parsed else {
            return;
        };
        assert_eq!(raw.obus.len(), 1);
        assert_eq!(raw.obus[0].offset, ByteOffset::new(1));
    }

    #[test]
    fn detects_ivf_and_preserves_obu_offsets() {
        let data = ivf_with_frame(&[0x01, 0x08]);
        let parsed = parse_bitstream_partial(&data);
        assert!(matches!(parsed, ParsedBitstream::Ivf(_)));
        let ParsedBitstream::Ivf(ivf) = parsed else {
            return;
        };
        assert!(ivf.error.is_none());
        assert_eq!(ivf.frames.len(), 1);
        assert_eq!(ivf.frames[0].frame.payload_offset, ByteOffset::new(44));
        assert_eq!(ivf.frames[0].obus.len(), 1);
        assert_eq!(ivf.frames[0].obus[0].offset, ByteOffset::new(45));
        assert_eq!(
            ivf.frames[0].obus[0].header.obu_type,
            ObuType::TemporalDelimiter
        );
    }

    #[test]
    fn records_annex_b_error_inside_ivf_frame() {
        let data = ivf_with_frame(&[0x05, 0x08]);
        let parsed = parse_bitstream_partial(&data);
        assert!(matches!(parsed, ParsedBitstream::Ivf(_)));
        let ParsedBitstream::Ivf(ivf) = parsed else {
            return;
        };
        assert!(ivf.error.is_none());
        assert_eq!(ivf.frames[0].obus.len(), 0);
        assert!(matches!(
            ivf.frames[0].error,
            Some(Error::ObuPayloadOutOfRange {
                offset,
                size: 5,
                remaining: 1
            }) if offset == ByteOffset::new(45)
        ));
    }

    #[test]
    fn parsers_never_panic_on_arbitrary_input() {
        for len in 0..128 {
            let bytes = vec![0x80; len];
            let _ = parse_bitstream_partial(&bytes);
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Container auto-detection and raw Annex B parsing must never panic on arbitrary input.
        #[test]
        fn bitstream_parser_never_panics(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = parse_bitstream_partial(&data);
        }

        /// The IVF branch of the container parser must never panic on arbitrary DKIF-prefixed input.
        #[test]
        fn ivf_detected_bitstream_parser_never_panics(
            tail in proptest::collection::vec(any::<u8>(), 0..2048)
        ) {
            let mut data = Vec::with_capacity(4 + tail.len());
            data.extend_from_slice(b"DKIF");
            data.extend_from_slice(&tail);
            let _ = parse_bitstream_partial(&data);
        }
    }
}
