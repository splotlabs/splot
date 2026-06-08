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
use crate::error::{Error, Result, TrailingBitsErrorKind};
use crate::headers::atlas_segment::{AtlasSegment, parse_atlas_segment};
use crate::headers::buffer_removal_timing::{BufferRemovalTiming, parse_buffer_removal_timing};
use crate::headers::content_interpretation::{ContentInterpretation, parse_content_interpretation};
use crate::headers::film_grain::{FilmGrainObu, parse_film_grain};
use crate::headers::layer_config_record::{LayerConfigurationRecord, parse_layer_config_record};
use crate::headers::operating_point_set::{OperatingPointSet, parse_operating_point_set};
use crate::headers::quantizer_matrix::{QuantizerMatrixObu, parse_quantizer_matrix};
use crate::headers::sequence::{SequenceHeader, parse_sequence_header};
use crate::hls::{
    MultiFrameHeader, MultistreamDecoderOperation, parse_msdo, parse_multi_frame_header,
};
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId};

/// Payload dispatch status for an OBU whose envelope and header have parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadStatus<'a, T> {
    /// Payload syntax was parsed into a typed representation.
    Parsed(T),
    /// Payload bytes are intentionally retained without syntax interpretation.
    Opaque(&'a [u8]),
    /// The OBU type is recognized, but its payload parser has not been implemented yet.
    Unimplemented {
        /// Feature ID that tracks the missing payload parser.
        feature: &'static str,
        /// Raw payload bytes within the declared OBU boundary.
        payload: &'a [u8],
    },
}

/// Parsed OBU payload syntax for OBU types currently modeled by `splot-core`.
///
/// `SequenceHeader` and `MultiFrameHeader` are boxed because they embed the large
/// `seg_info()` feature matrix and are much bigger than the other variants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParsedObu {
    /// `temporal_delimiter_obu()` (AV2 v1.0.0 § 5.5).
    TemporalDelimiter,
    /// `sequence_header_obu()` (AV2 v1.0.0 § 5.4).
    SequenceHeader(Box<SequenceHeader>),
    /// `multistream_decoder_operation_obu()` (AV2 v1.0.0 § 5.6).
    Msdo(MultistreamDecoderOperation),
    /// `multi_frame_header_obu()` (AV2 v1.0.0 § 5.7).
    MultiFrameHeader(Box<MultiFrameHeader>),
    /// `layer_config_record_obu()` (AV2 v1.0.0 § 5.8).
    LayerConfigurationRecord(Box<LayerConfigurationRecord>),
    /// `atlas_segment_info_obu()` (AV2 v1.0.0 § 5.9).
    AtlasSegment(Box<AtlasSegment>),
    /// `operating_point_set_obu()` (AV2 v1.0.0 § 5.10).
    OperatingPointSet(Box<OperatingPointSet>),
    /// `buffer_removal_timing_obu()` (AV2 v1.0.0 § 5.12).
    BufferRemovalTiming(BufferRemovalTiming),
    /// `quantizer_matrix_obu()` (AV2 v1.0.0 § 5.13).
    QuantizationMatrix(Box<QuantizerMatrixObu>),
    /// `film_grain_obu()` (AV2 v1.0.0 § 5.14).
    FilmGrain(Box<FilmGrainObu>),
    /// `content_interpretation_obu()` (AV2 v1.0.0 § 5.15).
    ContentInterpretation(ContentInterpretation),
}

