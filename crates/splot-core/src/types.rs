// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Strongly-typed AV2 OBU header identifiers (AV2 v1.0.0 § 5.2, § 6.2.2, Table 6.1).

use serde::{Deserialize, Serialize};

/// `obu_xlayer_id` value (31) denoting global scope (AV2 v1.0.0 § 3, `GLOBAL_XLAYER_ID`).
pub const GLOBAL_XLAYER_ID: ExtendedLayerId = ExtendedLayerId::from_bits(31);

/// `obu_tlayer_id`: temporal layer id of an OBU (2 bits; AV2 § 6.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TemporalLayerId(u8);

impl TemporalLayerId {
    /// Creates a temporal layer id from a value already known to fit the 2-bit
    /// `obu_tlayer_id` field (for example, just read from the bitstream).
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Creates a temporal layer id, returning `None` if `value > 3` (2 bits).
    #[must_use]
    pub const fn try_new(value: u8) -> Option<Self> {
        if value <= 3 { Some(Self(value)) } else { None }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// `obu_mlayer_id`: embedded layer id of an OBU (3 bits; AV2 § 6.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EmbeddedLayerId(u8);

impl EmbeddedLayerId {
    /// Creates an embedded layer id from a value already known to fit the 3-bit
    /// `obu_mlayer_id` field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Creates an embedded layer id, returning `None` if `value > 7` (3 bits).
    #[must_use]
    pub const fn try_new(value: u8) -> Option<Self> {
        if value <= 7 { Some(Self(value)) } else { None }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// `obu_xlayer_id`: extended layer id of an OBU (5 bits; AV2 § 6.2.2).
///
/// The value [`GLOBAL_XLAYER_ID`] (31) denotes global scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExtendedLayerId(u8);

impl ExtendedLayerId {
    /// Creates an extended layer id from a value already known to fit the 5-bit
    /// `obu_xlayer_id` field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Creates an extended layer id, returning `None` if `value > 31` (5 bits).
    #[must_use]
    pub const fn try_new(value: u8) -> Option<Self> {
        if value <= 31 { Some(Self(value)) } else { None }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Returns `true` if this is [`GLOBAL_XLAYER_ID`].
    #[must_use]
    pub const fn is_global(self) -> bool {
        self.0 == GLOBAL_XLAYER_ID.0
    }
}

/// AV2 OBU type (`obu_type`; AV2 v1.0.0 Table 6.1).
///
/// Raw values `26..=31` (and any other out-of-range value) are preserved as
/// [`ObuType::Reserved`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObuType {
    /// `0` — reserved.
    Reserved0,
    /// `1` — `OBU_SEQUENCE_HEADER`.
    SequenceHeader,
    /// `2` — `OBU_TEMPORAL_DELIMITER`.
    TemporalDelimiter,
    /// `3` — `OBU_MULTI_FRAME_HEADER`.
    MultiFrameHeader,
    /// `4` — `OBU_CLOSED_LOOP_KEY`.
    ClosedLoopKey,
    /// `5` — `OBU_OPEN_LOOP_KEY`.
    OpenLoopKey,
    /// `6` — `OBU_LEADING_TILE_GROUP`.
    LeadingTileGroup,
    /// `7` — `OBU_REGULAR_TILE_GROUP`.
    RegularTileGroup,
    /// `8` — `OBU_METADATA_SHORT`.
    MetadataShort,
    /// `9` — `OBU_METADATA_GROUP`.
    MetadataGroup,
    /// `10` — `OBU_SWITCH`.
    Switch,
    /// `11` — `OBU_LEADING_SEF`.
    LeadingSef,
    /// `12` — `OBU_REGULAR_SEF`.
    RegularSef,
    /// `13` — `OBU_LEADING_TIP`.
    LeadingTip,
    /// `14` — `OBU_REGULAR_TIP`.
    RegularTip,
    /// `15` — `OBU_BUFFER_REMOVAL_TIMING`.
    BufferRemovalTiming,
    /// `16` — `OBU_LAYER_CONFIGURATION_RECORD`.
    LayerConfigurationRecord,
    /// `17` — `OBU_ATLAS_SEGMENT`.
    AtlasSegment,
    /// `18` — `OBU_OPERATING_POINT_SET`.
    OperatingPointSet,
    /// `19` — `OBU_BRIDGE_FRAME`.
    BridgeFrame,
    /// `20` — `OBU_MSDO`.
    Msdo,
    /// `21` — `OBU_RAS_FRAME`.
    RasFrame,
    /// `22` — `OBU_QUANTIZATION_MATRIX`.
    QuantizationMatrix,
    /// `23` — `OBU_FILM_GRAIN`.
    FilmGrain,
    /// `24` — `OBU_CONTENT_INTERPRETATION`.
    ContentInterpretation,
    /// `25` — `OBU_PADDING`.
    Padding,
    /// `26..=31` (and any other raw value) — reserved for future use.
    Reserved(u8),
}

impl ObuType {
    /// Maps a raw 5-bit `obu_type` value to an [`ObuType`], preserving reserved values.
    #[must_use]
    pub const fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Reserved0,
            1 => Self::SequenceHeader,
            2 => Self::TemporalDelimiter,
            3 => Self::MultiFrameHeader,
            4 => Self::ClosedLoopKey,
            5 => Self::OpenLoopKey,
            6 => Self::LeadingTileGroup,
            7 => Self::RegularTileGroup,
            8 => Self::MetadataShort,
            9 => Self::MetadataGroup,
            10 => Self::Switch,
            11 => Self::LeadingSef,
            12 => Self::RegularSef,
            13 => Self::LeadingTip,
            14 => Self::RegularTip,
            15 => Self::BufferRemovalTiming,
            16 => Self::LayerConfigurationRecord,
            17 => Self::AtlasSegment,
            18 => Self::OperatingPointSet,
            19 => Self::BridgeFrame,
            20 => Self::Msdo,
            21 => Self::RasFrame,
            22 => Self::QuantizationMatrix,
            23 => Self::FilmGrain,
            24 => Self::ContentInterpretation,
            25 => Self::Padding,
            other => Self::Reserved(other),
        }
    }

    /// Returns the raw `obu_type` value.
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::Reserved0 => 0,
            Self::SequenceHeader => 1,
            Self::TemporalDelimiter => 2,
            Self::MultiFrameHeader => 3,
            Self::ClosedLoopKey => 4,
            Self::OpenLoopKey => 5,
            Self::LeadingTileGroup => 6,
            Self::RegularTileGroup => 7,
            Self::MetadataShort => 8,
            Self::MetadataGroup => 9,
            Self::Switch => 10,
            Self::LeadingSef => 11,
            Self::RegularSef => 12,
            Self::LeadingTip => 13,
            Self::RegularTip => 14,
            Self::BufferRemovalTiming => 15,
            Self::LayerConfigurationRecord => 16,
            Self::AtlasSegment => 17,
            Self::OperatingPointSet => 18,
            Self::BridgeFrame => 19,
            Self::Msdo => 20,
            Self::RasFrame => 21,
            Self::QuantizationMatrix => 22,
            Self::FilmGrain => 23,
            Self::ContentInterpretation => 24,
            Self::Padding => 25,
            Self::Reserved(value) => value,
        }
    }

