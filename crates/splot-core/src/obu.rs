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
use crate::headers::frame::{FrameHeaderPrefix, parse_frame_header_prefix};
use crate::headers::layer_config_record::{LayerConfigurationRecord, parse_layer_config_record};
use crate::headers::metadata::{
    MetadataGroupObu, MetadataShortObu, parse_metadata_group, parse_metadata_short,
};
use crate::headers::operating_point_set::{OperatingPointSet, parse_operating_point_set};
use crate::headers::padding::{PaddingObu, parse_padding_obu};
use crate::headers::quantizer_matrix::{QuantizerMatrixObu, parse_quantizer_matrix};
use crate::headers::sequence::{SequenceHeader, parse_sequence_header};
use crate::headers::tile_group::{TileGroupHeaderPrefix, parse_tile_group_prefix};
use crate::hls::{
    MultiFrameHeader, MultistreamDecoderOperation, parse_msdo, parse_multi_frame_header,
};
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId};

/// The reason the state-dependent remainder of a frame-carrying OBU payload was not
/// parsed by the stateless dispatcher: the rest of § 5.18 / § 5.19 needs the activated
/// sequence header (and, on the `cur_mfh_id > 0` path, the referenced multi-frame
/// header) plus per-extended-layer decoder state, none of which the stateless front
/// door holds.
const BLOCKED_ON_ACTIVE_SEQUENCE_HEADER_STATE: &str = "active sequence header state";

/// The state-free prefix of a frame-carrying OBU payload parsed by the stateless
/// dispatcher (AV2 v1.0.0 § 5.18.2 / § 5.19).
///
/// The dispatcher parses exactly the portion of `tile_group_obu()` / `frame_header()`
/// that needs no cross-OBU state — the activation/reference fields — and returns this
/// inside [`PayloadStatus::PrefixParsed`]. The richer, state-aware surface lives in the
/// inspector's stateful frame-header views and the validator's direct-call path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FramePayloadPrefix {
    /// The `tile_group_obu()` prefix (AV2 v1.0.0 § 5.19): `is_first_tile_group`,
    /// `frame_header_present_flag`, and the first tile group's frame-header prefix.
    TileGroup(TileGroupHeaderPrefix),
    /// The `frame_header()` activation prefix (AV2 v1.0.0 § 5.18.2) carried directly by
    /// an `OBU_*_SEF` / `OBU_*_TIP` / `OBU_BRIDGE_FRAME` payload.
    FrameHeader(FrameHeaderPrefix),
}

impl FramePayloadPrefix {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::TileGroup(_) => "tile_group_prefix",
            Self::FrameHeader(_) => "frame_header_prefix",
        }
    }
}