impl ParsedObu {
    /// Returns the implementation-matrix feature ID for this parsed payload syntax.
    #[must_use]
    pub const fn feature_id(&self) -> &'static str {
        match self {
            Self::TemporalDelimiter => "AV2-5.5-TEMPORAL-DELIMITER",
            Self::SequenceHeader(_) => "AV2-5.4-SEQUENCE-HEADER",
            Self::Msdo(_) => "AV2-5.6-MSDO",
            Self::MultiFrameHeader(_) => "AV2-5.7-MULTI-FRAME-HEADER",
            Self::LayerConfigurationRecord(_) => "AV2-5.8-LAYER-CONFIG-RECORD",
            Self::AtlasSegment(_) => "AV2-5.9-ATLAS-SEGMENT",
            Self::OperatingPointSet(_) => "AV2-5.10-OPERATING-POINT-SET",
            Self::BufferRemovalTiming(_) => "AV2-5.12-BUFFER-REMOVAL-TIMING",
            Self::QuantizationMatrix(_) => "AV2-5.13-QUANTIZATION-MATRIX",
            Self::FilmGrain(_) => "AV2-5.14-FILM-GRAIN",
            Self::ContentInterpretation(_) => "AV2-5.15-CONTENT-INTERPRETATION",
        }
    }

    /// Returns a stable snake-case syntax label for tools and JSON output.
    #[must_use]
    pub const fn syntax_name(&self) -> &'static str {
        match self {
            Self::TemporalDelimiter => "temporal_delimiter_obu",
            Self::SequenceHeader(_) => "sequence_header_obu",
            Self::Msdo(_) => "multistream_decoder_operation_obu",
            Self::MultiFrameHeader(_) => "multi_frame_header_obu",
            Self::LayerConfigurationRecord(_) => "layer_config_record_obu",
            Self::AtlasSegment(_) => "atlas_segment_info_obu",
            Self::OperatingPointSet(_) => "operating_point_set_obu",
            Self::BufferRemovalTiming(_) => "buffer_removal_timing_obu",
            Self::QuantizationMatrix(_) => "quantizer_matrix_obu",
            Self::FilmGrain(_) => "film_grain_obu",
            Self::ContentInterpretation(_) => "content_interpretation_obu",
        }
    }
}

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