    /// Returns the AV2 spec name (e.g. `"OBU_SEQUENCE_HEADER"`; AV2 Table 6.1).
    #[must_use]
    pub const fn spec_name(self) -> &'static str {
        match self {
            Self::Reserved0 | Self::Reserved(_) => "Reserved",
            Self::SequenceHeader => "OBU_SEQUENCE_HEADER",
            Self::TemporalDelimiter => "OBU_TEMPORAL_DELIMITER",
            Self::MultiFrameHeader => "OBU_MULTI_FRAME_HEADER",
            Self::ClosedLoopKey => "OBU_CLOSED_LOOP_KEY",
            Self::OpenLoopKey => "OBU_OPEN_LOOP_KEY",
            Self::LeadingTileGroup => "OBU_LEADING_TILE_GROUP",
            Self::RegularTileGroup => "OBU_REGULAR_TILE_GROUP",
            Self::MetadataShort => "OBU_METADATA_SHORT",
            Self::MetadataGroup => "OBU_METADATA_GROUP",
            Self::Switch => "OBU_SWITCH",
            Self::LeadingSef => "OBU_LEADING_SEF",
            Self::RegularSef => "OBU_REGULAR_SEF",
            Self::LeadingTip => "OBU_LEADING_TIP",
            Self::RegularTip => "OBU_REGULAR_TIP",
            Self::BufferRemovalTiming => "OBU_BUFFER_REMOVAL_TIMING",
            Self::LayerConfigurationRecord => "OBU_LAYER_CONFIGURATION_RECORD",
            Self::AtlasSegment => "OBU_ATLAS_SEGMENT",
            Self::OperatingPointSet => "OBU_OPERATING_POINT_SET",
            Self::BridgeFrame => "OBU_BRIDGE_FRAME",
            Self::Msdo => "OBU_MSDO",
            Self::RasFrame => "OBU_RAS_FRAME",
            Self::QuantizationMatrix => "OBU_QUANTIZATION_MATRIX",
            Self::FilmGrain => "OBU_FILM_GRAIN",
            Self::ContentInterpretation => "OBU_CONTENT_INTERPRETATION",
            Self::Padding => "OBU_PADDING",
        }
    }

    /// `true` for reserved OBU types (`0` or `26..=31`); conformant decoders ignore
    /// them (AV2 Table 6.1).
    #[must_use]
    pub const fn is_reserved(self) -> bool {
        matches!(self, Self::Reserved0 | Self::Reserved(_))
    }

    /// `is_tile_group()` per AV2 v1.0.0 § 5.2.1.
    #[must_use]
    pub const fn is_tile_group(self) -> bool {
        matches!(
            self,
            Self::LeadingTileGroup
                | Self::RegularTileGroup
                | Self::ClosedLoopKey
                | Self::OpenLoopKey
                | Self::Switch
                | Self::RasFrame
        )
    }

    /// `is_extensible_obu()` per AV2 v1.0.0 § 5.2.1.
    #[must_use]
    pub const fn is_extensible_obu(self) -> bool {
        matches!(
            self,
            Self::SequenceHeader
                | Self::MultiFrameHeader
                | Self::LayerConfigurationRecord
                | Self::ContentInterpretation
                | Self::OperatingPointSet
                | Self::AtlasSegment
        )
    }

    /// `is_tip_frame()` per AV2 v1.0.0 § 5.2.1.
    #[must_use]
    pub const fn is_tip_frame(self) -> bool {
        matches!(self, Self::LeadingTip | Self::RegularTip)
    }

    /// `is_sef()` (show-existing-frame) per AV2 v1.0.0 § 5.2.1.
    #[must_use]
    pub const fn is_sef(self) -> bool {
        matches!(self, Self::LeadingSef | Self::RegularSef)
    }

    /// `true` if `obu_xlayer_id` must equal [`GLOBAL_XLAYER_ID`] for this type
    /// (AV2 v1.0.0 § 5.2.2 / § 6.2.2: `OBU_MSDO`, `OBU_TEMPORAL_DELIMITER`).
    #[must_use]
    pub const fn requires_global_xlayer(self) -> bool {
        matches!(self, Self::Msdo | Self::TemporalDelimiter)
    }

    /// `true` if this type is permitted to carry `obu_xlayer_id == GLOBAL_XLAYER_ID`
    /// (AV2 v1.0.0 § 6.2.2).
    #[must_use]
    pub const fn permits_global_xlayer(self) -> bool {
        matches!(
            self,
            Self::TemporalDelimiter
                | Self::BufferRemovalTiming
                | Self::MetadataShort
                | Self::MetadataGroup
                | Self::LayerConfigurationRecord
                | Self::AtlasSegment
                | Self::OperatingPointSet
                | Self::Msdo
                | Self::Padding
        )
    }

    /// `true` if both `obu_tlayer_id` and `obu_mlayer_id` must be `0` for this type
    /// (AV2 v1.0.0 § 6.2.2).
    #[must_use]
    pub const fn requires_base_temporal_and_embedded_layer(self) -> bool {
        matches!(
            self,
            Self::SequenceHeader
                | Self::TemporalDelimiter
                | Self::LayerConfigurationRecord
                | Self::OperatingPointSet
                | Self::AtlasSegment
        )
    }

    /// `true` if `obu_tlayer_id` must be `0` for this type (AV2 v1.0.0 § 6.2.2:
    /// `OBU_CLOSED_LOOP_KEY`, `OBU_OPEN_LOOP_KEY`, `OBU_SWITCH`, `OBU_RAS_FRAME`).
    #[must_use]
    pub const fn requires_base_temporal_layer(self) -> bool {
        matches!(
            self,
            Self::ClosedLoopKey | Self::OpenLoopKey | Self::Switch | Self::RasFrame
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_round_trips_for_all_values() {
        for raw in 0u8..=31 {
            assert_eq!(ObuType::from_raw(raw).raw(), raw);
        }
    }

    #[test]
    fn reserved_values() {
        assert_eq!(ObuType::from_raw(0), ObuType::Reserved0);
        assert_eq!(ObuType::from_raw(26), ObuType::Reserved(26));
        assert!(ObuType::from_raw(31).is_reserved());
        assert!(!ObuType::SequenceHeader.is_reserved());
    }

    #[test]
    fn global_xlayer_constant() {
        assert_eq!(GLOBAL_XLAYER_ID.get(), 31);
        assert!(GLOBAL_XLAYER_ID.is_global());
        assert!(!ExtendedLayerId::from_bits(0).is_global());
    }

    #[test]
    fn checked_constructors_enforce_field_widths() {
        assert_eq!(
            TemporalLayerId::try_new(3).map(TemporalLayerId::get),
            Some(3)
        );
        assert!(TemporalLayerId::try_new(4).is_none());
        assert_eq!(
            EmbeddedLayerId::try_new(7).map(EmbeddedLayerId::get),
            Some(7)
        );
        assert!(EmbeddedLayerId::try_new(8).is_none());
        assert_eq!(
            ExtendedLayerId::try_new(31).map(ExtendedLayerId::get),
            Some(31)
        );
        assert!(ExtendedLayerId::try_new(32).is_none());
    }

    #[test]
    fn helper_predicates_match_spec() {
        assert!(ObuType::Msdo.requires_global_xlayer());
        assert!(ObuType::TemporalDelimiter.requires_global_xlayer());
        assert!(!ObuType::SequenceHeader.requires_global_xlayer());

        assert!(ObuType::RegularTileGroup.is_tile_group());
        assert!(ObuType::Switch.is_tile_group());
        assert!(!ObuType::SequenceHeader.is_tile_group());

        assert!(ObuType::SequenceHeader.is_extensible_obu());
        assert!(ObuType::AtlasSegment.is_extensible_obu());
        assert!(!ObuType::TemporalDelimiter.is_extensible_obu());

        assert!(ObuType::AtlasSegment.requires_base_temporal_and_embedded_layer());
        assert!(ObuType::ClosedLoopKey.requires_base_temporal_layer());
        assert!(ObuType::Padding.permits_global_xlayer());
        assert!(!ObuType::SequenceHeader.permits_global_xlayer());
    }
}