/// Payload dispatch status for an OBU whose envelope and header have parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadStatus<'a, T> {
    /// Payload syntax was parsed into a typed representation.
    Parsed(T),
    /// Payload bytes are intentionally retained without syntax interpretation.
    Opaque(&'a [u8]),
    /// A frame-carrying OBU whose state-free prefix parsed, but whose remainder needs
    /// cross-OBU state the stateless dispatcher does not hold.
    ///
    /// This is the honest result for the 11 frame-carrying OBU types (the tile-group
    /// family and the SEF / TIP / bridge family): their § 5.18.2 / § 5.19 activation
    /// prefix is parsed into `prefix`, and `blocked_on` names what the rest needs (the
    /// activated sequence header state). The richer state-aware surface is the
    /// inspector's stateful frame-header views and the validator's direct-call path.
    PrefixParsed {
        /// The parsed state-free prefix.
        prefix: FramePayloadPrefix,
        /// Why the state-dependent remainder was not parsed here.
        blocked_on: &'static str,
        /// Feature ID that owns the state-dependent remainder.
        feature: &'static str,
    },
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
    /// `padding_obu()` (AV2 v1.0.0 § 5.16).
    Padding(PaddingObu),
    /// `metadata_short_obu()` (AV2 v1.0.0 § 5.17.2).
    MetadataShort(Box<MetadataShortObu>),
    /// `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3).
    MetadataGroup(Box<MetadataGroupObu>),
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
            Self::Padding(_) => "AV2-5.16-PADDING",
            Self::MetadataShort(_) => "AV2-5.17.2-METADATA-SHORT",
            Self::MetadataGroup(_) => "AV2-5.17.3-METADATA-GROUP",
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
            Self::Padding(_) => "padding_obu",
            Self::MetadataShort(_) => "metadata_short_obu",
            Self::MetadataGroup(_) => "metadata_group_obu",
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
/// This dispatcher is **stateless**: it never threads cross-OBU state (the activated
/// sequence header, the multi-frame-header store, per-extended-layer decoder state).
/// Fully-decidable payloads are parsed into a typed [`PayloadStatus::Parsed`]; the 11
/// frame-carrying OBU types (the tile-group family and the SEF / TIP / bridge family)
/// have their state-free § 5.18.2 / § 5.19 activation prefix parsed and returned as
/// [`PayloadStatus::PrefixParsed`], whose `blocked_on` names the state the remainder
/// needs. The richer, state-aware surface for those types is the inspector's stateful
/// frame-header views and the validator's direct-call path. Reserved payloads stay
/// [`PayloadStatus::Opaque`]; payload syntax errors are returned as typed [`Error`]
/// values.
///
/// # Errors
/// Returns [`Error::InvalidTrailingBits`] or [`Error::UnexpectedEof`] if a currently
/// implemented payload has malformed trailing bits, or [`Error::UnexpectedEof`] /
/// [`Error::InvalidUvlc`] if a frame-carrying OBU's payload ends or is malformed inside
/// the state-free activation prefix.
pub fn dispatch_obu_payload(
    header: ObuHeader,
    payload: &[u8],
    payload_offset: ByteOffset,
) -> Result<PayloadStatus<'_, ParsedObu>> {
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
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::BufferRemovalTiming(
                buffer_removal_timing,
            )))
        }
        ObuType::QuantizationMatrix => {
            let mut reader = BitReader::new(payload, payload_offset);
            let quantizer_matrix = parse_quantizer_matrix(&mut reader)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::QuantizationMatrix(
                Box::new(quantizer_matrix),
            )))
        }
        ObuType::FilmGrain => {
            let mut reader = BitReader::new(payload, payload_offset);
            let film_grain = parse_film_grain(&mut reader)?;
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
        ObuType::Padding => {
            let padding = parse_padding_obu(payload, payload_offset)?;
            Ok(PayloadStatus::Parsed(ParsedObu::Padding(padding)))
        }
        ObuType::MetadataShort => {
            let mut reader = BitReader::new(payload, payload_offset);
            let metadata = parse_metadata_short(&mut reader, payload.len())?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::MetadataShort(Box::new(
                metadata,
            ))))
        }
        ObuType::MetadataGroup => {
            let mut reader = BitReader::new(payload, payload_offset);
            let metadata = parse_metadata_group(&mut reader, header.extended_layer_id)?;
            finish_obu_payload(&mut reader, payload, header.obu_type.is_extensible_obu())?;
            Ok(PayloadStatus::Parsed(ParsedObu::MetadataGroup(Box::new(
                metadata,
            ))))
        }
        obu_type if obu_type.is_tile_group() => {
            let mut reader = BitReader::new(payload, payload_offset);
            let prefix = parse_tile_group_prefix(&mut reader, obu_type, None)?;
            Ok(PayloadStatus::PrefixParsed {
                prefix: FramePayloadPrefix::TileGroup(prefix),
                blocked_on: BLOCKED_ON_ACTIVE_SEQUENCE_HEADER_STATE,
                feature: "AV2-5.19-TILE-GROUP",
            })
        }
        obu_type
            if obu_type.is_sef() || obu_type.is_tip_frame() || obu_type == ObuType::BridgeFrame =>
        {
            let mut reader = BitReader::new(payload, payload_offset);
            let prefix = parse_frame_header_prefix(&mut reader, obu_type, None)?;
            Ok(PayloadStatus::PrefixParsed {
                prefix: FramePayloadPrefix::FrameHeader(prefix),
                blocked_on: BLOCKED_ON_ACTIVE_SEQUENCE_HEADER_STATE,
                feature: "AV2-5.18-FRAME-HEADER",
            })
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
        let obu_extension_flag = reader.read_flag()?;
        if obu_extension_flag {
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

/// Returns the implementation-matrix feature ID that owns the payload parser for an
/// `obu_type` that `dispatch_obu_payload` does not yet parse (its catch-all arm).
///
/// As of the frame-carrying prefix dispatch, **no** type reaches this function: every
/// frame-carrying type is handled by an explicit `PrefixParsed` arm, every other type is
/// parsed by an explicit dispatch arm or kept opaque (reserved types). All variants are
/// matched only to keep the match exhaustive and to keep an honest fallback feature ID if
/// the dispatch arms ever change; the tile-group / frame-header arms below name the
/// state-dependent residual owners directly.
fn unimplemented_payload_feature(obu_type: ObuType) -> &'static str {
    match obu_type {
        ObuType::ClosedLoopKey
        | ObuType::OpenLoopKey
        | ObuType::LeadingTileGroup
        | ObuType::RegularTileGroup
        | ObuType::Switch
        | ObuType::RasFrame => "AV2-5.19-TILE-GROUP",
        ObuType::LeadingSef
        | ObuType::RegularSef
        | ObuType::LeadingTip
        | ObuType::RegularTip
        | ObuType::BridgeFrame => "AV2-5.18-FRAME-HEADER",
        ObuType::Reserved0
        | ObuType::TemporalDelimiter
        | ObuType::Reserved(_)
        | ObuType::SequenceHeader
        | ObuType::MultiFrameHeader
        | ObuType::Msdo
        | ObuType::LayerConfigurationRecord
        | ObuType::AtlasSegment
        | ObuType::OperatingPointSet
        | ObuType::BufferRemovalTiming
        | ObuType::QuantizationMatrix
        | ObuType::FilmGrain
        | ObuType::ContentInterpretation
        | ObuType::Padding
        | ObuType::MetadataShort
        | ObuType::MetadataGroup => "AV2-5.2.1-OBU-DISPATCH",
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
        if reader.read_flag()? {
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
        let header = read_obu_header(&[0x08], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::TemporalDelimiter);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);
    }

    #[test]
    fn msdo_infers_global_xlayer() {
        let header = read_obu_header(&[0x50], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::Msdo);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);
    }

    #[test]
    fn missing_extension_byte_is_eof() {
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
        let header = read_obu_header(&[0x04], ByteOffset::new(0)).unwrap();
        let payload = [0xAB];
        assert!(matches!(
            dispatch_obu_payload(header, &payload, ByteOffset::new(1)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn dispatch_parses_metadata_short_payload() {
        let header = read_obu_header(&[0x20], ByteOffset::new(0)).unwrap();
        let payload = [0x08, 0x04, 0x80];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::MetadataShort(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "metadata_short_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.17.2-METADATA-SHORT");
        }
    }

    #[test]
    fn dispatch_parses_metadata_group_payload() {
        let header = read_obu_header(&[0x24], ByteOffset::new(0)).unwrap();
        let payload = [0x00, 0x00, 0x04, 0x01, 0x80];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::MetadataGroup(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "metadata_group_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.17.3-METADATA-GROUP");
        }
    }

    #[test]
    fn dispatch_parses_padding_payload() {
        let header = read_obu_header(&[0x64], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::Padding);
        let payload = [0xDE, 0xAD, 0x80];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::Parsed(ParsedObu::Padding(_))
        ));
        if let PayloadStatus::Parsed(parsed) = &status {
            assert_eq!(parsed.syntax_name(), "padding_obu");
            assert_eq!(parsed.feature_id(), "AV2-5.16-PADDING");
        }
    }

    #[test]
    fn dispatch_parses_layer_config_record_payload() {
        let header = read_obu_header(&[0xC0, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::LayerConfigurationRecord);
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
        let header = read_obu_header(&[0xC4, 0x03], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::AtlasSegment);
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
        let header = read_obu_header(&[0xC8, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::OperatingPointSet);
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
        let header = read_obu_header(&[0x3C], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::BufferRemovalTiming);
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
        let header = read_obu_header(&[0x58], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::QuantizationMatrix);
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
        let header = read_obu_header(&[0x5C], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::FilmGrain);
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

    #[test]
    fn dispatch_parses_tile_group_prefix_and_blocks_on_state() {
        let header = read_obu_header(&[0x10], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::ClosedLoopKey);
        let payload = [0xD8];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::PrefixParsed {
                prefix: FramePayloadPrefix::TileGroup(_),
                blocked_on: "active sequence header state",
                feature: "AV2-5.19-TILE-GROUP",
            }
        ));
        if let PayloadStatus::PrefixParsed { prefix, .. } = &status {
            assert_eq!(prefix.label(), "tile_group_prefix");
            if let FramePayloadPrefix::TileGroup(tg) = prefix {
                assert!(tg.is_first_tile_group);
                assert!(tg.frame_header_present_flag);
                let fh = tg
                    .frame_header
                    .expect("first tile group carries a frame header");
                assert!(fh.cur_mfh_id.is_zero());
                assert_eq!(fh.seq_header_id_in_frame_header, Some(2));
            }
        }
    }

    #[test]
    fn dispatch_parses_bridge_frame_prefix_and_blocks_on_state() {
        // 0x4C = 0b0_10011_00 -> ext=0, type=19 (BridgeFrame, a frame-header type).
        let header = read_obu_header(&[0x4C], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::BridgeFrame);
        // IsBridge infers cur_mfh_id=0 (no bits), then seq_header_id=uvlc(0) (a `1` bit) -> 0x80.
        let payload = [0x80];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::PrefixParsed {
                prefix: FramePayloadPrefix::FrameHeader(_),
                blocked_on: "active sequence header state",
                feature: "AV2-5.18-FRAME-HEADER",
            }
        ));
        if let PayloadStatus::PrefixParsed { prefix, .. } = &status {
            assert_eq!(prefix.label(), "frame_header_prefix");
            if let FramePayloadPrefix::FrameHeader(fh) = prefix {
                assert!(fh.is_bridge);
                assert!(fh.cur_mfh_id.is_zero());
                assert_eq!(fh.seq_header_id_in_frame_header, Some(0));
            }
        }
    }

    #[test]
    fn dispatch_parses_sef_prefix_and_blocks_on_state() {
        let header = read_obu_header(&[0x30], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::RegularSef);
        let payload = [0xC0];
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            &status,
            PayloadStatus::PrefixParsed {
                prefix: FramePayloadPrefix::FrameHeader(_),
                feature: "AV2-5.18-FRAME-HEADER",
                ..
            }
        ));
        if let PayloadStatus::PrefixParsed {
            prefix: FramePayloadPrefix::FrameHeader(fh),
            ..
        } = &status
        {
            assert!(!fh.is_bridge);
            assert!(fh.is_regular);
            assert!(fh.cur_mfh_id.is_zero());
        }
    }

    #[test]
    fn dispatch_tile_group_eof_in_prefix_is_structured_error() {
        let header = read_obu_header(&[0x10], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &[], ByteOffset::new(1)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn dispatch_frame_header_eof_in_prefix_is_structured_error() {
        let header = read_obu_header(&[0x30], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &[], ByteOffset::new(1)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn dispatch_clk_tile_group_prefix_leaves_starts_cvs_unknown() {
        let header = read_obu_header(&[0x10], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::ClosedLoopKey);
        let status = dispatch_obu_payload(header, &[0xD8], ByteOffset::new(1)).unwrap();
        assert!(
            matches!(
                &status,
                PayloadStatus::PrefixParsed {
                    prefix: FramePayloadPrefix::TileGroup(_),
                    ..
                }
            ),
            "expected a tile-group prefix, got {status:?}"
        );
        if let PayloadStatus::PrefixParsed {
            prefix: FramePayloadPrefix::TileGroup(tg),
            ..
        } = &status
        {
            let fh = tg
                .frame_header
                .expect("first tile group carries a frame header");
            assert_eq!(fh.starts_cvs, None, "stateless CLK startCVS is unknown");
        }
    }

    #[test]
    fn dispatch_non_clk_frame_header_prefix_has_some_false_starts_cvs() {
        let header = read_obu_header(&[0x30], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::RegularSef);
        let status = dispatch_obu_payload(header, &[0xC0], ByteOffset::new(1)).unwrap();
        assert!(
            matches!(
                &status,
                PayloadStatus::PrefixParsed {
                    prefix: FramePayloadPrefix::FrameHeader(_),
                    ..
                }
            ),
            "expected a frame-header prefix, got {status:?}"
        );
        if let PayloadStatus::PrefixParsed {
            prefix: FramePayloadPrefix::FrameHeader(fh),
            ..
        } = &status
        {
            assert_eq!(fh.starts_cvs, Some(false));
        }
    }

    #[test]
    fn dispatch_covers_every_frame_carrying_type_without_unimplemented() {
        for obu_type in [
            ObuType::ClosedLoopKey,
            ObuType::OpenLoopKey,
            ObuType::LeadingTileGroup,
            ObuType::RegularTileGroup,
            ObuType::Switch,
            ObuType::RasFrame,
            ObuType::LeadingSef,
            ObuType::RegularSef,
            ObuType::LeadingTip,
            ObuType::RegularTip,
            ObuType::BridgeFrame,
        ] {
            let header_byte = obu_type.raw() << 2;
            let header = read_obu_header(&[header_byte], ByteOffset::new(0)).unwrap();
            assert_eq!(header.obu_type, obu_type);
            let payload: &[u8] = if obu_type.is_tile_group() {
                &[0x00] // is_first_tile_group=0, frame_header_present_flag=0
            } else {
                &[0xC0] // cur_mfh_id=uvlc(0), seq_header_id=uvlc(0) (bridge ignores the first)
            };
            let status = dispatch_obu_payload(header, payload, ByteOffset::new(1)).unwrap();
            assert!(
                matches!(status, PayloadStatus::PrefixParsed { .. }),
                "{obu_type:?} must dispatch its prefix, got {status:?}"
            );
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