/// Dispatches an OBU payload according to `obu_type` (AV2 v1.0.0 § 5.2.1).
///
/// This is intentionally a partial dispatcher: OBU types whose payload syntax is
/// not yet implemented return [`PayloadStatus::Unimplemented`] with the matrix
/// feature ID that owns the missing parser. Payload syntax errors for implemented
/// cases are returned as typed [`Error`] values.
///
/// # Errors
/// Returns [`Error::InvalidTrailingBits`] or [`Error::UnexpectedEof`] if a
/// currently implemented temporal-delimiter payload has malformed trailing bits.
pub fn dispatch_obu_payload<'a>(
    header: ObuHeader,
    payload: &'a [u8],
    payload_offset: ByteOffset,
) -> Result<PayloadStatus<'a, ParsedObu>> {
    match header.obu_type {
        ObuType::Reserved0 | ObuType::Reserved(_) => Ok(PayloadStatus::Opaque(payload)),
        ObuType::TemporalDelimiter => {
            parse_empty_payload_syntax(payload, payload_offset)?;
            Ok(PayloadStatus::Parsed(ParsedObu::TemporalDelimiter))
        }
        ObuType::SequenceHeader => {
            let mut reader = BitReader::new(payload, payload_offset);
            let sequence_header = parse_sequence_header(&mut reader)?;
            if let Some(feature) = sequence_header.unimplemented_at {
                return Ok(PayloadStatus::Unimplemented { feature, payload });
            }
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(Box::new(
                sequence_header,
            ))))
        }
        ObuType::Msdo => {
            let mut reader = BitReader::new(payload, payload_offset);
            let msdo = parse_msdo(&mut reader)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::Msdo(msdo)))
        }
        ObuType::MultiFrameHeader => {
            let mut reader = BitReader::new(payload, payload_offset);
            let multi_frame_header = parse_multi_frame_header(&mut reader)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::MultiFrameHeader(
                Box::new(multi_frame_header),
            )))
        }
        ObuType::LayerConfigurationRecord => {
            let mut reader = BitReader::new(payload, payload_offset);
            let layer_config_record =
                parse_layer_config_record(&mut reader, header.extended_layer_id)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(
                Box::new(layer_config_record),
            )))
        }
        ObuType::AtlasSegment => {
            let mut reader = BitReader::new(payload, payload_offset);
            let atlas_segment = parse_atlas_segment(&mut reader)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::AtlasSegment(Box::new(
                atlas_segment,
            ))))
        }
        ObuType::OperatingPointSet => {
            let mut reader = BitReader::new(payload, payload_offset);
            let operating_point_set =
                parse_operating_point_set(&mut reader, header.extended_layer_id)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::OperatingPointSet(
                Box::new(operating_point_set),
            )))
        }
        ObuType::BufferRemovalTiming => {
            let mut reader = BitReader::new(payload, payload_offset);
            let buffer_removal_timing = parse_buffer_removal_timing(&mut reader)?;
            // OBU_BUFFER_REMOVAL_TIMING is not extensible (§ 5.2.1, ObuType::
            // is_extensible_obu), so finish_obu_payload uses trailing_bits() only.
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::BufferRemovalTiming(
                buffer_removal_timing,
            )))
        }
        ObuType::QuantizationMatrix => {
            let mut reader = BitReader::new(payload, payload_offset);
            let quantizer_matrix = parse_quantizer_matrix(
                &mut reader,
                header.temporal_layer_id,
                header.embedded_layer_id,
            )?;
            // OBU_QUANTIZATION_MATRIX is not extensible (§ 5.2.1, ObuType::
            // is_extensible_obu), so finish_obu_payload uses trailing_bits() only.
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::QuantizationMatrix(
                Box::new(quantizer_matrix),
            )))
        }
        ObuType::FilmGrain => {
            let mut reader = BitReader::new(payload, payload_offset);
            let film_grain = parse_film_grain(
                &mut reader,
                header.temporal_layer_id,
                header.embedded_layer_id,
            )?;
            // OBU_FILM_GRAIN is not extensible (§ 5.2.1), so finish_obu_payload uses
            // trailing_bits() only.
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::FilmGrain(Box::new(
                film_grain,
            ))))
        }
        ObuType::ContentInterpretation => {
            let mut reader = BitReader::new(payload, payload_offset);
            let content_interpretation = parse_content_interpretation(&mut reader)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::ContentInterpretation(
                content_interpretation,
            )))
        }
        obu_type => Ok(PayloadStatus::Unimplemented {
            feature: unimplemented_payload_feature(obu_type),
            payload,
        }),
    }
}

/// Validates the bits between the end of an OBU's parsed syntax and the OBU
/// boundary (AV2 v1.0.0 § 5.2.1 `open_bitstream_unit`).
///
/// `reader` must be positioned immediately after the OBU's parsed syntax, over the
/// same `payload` slice. For `is_extensible_obu()` types, an `obu_extension_flag`
/// follows the syntax; AV2 § 6.2.1 requires it to be `0` in this specification
/// version, so a set flag is rejected as [`Error::InvalidObuExtension`]. Otherwise
/// the remainder is `trailing_bits()`. Non-extensible types use `trailing_bits()`
/// directly. Tile groups (`usedArith`) are not handled here.
///
/// This is the shared "finish" logic used by both `dispatch_obu_payload` and the
/// `splot-validate` payload checks, so the validator and inspector agree on
/// payload-tail conformance.
///
/// # Errors
/// Returns [`Error::InvalidObuExtension`] for a set `obu_extension_flag`,
/// [`Error::InvalidTrailingBits`] for malformed trailing bits, or
/// [`Error::UnexpectedEof`] if the tail is truncated.
pub fn finish_obu_payload(
    reader: &mut BitReader<'_>,
    payload: &[u8],
    is_extensible: bool,
) -> Result<()> {
    if payload.is_empty() {
        return Ok(());
    }

    if is_extensible {
        let flag_offset = reader.byte_offset();
        let flag_bit_offset = reader.bit_offset();
        let obu_extension_flag = reader.read_bit()? != 0;
        if obu_extension_flag {
            // AV2 § 6.2.1: it is a requirement of bitstream conformance that
            // obu_extension_flag is equal to 0 in bitstreams conforming to this
            // specification version, so a set flag is a conformance violation
            // rather than opaque extension data.
            return Err(Error::InvalidObuExtension {
                offset: flag_offset,
                bit_offset: flag_bit_offset,
            });
        }
    }

    let remaining = reader.remaining_bits();
    parse_trailing_bits(reader, remaining)
}

