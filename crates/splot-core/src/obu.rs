// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 OBU header parsing (AV2 v1.0.0 § 5.2.2).
//!
//! This is the **AV2** OBU header, not the AV1 OBU header: there is no
//! `obu_forbidden_bit`, no `obu_has_size_field`, and no AV1-style extension byte.
//! The layout is:
//!
//! ```text
//! obu_header() {
//!     obu_header_extension_flag  f(1)   // bit 7
//!     obu_type                   f(5)   // bits 6..2
//!     obu_tlayer_id              f(2)   // bits 1..0
//!     if ( obu_header_extension_flag == 1 ) {
//!         obu_mlayer_id          f(3)   // byte 2, bits 7..5
//!         obu_xlayer_id          f(5)   // byte 2, bits 4..0
//!     }
//! }
//! ```

use serde::{Deserialize, Serialize};

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId};

/// A parsed AV2 OBU header (AV2 v1.0.0 § 5.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObuHeader {
    /// `obu_header_extension_flag`: whether the 1-byte extension is present.
    pub has_header_extension: bool,
    /// `obu_type` (AV2 Table 6.1).
    pub obu_type: ObuType,
    /// `obu_tlayer_id`.
    pub temporal_layer_id: TemporalLayerId,
    /// `obu_mlayer_id` (inferred `0` when the extension is absent).
    pub embedded_layer_id: EmbeddedLayerId,
    /// `obu_xlayer_id` (inferred per § 5.2.2 when the extension is absent).
    pub extended_layer_id: ExtendedLayerId,
    /// Number of header bytes consumed (`1` without extension, `2` with).
    pub header_size_bytes: u8,
}

/// Parses an AV2 OBU header from `obu_bytes`, whose first byte is at absolute
/// offset `start` (AV2 v1.0.0 § 5.2.2).
///
/// `obu_bytes` should contain only this OBU's bytes (per Annex B,
/// `open_bitstream_unit` receives exactly `num_bytes_in_obu` bytes), so a header
/// that signals an extension it has no room for returns [`Error::UnexpectedEof`]
/// instead of reading into the following OBU.
///
/// When the extension flag is `0`, `obu_xlayer_id` is inferred to
/// [`GLOBAL_XLAYER_ID`] for `OBU_MSDO` and `OBU_TEMPORAL_DELIMITER`, and `0`
/// otherwise; `obu_mlayer_id` is inferred to `0`.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the header (including the extension byte,
/// when signalled) does not fit in `obu_bytes`.
pub fn read_obu_header_from_slice(obu_bytes: &[u8], start: ByteOffset) -> Result<ObuHeader> {
    let mut reader = BitReader::new(obu_bytes, start);
    let has_header_extension = reader.read_bit()? == 1;
    let obu_type = ObuType::from_raw(reader.read_bits_u8(5)?);
    let temporal_layer_id = TemporalLayerId::from_bits(reader.read_bits_u8(2)?);

    let (embedded_layer_id, extended_layer_id, header_size_bytes) = if has_header_extension {
        let embedded = EmbeddedLayerId::from_bits(reader.read_bits_u8(3)?);
        let extended = ExtendedLayerId::from_bits(reader.read_bits_u8(5)?);
        (embedded, extended, 2)
    } else {
        let extended = if obu_type.requires_global_xlayer() {
            GLOBAL_XLAYER_ID
        } else {
            ExtendedLayerId::from_bits(0)
        };
        (EmbeddedLayerId::from_bits(0), extended, 1)
    };

    Ok(ObuHeader {
        has_header_extension,
        obu_type,
        temporal_layer_id,
        embedded_layer_id,
        extended_layer_id,
        header_size_bytes,
    })
}

/// Parses an AV2 OBU header from `input` starting at absolute offset `start`,
/// reading from `input[start..]` (AV2 v1.0.0 § 5.2.2).
///
/// For Annex B parsing, prefer [`read_obu_header_from_slice`] with the OBU's
/// declared bytes so the header parser cannot read past the OBU boundary.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the header is truncated, or
/// [`Error::InvalidObuHeader`] if `start` is out of range.
pub fn read_obu_header(input: &[u8], start: ByteOffset) -> Result<ObuHeader> {
    let start_idx = usize::try_from(start.get()).map_err(|_| Error::InvalidObuHeader {
        offset: start,
        message: "start offset overflows usize".to_owned(),
    })?;
    let buf = match input.get(start_idx..) {
        Some(slice) => slice,
        None => &[],
    };
    read_obu_header_from_slice(buf, start)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn header_without_extension() {
        // 0x04 = 0b0_00001_00 -> ext=0, type=1 (SequenceHeader), tlayer=0.
        let header = read_obu_header(&[0x04], ByteOffset::new(0)).unwrap();
        assert!(!header.has_header_extension);
        assert_eq!(header.obu_type, ObuType::SequenceHeader);
        assert_eq!(header.temporal_layer_id.get(), 0);
        assert_eq!(header.embedded_layer_id.get(), 0);
        assert_eq!(header.extended_layer_id.get(), 0);
        assert_eq!(header.header_size_bytes, 1);
    }

    #[test]
    fn header_with_extension() {
        // 0x99 = 0b1_00110_01 -> ext=1, type=6, tlayer=1.
        // 0x65 = 0b011_00101 -> mlayer=3, xlayer=5.
        let header = read_obu_header(&[0x99, 0x65], ByteOffset::new(0)).unwrap();
        assert!(header.has_header_extension);
        assert_eq!(header.obu_type, ObuType::LeadingTileGroup);
        assert_eq!(header.temporal_layer_id.get(), 1);
        assert_eq!(header.embedded_layer_id.get(), 3);
        assert_eq!(header.extended_layer_id.get(), 5);
        assert_eq!(header.header_size_bytes, 2);
    }

    #[test]
    fn temporal_delimiter_infers_global_xlayer() {
        // 0x08 = 0b0_00010_00 -> ext=0, type=2 (TemporalDelimiter).
        let header = read_obu_header(&[0x08], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::TemporalDelimiter);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);
    }

    #[test]
    fn msdo_infers_global_xlayer() {
        // 0x50 = 0b0_10100_00 -> ext=0, type=20 (Msdo).
        let header = read_obu_header(&[0x50], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::Msdo);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);
    }

    #[test]
    fn missing_extension_byte_is_eof() {
        // 0x99 signals an extension, but the second byte is missing.
        assert!(matches!(
            read_obu_header(&[0x99], ByteOffset::new(0)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn empty_input_is_eof() {
        assert!(matches!(
            read_obu_header(&[], ByteOffset::new(0)),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