fn parse_empty_payload_syntax(payload: &[u8], payload_offset: ByteOffset) -> Result<()> {
    if payload.is_empty() {
        return Ok(());
    }

    let mut reader = BitReader::new(payload, payload_offset);
    let nb_bits = (payload.len() as u64).saturating_mul(8);
    parse_trailing_bits(&mut reader, nb_bits)
}

fn unimplemented_payload_feature(obu_type: ObuType) -> &'static str {
    match obu_type {
        ObuType::SequenceHeader => "AV2-5.4-SEQUENCE-HEADER",
        ObuType::MultiFrameHeader => "AV2-5.7-MULTI-FRAME-HEADER",
        ObuType::ClosedLoopKey
        | ObuType::OpenLoopKey
        | ObuType::LeadingTileGroup
        | ObuType::RegularTileGroup
        | ObuType::Switch
        | ObuType::RasFrame => "AV2-5.19-TILE-GROUP",
        ObuType::MetadataShort | ObuType::MetadataGroup => "AV2-5.17-METADATA",
        ObuType::LeadingSef
        | ObuType::RegularSef
        | ObuType::LeadingTip
        | ObuType::RegularTip
        | ObuType::BridgeFrame => "AV2-5.18-FRAME-HEADER",
        ObuType::BufferRemovalTiming => "AV2-5.12-BUFFER-REMOVAL-TIMING",
        ObuType::LayerConfigurationRecord => "AV2-5.8-LAYER-CONFIG-RECORD",
        ObuType::AtlasSegment => "AV2-5.9-ATLAS-SEGMENT",
        ObuType::OperatingPointSet => "AV2-5.10-OPERATING-POINT-SET",
        ObuType::Msdo => "AV2-5.6-MSDO",
        ObuType::QuantizationMatrix => "AV2-5.13-QUANTIZATION-MATRIX",
        ObuType::FilmGrain => "AV2-5.14-FILM-GRAIN",
        ObuType::ContentInterpretation => "AV2-5.15-CONTENT-INTERPRETATION",
        ObuType::Padding => "AV2-5.16-PADDING",
        ObuType::Reserved0 | ObuType::TemporalDelimiter | ObuType::Reserved(_) => {
            // Unreachable via dispatch_obu_payload(); present only for match exhaustiveness.
            "AV2-5.2.1-OBU-DISPATCH"
        }
    }
}

/// Parses AV2 `trailing_bits(nbBits)` from `reader` (AV2 v1.0.0 § 5.2.3).
///
/// The parser consumes exactly `nb_bits` bits: the first bit must be
/// `trailing_one_bit == 1`, and every remaining bit must be zero per AV2 § 6.2.3.
///
/// # Errors
/// Returns [`Error::InvalidTrailingBits`] if `nb_bits == 0`, if the first bit is
/// not `1`, or if any zero-padding bit is not `0`. Returns
/// [`Error::UnexpectedEof`] if fewer than `nb_bits` bits remain.
pub fn parse_trailing_bits(reader: &mut BitReader<'_>, nb_bits: u64) -> Result<()> {
    if nb_bits == 0 {
        return Err(Error::InvalidTrailingBits {
            offset: reader.byte_offset(),
            bit_offset: reader.bit_offset(),
            kind: TrailingBitsErrorKind::Empty,
        });
    }

    let offset = reader.byte_offset();
    let bit_offset = reader.bit_offset();
    if reader.read_bit()? != 1 {
        return Err(Error::InvalidTrailingBits {
            offset,
            bit_offset,
            kind: TrailingBitsErrorKind::MissingOneBit,
        });
    }

    for _ in 1..nb_bits {
        let offset = reader.byte_offset();
        let bit_offset = reader.bit_offset();
        if reader.read_bit()? != 0 {
            return Err(Error::InvalidTrailingBits {
                offset,
                bit_offset,
                kind: TrailingBitsErrorKind::ZeroBitNotZero,
            });
        }
    }

    Ok(())
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

    #[test]
    fn trailing_bits_accepts_one_bit_followed_by_zeroes() {
        let mut reader = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
        parse_trailing_bits(&mut reader, 8).unwrap();
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn trailing_bits_rejects_empty_payload_bits() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_trailing_bits(&mut reader, 0),
            Err(Error::InvalidTrailingBits {
                kind: TrailingBitsErrorKind::Empty,
                ..
            })
        ));
    }

    #[test]
    fn trailing_bits_requires_first_bit_to_be_one() {
        let mut reader = BitReader::new(&[0b0000_0000], ByteOffset::new(0));
        assert!(matches!(
            parse_trailing_bits(&mut reader, 8),
            Err(Error::InvalidTrailingBits {
                kind: TrailingBitsErrorKind::MissingOneBit,
                ..
            })
        ));
    }

    #[test]
    fn trailing_bits_requires_zero_padding() {
        let mut reader = BitReader::new(&[0b1100_0000], ByteOffset::new(0));
        assert!(matches!(
            parse_trailing_bits(&mut reader, 8),
            Err(Error::InvalidTrailingBits {
                kind: TrailingBitsErrorKind::ZeroBitNotZero,
                ..
            })
        ));
    }

    #[test]
    fn trailing_bits_reports_eof() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_trailing_bits(&mut reader, 1),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn dispatch_parses_temporal_delimiter_payload() {
        let header = read_obu_header(&[0x08], ByteOffset::new(0)).unwrap();
        let status = dispatch_obu_payload(header, &[0x80], ByteOffset::new(1)).unwrap();
        assert!(matches!(
            status,
            PayloadStatus::Parsed(ParsedObu::TemporalDelimiter)
        ));
    }

    #[test]
    fn dispatch_keeps_reserved_payload_opaque() {
        let header = read_obu_header(&[0x00], ByteOffset::new(0)).unwrap();
        let payload = [0x40];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert_eq!(status, PayloadStatus::Opaque(&payload));
    }

    #[test]
    fn dispatch_rejects_bad_empty_syntax_payload_trailing_bits() {
        let header = read_obu_header(&[0x08], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &[0x00], ByteOffset::new(1)),
            Err(Error::InvalidTrailingBits {
                kind: TrailingBitsErrorKind::MissingOneBit,
                ..
            })
        ));
    }

    #[test]
    fn dispatch_keeps_all_zero_reserved_payload_opaque() {
        let header = read_obu_header(&[0x00], ByteOffset::new(0)).unwrap();
        let payload = [0x00];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert_eq!(status, PayloadStatus::Opaque(&payload));
    }

    #[test]
    fn dispatch_attempts_sequence_header_payload() {
        // A 1-byte payload is too short for a full sequence header, so dispatch now
        // surfaces a typed parse error rather than reporting the payload unimplemented.
        let header = read_obu_header(&[0x04], ByteOffset::new(0)).unwrap();
        let payload = [0xAB];
        assert!(matches!(
            dispatch_obu_payload(header, &payload, ByteOffset::new(1)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn dispatch_marks_metadata_payload_unimplemented() {
        // OBU_METADATA_SHORT payload parsing is not implemented; the dispatcher
        // still preserves the raw payload and names the owning Feature ID.
        // 0x20 = 0b0_01000_00 -> ext=0, type=8 (MetadataShort).
        let header = read_obu_header(&[0x20], ByteOffset::new(0)).unwrap();
        let payload = [0xAB];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert_eq!(
            status,
            PayloadStatus::Unimplemented {
                feature: "AV2-5.17-METADATA",
                payload: &payload,
            }
        );
    }

    #[test]
    fn dispatch_parses_layer_config_record_payload() {
        // 0xC0 0x1F -> ext=1, type=16 (LayerConfigurationRecord), xlayer=31 (global).
        let header = read_obu_header(&[0xC0, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::LayerConfigurationRecord);
        // Minimal global LCR (lcr_global_config_record_id=1, lcr_xlayer_map=1, all flags
        // 0) followed by obu_extension_flag(0) + trailing_one_bit.
        let payload = [0x20, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(2)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "layer_config_record_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.8-LAYER-CONFIG-RECORD");
        }
    }

    #[test]
    fn dispatch_parses_atlas_segment_payload() {
        // 0xC4 0x03 -> ext=1, type=17 (AtlasSegment), xlayer=3.
        let header = read_obu_header(&[0xC4, 0x03], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::AtlasSegment);
        // SINGLE_ATLAS (mode 2), nominal dims, no signaled ids, then the OBU tail.
        let payload = [0x0F, 0x20];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(2)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::AtlasSegment(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "atlas_segment_info_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.9-ATLAS-SEGMENT");
        }
    }

    #[test]
    fn dispatch_parses_operating_point_set_payload() {
        // 0xC8 0x1F -> ext=1, type=18 (OperatingPointSet), xlayer=31 (global).
        let header = read_obu_header(&[0xC8, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::OperatingPointSet);
        // Reset-only OPS (ops_reset_flag=0, ops_id=0, ops_cnt=0) then the extensible
        // tail: obu_extension_flag(0) + trailing_one_bit.
        let payload = [0x00, 0x40];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(2)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::OperatingPointSet(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "operating_point_set_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.10-OPERATING-POINT-SET");
        }
    }

    #[test]
    fn dispatch_parses_buffer_removal_timing_payload() {
        // 0x3C -> ext=0, type=15 (BufferRemovalTiming), xlayer inferred 0.
        let header = read_obu_header(&[0x3C], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::BufferRemovalTiming);
        // br_ops_dependent_flag(0) + br_time = rg(4) of 0, then trailing_one_bit
        // (BRT is non-extensible, so there is no obu_extension_flag).
        let payload = [0x02];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::BufferRemovalTiming(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "buffer_removal_timing_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.12-BUFFER-REMOVAL-TIMING");
        }
    }

    #[test]
    fn dispatch_parses_quantizer_matrix_payload() {
        // 0x58 -> ext=0, type=22 (QuantizationMatrix), tlayer=0.
        let header = read_obu_header(&[0x58], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::QuantizationMatrix);
        // qm_bit_map=0 (f(15)) + qm_chroma_info_present_flag=0 (f(1)) = 16 zero bits,
        // then trailing_one_bit (QM is non-extensible).
        let payload = [0x00, 0x00, 0x80];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::QuantizationMatrix(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "quantizer_matrix_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.13-QUANTIZATION-MATRIX");
        }
    }

    #[test]
    fn dispatch_parses_film_grain_payload() {
        // 0x5C -> ext=0, type=23 (FilmGrain), tlayer=0.
        let header = read_obu_header(&[0x5C], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::FilmGrain);
        // fgm_update_flags=0 (f(8)) + fgm_chroma_idc=uvlc(0) (a `1` bit) then
        // trailing_one_bit: byte 0x00, then 0xC0 = `1`(uvlc) `1`(trailing) padding.
        let payload = [0x00, 0xC0];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::FilmGrain(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "film_grain_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.14-FILM-GRAIN");
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `trailing_bits(nbBits)` must never panic on arbitrary payload-shaped input.
        #[test]
        fn trailing_bits_never_panic(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            nb_bits in 0u64..=512,
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_trailing_bits(&mut reader, nb_bits);
        }
    }
}
