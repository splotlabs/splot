// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 metadata OBU syntax model (AV2 v1.0.0 § 5.17 / § 6.16).
//!
//! Two OBU types carry metadata: `OBU_METADATA_SHORT` (`metadata_short_obu()`, § 5.17.2)
//! provides a compact 1-byte header, and `OBU_METADATA_GROUP` (`metadata_group_obu()`,
//! § 5.17.3) carries multiple metadata units with per-unit headers, layer targeting, and
//! priority. Both share `metadata_unit()` (§ 5.17.1), which dispatches on `metadata_type`
//! to the typed § 5.17.4-§ 5.17.13 child payloads.
//!
//! Every `metadata_unit()` is bounded to exactly its declared `metadataPayloadSize`
//! bytes (via [`BitReader::take_bytes`]), so child syntax cannot overread its declared
//! size and the `metadata_unit_remaining_bit` padding (§ 6.16.1, "can take any value") is
//! skipped. Reserved / unknown / private `metadata_type` values are preserved as
//! [`MetadataPayload::UnknownRaw`] rather than returning [`Error::Unimplemented`].
//! Variable-length blob payloads (ITU-T T.35, ICC, user data) retain only their byte
//! length so callers never dump unbounded raw bytes.

use crate::bitio::BitReader;
use crate::error::{Error, MetadataErrorKind, Result};
use crate::types::ExtendedLayerId;

/// `muh_layer_idc` value `LAYER_VALUES` (AV2 § 6.16.3): the metadata applies to an
/// explicitly signaled set of layer values, so the `muh_xlayer_map` / `muh_mlayer_map`
/// maps are present.
const LAYER_VALUES: u8 = 3;

/// AV2 `metadata_type` (AV2 v1.0.0 § 6.16, Table 6.17).
///
/// Values `0` and `11` and greater are reserved for AOMedia use and are preserved as
/// [`MetadataType::Reserved`] so the raw value round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataType {
    /// `1` — `METADATA_TYPE_HDR_CLL`.
    HdrCll,
    /// `2` — `METADATA_TYPE_HDR_MDCV`.
    HdrMdcv,
    /// `3` — `METADATA_TYPE_ITUT_T35`.
    ItutT35,
    /// `4` — `METADATA_TYPE_TIMECODE`.
    Timecode,
    /// `5` — `METADATA_TYPE_DECODED_FRAME_HASH`.
    DecodedFrameHash,
    /// `6` — `METADATA_TYPE_BANDING_HINTS`.
    BandingHints,
    /// `7` — `METADATA_TYPE_ICC_PROFILE`.
    IccProfile,
    /// `8` — `METADATA_TYPE_SCAN_TYPE`.
    ScanType,
    /// `9` — `METADATA_TYPE_TEMPORAL_POINT_INFO`.
    TemporalPointInfo,
    /// `10` — `METADATA_TYPE_USER_DATA_UNREGISTERED`.
    UserDataUnregistered,
    /// `0`, or `11` and greater — reserved for AOMedia use (preserves the raw value).
    Reserved(u32),
}

impl MetadataType {
    /// Classifies a raw `metadata_type` value (AV2 § 6.16, Table 6.17).
    #[must_use]
    pub const fn from_value(value: u32) -> Self {
        match value {
            1 => Self::HdrCll,
            2 => Self::HdrMdcv,
            3 => Self::ItutT35,
            4 => Self::Timecode,
            5 => Self::DecodedFrameHash,
            6 => Self::BandingHints,
            7 => Self::IccProfile,
            8 => Self::ScanType,
            9 => Self::TemporalPointInfo,
            10 => Self::UserDataUnregistered,
            other => Self::Reserved(other),
        }
    }

    /// Returns the raw `metadata_type` value.
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::HdrCll => 1,
            Self::HdrMdcv => 2,
            Self::ItutT35 => 3,
            Self::Timecode => 4,
            Self::DecodedFrameHash => 5,
            Self::BandingHints => 6,
            Self::IccProfile => 7,
            Self::ScanType => 8,
            Self::TemporalPointInfo => 9,
            Self::UserDataUnregistered => 10,
            Self::Reserved(value) => value,
        }
    }

    /// Returns the AV2 Table 6.17 name (e.g. `"METADATA_TYPE_HDR_CLL"`), or `"Reserved"`
    /// for a reserved value.
    #[must_use]
    pub const fn spec_name(self) -> &'static str {
        match self {
            Self::HdrCll => "METADATA_TYPE_HDR_CLL",
            Self::HdrMdcv => "METADATA_TYPE_HDR_MDCV",
            Self::ItutT35 => "METADATA_TYPE_ITUT_T35",
            Self::Timecode => "METADATA_TYPE_TIMECODE",
            Self::DecodedFrameHash => "METADATA_TYPE_DECODED_FRAME_HASH",
            Self::BandingHints => "METADATA_TYPE_BANDING_HINTS",
            Self::IccProfile => "METADATA_TYPE_ICC_PROFILE",
            Self::ScanType => "METADATA_TYPE_SCAN_TYPE",
            Self::TemporalPointInfo => "METADATA_TYPE_TEMPORAL_POINT_INFO",
            Self::UserDataUnregistered => "METADATA_TYPE_USER_DATA_UNREGISTERED",
            Self::Reserved(_) => "Reserved",
        }
    }
}

/// `metadata_hdr_cll()` payload (AV2 v1.0.0 § 5.17.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataHdrCll {
    /// `max_cll` (`f(16)`).
    pub max_cll: u16,
    /// `max_fall` (`f(16)`).
    pub max_fall: u16,
}

/// `metadata_hdr_mdcv()` payload (AV2 v1.0.0 § 5.17.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataHdrMdcv {
    /// `primary_chromaticity_x[i]` (`f(16)`).
    pub primary_chromaticity_x: [u16; 3],
    /// `primary_chromaticity_y[i]` (`f(16)`).
    pub primary_chromaticity_y: [u16; 3],
    /// `white_point_chromaticity_x` (`f(16)`).
    pub white_point_chromaticity_x: u16,
    /// `white_point_chromaticity_y` (`f(16)`).
    pub white_point_chromaticity_y: u16,
    /// `luminance_max` (`f(32)`).
    pub luminance_max: u32,
    /// `luminance_min` (`f(32)`).
    pub luminance_min: u32,
}

/// `metadata_itut_t35()` payload (AV2 v1.0.0 § 5.17.4). The registered payload bytes are
/// summarized by length so the inspector never dumps unbounded raw data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataItutT35 {
    /// `itu_t_t35_country_code` (`f(8)`).
    pub itu_t_t35_country_code: u8,
    /// `itu_t_t35_country_code_extension_byte`, present only when the country code is
    /// `0xFF`.
    pub itu_t_t35_country_code_extension_byte: Option<u8>,
    /// Number of `itu_t_t35_payload_bytes` (`t35PayloadSize`).
    pub payload_len: usize,
}

/// `metadata_timecode()` payload (AV2 v1.0.0 § 5.17.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataTimecode {
    /// `counting_type` (`f(5)`).
    pub counting_type: u8,
    /// `full_timestamp_flag` (`f(1)`).
    pub full_timestamp_flag: bool,
    /// `discontinuity_flag` (`f(1)`).
    pub discontinuity_flag: bool,
    /// `cnt_dropped_flag` (`f(1)`).
    pub cnt_dropped_flag: bool,
    /// `n_frames` (`f(9)`).
    pub n_frames: u16,
    /// `seconds_value` (`f(6)`), when present.
    pub seconds_value: Option<u8>,
    /// `minutes_value` (`f(6)`), when present.
    pub minutes_value: Option<u8>,
    /// `hours_value` (`f(5)`), when present.
    pub hours_value: Option<u8>,
    /// `time_offset_length` (`f(5)`).
    pub time_offset_length: u8,
    /// `time_offset_value` (`f(time_offset_length)`), present when the length is non-zero.
    pub time_offset_value: Option<u32>,
}

/// `metadata_scan_type()` payload (AV2 v1.0.0 § 5.17.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataScanType {
    /// `mps_pic_struct_type` (`f(5)`).
    pub mps_pic_struct_type: u8,
    /// `mps_source_scan_type_idc` (`f(2)`).
    pub mps_source_scan_type_idc: u8,
    /// `mps_duplicate_flag` (`f(1)`).
    pub mps_duplicate_flag: bool,
}

/// `metadata_temporal_point_info()` payload (AV2 v1.0.0 § 5.17.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataTemporalPointInfo {
    /// `frame_presentation_time` (`leb128()`).
    pub frame_presentation_time: u32,
}

/// `metadata_decoded_frame_hash()` payload (AV2 v1.0.0 § 5.17.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataDecodedFrameHash {
    /// `hash_type` (`f(4)`).
    pub hash_type: u8,
    /// `per_plane` (`f(1)`).
    pub per_plane: bool,
    /// `has_grain` (`f(1)`).
    pub has_grain: bool,
    /// `is_monochrome` (`f(1)`).
    pub is_monochrome: bool,
    /// `reserved` (`f(1)`).
    pub reserved: u8,
    /// `plane_hash[i]` (`le(16)`), present when `per_plane` is set (1 plane if
    /// `is_monochrome`, else 3).
    pub plane_hashes: Vec<[u8; 16]>,
    /// `frame_hash` (`le(16)`), present when `per_plane` is clear.
    pub frame_hash: Option<[u8; 16]>,
}

/// `metadata_icc_profile()` payload (AV2 v1.0.0 § 5.17.9). The profile bytes are
/// summarized by length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataIccProfile {
    /// Number of `icc_profile_data_payload_bytes` (`metadataPayloadSize`).
    pub payload_len: usize,
}

/// `metadata_user_data_unregistered()` payload (AV2 v1.0.0 § 5.17.13). The user data
/// bytes are summarized by length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataUserDataUnregistered {
    /// `uuid_iso_iec_11578` (`f(128)`).
    pub uuid_iso_iec_11578: [u8; 16],
    /// Number of `user_data_payload_byte` values (`metadataPayloadSize - 16`).
    pub payload_len: usize,
}

/// A reserved / unknown / private metadata payload (AV2 § 6.16.1 NOTE): the raw bytes are
/// preserved by length only and child parsing is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataUnknownRaw {
    /// Number of raw payload bytes (`metadataPayloadSize`).
    pub raw_len: usize,
}

/// One per-component banding entry inside `metadata_banding_hints()` (AV2 § 5.17.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandingComponent {
    /// `banding_in_component_present_flag` (`f(1)`).
    pub banding_in_component_present_flag: bool,
    /// `max_band_width_minus_4` (`f(6)`), present when banding is present.
    pub max_band_width_minus_4: Option<u8>,
    /// `max_band_step_minus_1` (`f(4)`), present when banding is present.
    pub max_band_step_minus_1: Option<u8>,
}

/// `varying_size_band_units_flag` payload of `metadata_banding_hints()` (AV2 § 5.17.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaryingBandUnits {
    /// `band_block_in_luma_samples` (`f(3)`).
    pub band_block_in_luma_samples: u8,
    /// `vert_size_in_band_blocks_minus_1[r]` (`f(5)`), one per band-unit row.
    pub vert_size_in_band_blocks_minus_1: Vec<u8>,
    /// `horz_size_in_band_blocks_minus_1[c]` (`f(5)`), one per band-unit column.
    pub horz_size_in_band_blocks_minus_1: Vec<u8>,
}

/// `band_units_information_present_flag` payload of `metadata_banding_hints()`
/// (AV2 § 5.17.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandUnits {
    /// `num_band_units_rows_minus_1` (`f(5)`).
    pub num_band_units_rows_minus_1: u8,
    /// `num_band_units_cols_minus_1` (`f(5)`).
    pub num_band_units_cols_minus_1: u8,
    /// Varying-size band-unit info, present when `varying_size_band_units_flag` is set.
    pub varying_size: Option<VaryingBandUnits>,
    /// `banding_in_band_unit_present_flag[r][c]` (`f(1)`), row-major over
    /// `(rows + 1) * (cols + 1)` band units.
    pub banding_in_band_unit_present: Vec<bool>,
}

/// `banding_hints_flag` payload of `metadata_banding_hints()` (AV2 § 5.17.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandingHintsDetail {
    /// `three_color_components_flag` (`f(1)`).
    pub three_color_components_flag: bool,
    /// One entry per signaled color component (3 if `three_color_components_flag`, else 1).
    pub components: Vec<BandingComponent>,
    /// Band-units info, present when `band_units_information_present_flag` is set.
    pub band_units: Option<BandUnits>,
}

/// `metadata_banding_hints()` payload (AV2 v1.0.0 § 5.17.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataBandingHints {
    /// `coding_banding_present_flag` (`f(1)`).
    pub coding_banding_present_flag: bool,
    /// `source_banding_present_flag` (`f(1)`).
    pub source_banding_present_flag: bool,
    /// Banding-hint detail, present when `coding_banding_present_flag` and
    /// `banding_hints_flag` are both set.
    pub hints: Option<BandingHintsDetail>,
}

/// A typed metadata payload (AV2 v1.0.0 § 5.17.4-§ 5.17.13), selected by `metadata_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataPayload {
    /// `metadata_hdr_cll()` (§ 5.17.5).
    HdrCll(MetadataHdrCll),
    /// `metadata_hdr_mdcv()` (§ 5.17.6).
    HdrMdcv(MetadataHdrMdcv),
    /// `metadata_itut_t35()` (§ 5.17.4).
    ItutT35(MetadataItutT35),
    /// `metadata_timecode()` (§ 5.17.7).
    Timecode(MetadataTimecode),
    /// `metadata_decoded_frame_hash()` (§ 5.17.12).
    DecodedFrameHash(MetadataDecodedFrameHash),
    /// `metadata_banding_hints()` (§ 5.17.8).
    BandingHints(MetadataBandingHints),
    /// `metadata_icc_profile()` (§ 5.17.9).
    IccProfile(MetadataIccProfile),
    /// `metadata_scan_type()` (§ 5.17.10).
    ScanType(MetadataScanType),
    /// `metadata_temporal_point_info()` (§ 5.17.11).
    TemporalPointInfo(MetadataTemporalPointInfo),
    /// `metadata_user_data_unregistered()` (§ 5.17.13).
    UserDataUnregistered(MetadataUserDataUnregistered),
    /// A reserved / unknown / private metadata payload preserved as raw (§ 6.16.1 NOTE).
    UnknownRaw(MetadataUnknownRaw),
}

/// A parsed `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataUnit {
    /// The `metadata_type` that selected the payload.
    pub metadata_type: MetadataType,
    /// `metadataPayloadSize`: the declared payload size in bytes.
    pub payload_size: usize,
    /// The typed child payload.
    pub payload: MetadataPayload,
}

/// A parsed `metadata_short_obu(obuPayloadSize)` (AV2 v1.0.0 § 5.17.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataShortObu {
    /// `metadata_is_suffix` (`f(1)`).
    pub metadata_is_suffix: bool,
    /// `muh_layer_idc` (`f(3)`). AV2 § 6.16.2 requires this to be less than 3.
    pub muh_layer_idc: u8,
    /// `muh_cancel_flag` (`f(1)`).
    pub muh_cancel_flag: bool,
    /// `muh_persistence_idc` (`f(3)`).
    pub muh_persistence_idc: u8,
    /// `metadata_type` (`leb128()`).
    pub metadata_type: MetadataType,
    /// `Leb128Bytes` consumed by `metadata_type` (drives `metadataPayloadSize`).
    pub metadata_type_leb128_bytes: u8,
    /// The metadata unit, absent when `muh_cancel_flag` is set.
    pub unit: Option<MetadataUnit>,
}

/// A parsed metadata group unit header plus its `metadata_unit()` (AV2 v1.0.0 § 5.17.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGroupUnit {
    /// `metadata_type` (`leb128()`).
    pub metadata_type: MetadataType,
    /// `muh_header_size` (`f(7)`).
    pub muh_header_size: u8,
    /// `muh_cancel_flag` (`f(1)`).
    pub muh_cancel_flag: bool,
    /// `muh_payload_size` (`leb128()`), absent when `muh_cancel_flag` is set.
    pub muh_payload_size: Option<u32>,
    /// `muh_layer_idc` (`f(3)`), absent when `muh_cancel_flag` is set.
    pub muh_layer_idc: Option<u8>,
    /// `muh_persistence_idc` (`f(3)`), absent when `muh_cancel_flag` is set.
    pub muh_persistence_idc: Option<u8>,
    /// `muh_priority` (`f(8)`), absent when `muh_cancel_flag` is set.
    pub muh_priority: Option<u8>,
    /// `muh_reserved_zero_2bits` (`f(2)`), absent when `muh_cancel_flag` is set. AV2
    /// § 6.16.3 requires this to be zero.
    pub muh_reserved_zero_2bits: Option<u8>,
    /// `muh_xlayer_map` (`f(32)`), present only when `muh_layer_idc == LAYER_VALUES` and
    /// the OBU is global. AV2 § 6.16.3 requires bit 31 to be zero.
    pub muh_xlayer_map: Option<u32>,
    /// The `muh_mlayer_map` (`f(8)`) bytes: one per set `muh_xlayer_map` bit when global,
    /// a single byte when local. AV2 § 6.16.3 requires bit `m` to be zero for `m` less
    /// than `obu_mlayer_id`.
    pub muh_mlayer_maps: Vec<u8>,
    /// Number of `muh_header_extension_byte` values (consumed and ignored).
    pub header_extension_len: usize,
    /// The metadata unit, absent when `muh_cancel_flag` is set.
    pub unit: Option<MetadataUnit>,
}

/// A parsed `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGroupObu {
    /// `metadata_is_suffix` (`f(1)`).
    pub metadata_is_suffix: bool,
    /// `metadata_necessity_idc` (`f(2)`).
    pub metadata_necessity_idc: u8,
    /// `metadata_application_id` (`f(5)`).
    pub metadata_application_id: u8,
    /// The metadata units (`metadata_unit_cnt_minus_1 + 1` entries).
    pub units: Vec<MetadataGroupUnit>,
}

/// Reads a `leb128()` value and the number of bytes it occupied (`Leb128Bytes`).
///
/// `leb128()` only appears at byte-aligned positions in AV2 syntax, so the byte-offset
/// delta is exactly the number of bytes consumed (`1..=8`).
fn read_leb128_bytes(reader: &mut BitReader<'_>) -> Result<(u32, u8)> {
    let before = reader.byte_offset().get();
    let value = reader.read_leb128()?;
    let after = reader.byte_offset().get();
    let bytes = after.saturating_sub(before) as u8;
    Ok((value, bytes))
}

/// Builds an `Error::InvalidMetadata` located at `reader` with the given `kind`.
fn metadata_error(reader: &BitReader<'_>, kind: MetadataErrorKind) -> Error {
    Error::InvalidMetadata {
        offset: reader.byte_offset(),
        bit_offset: reader.bit_offset(),
        kind,
    }
}

/// Parses `metadata_short_obu(obuPayloadSize)` (AV2 v1.0.0 § 5.17.2).
///
/// `reader` reads from the OBU payload and `obu_payload_size` is `obuPayloadSize` (the
/// payload length). On `muh_cancel_flag` the parser returns after `metadata_type`,
/// leaving the reader positioned for the OBU `trailing_bits()`. Otherwise it parses the
/// bounded `metadata_unit(metadataPayloadSize)` and leaves the reader positioned for the
/// trailing bits.
///
/// # Errors
/// Returns [`Error::InvalidMetadata`] with [`MetadataErrorKind::UnitPayloadUnderflow`]
/// when `obuPayloadSize - 2 - Leb128Bytes` underflows or the metadata unit child syntax
/// overruns its declared size, or a descriptor / [`Error::UnexpectedEof`] error for
/// truncated input.
pub fn parse_metadata_short(
    reader: &mut BitReader<'_>,
    obu_payload_size: usize,
) -> Result<MetadataShortObu> {
    let first = reader.read_bits_u8(8)?;
    let metadata_is_suffix = (first & 0x80) != 0;
    let muh_layer_idc = (first >> 4) & 0x07;
    let muh_cancel_flag = (first & 0x08) != 0;
    let muh_persistence_idc = first & 0x07;

    let (metadata_type_value, metadata_type_leb128_bytes) = read_leb128_bytes(reader)?;
    let metadata_type = MetadataType::from_value(metadata_type_value);

    let unit = if muh_cancel_flag {
        None
    } else {
        let metadata_payload_size = obu_payload_size
            .checked_sub(2)
            .and_then(|value| value.checked_sub(usize::from(metadata_type_leb128_bytes)))
            .ok_or_else(|| metadata_error(reader, MetadataErrorKind::UnitPayloadUnderflow))?;
        let mut unit_reader = reader.take_bytes(metadata_payload_size)?;
        Some(parse_metadata_unit(
            &mut unit_reader,
            metadata_payload_size,
            metadata_type,
        )?)
    };

    Ok(MetadataShortObu {
        metadata_is_suffix,
        muh_layer_idc,
        muh_cancel_flag,
        muh_persistence_idc,
        metadata_type,
        metadata_type_leb128_bytes,
        unit,
    })
}

/// Parses `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3).
///
/// `obu_xlayer_id` comes from the OBU header and selects the layer-map branch
/// (`muh_xlayer_map` is present only for a global OBU). The parser bounds each
/// `metadata_unit(muh_payload_size)` and leaves the reader positioned for the OBU
/// `trailing_bits()`. The `obu_mlayer_id` bound on `muh_mlayer_map` is a § 6.16.3
/// conformance check handled by the validator, which reads the parsed
/// [`MetadataGroupUnit::muh_mlayer_maps`].
///
/// # Errors
/// Returns [`Error::InvalidMetadata`] with [`MetadataErrorKind::GroupUnitCountTooLarge`]
/// for `metadata_unit_cnt_minus_1 >= 16383`, [`MetadataErrorKind::GroupHeaderUnderflow`]
/// for a negative `headerRemainingBytes`, [`MetadataErrorKind::UnitPayloadUnderflow`] for
/// an overrun metadata unit, or a descriptor / [`Error::UnexpectedEof`] error for
/// truncated input.
pub fn parse_metadata_group(
    reader: &mut BitReader<'_>,
    obu_xlayer_id: ExtendedLayerId,
) -> Result<MetadataGroupObu> {
    let first = reader.read_bits_u8(8)?;
    let metadata_is_suffix = (first & 0x80) != 0;
    let metadata_necessity_idc = (first >> 5) & 0x03;
    let metadata_application_id = first & 0x1F;

    let (metadata_unit_cnt_minus_1, _) = read_leb128_bytes(reader)?;
    if metadata_unit_cnt_minus_1 >= 16383 {
        return Err(metadata_error(
            reader,
            MetadataErrorKind::GroupUnitCountTooLarge,
        ));
    }

    let unit_count = metadata_unit_cnt_minus_1 + 1;
    let mut units = Vec::new();
    for _ in 0..unit_count {
        units.push(parse_metadata_group_unit(reader, obu_xlayer_id)?);
    }

    Ok(MetadataGroupObu {
        metadata_is_suffix,
        metadata_necessity_idc,
        metadata_application_id,
        units,
    })
}

fn parse_metadata_group_unit(
    reader: &mut BitReader<'_>,
    obu_xlayer_id: ExtendedLayerId,
) -> Result<MetadataGroupUnit> {
    let (metadata_type_value, _) = read_leb128_bytes(reader)?;
    let metadata_type = MetadataType::from_value(metadata_type_value);

    let header_byte = reader.read_bits_u8(8)?;
    let muh_header_size = header_byte >> 1;
    let muh_cancel_flag = (header_byte & 0x01) != 0;

    let mut header_remaining = i32::from(muh_header_size);
    let mut muh_payload_size = None;
    let mut muh_layer_idc = None;
    let mut muh_persistence_idc = None;
    let mut muh_priority = None;
    let mut muh_reserved_zero_2bits = None;
    let mut muh_xlayer_map = None;
    let mut muh_mlayer_maps = Vec::new();

    if !muh_cancel_flag {
        let (payload_size, payload_size_bytes) = read_leb128_bytes(reader)?;
        muh_payload_size = Some(payload_size);
        header_remaining -= i32::from(payload_size_bytes);

        let layer_idc = reader.read_bits_u8(3)?;
        muh_persistence_idc = Some(reader.read_bits_u8(3)?);
        muh_priority = Some(reader.read_bits_u8(8)?);
        muh_reserved_zero_2bits = Some(reader.read_bits_u8(2)?);
        muh_layer_idc = Some(layer_idc);
        header_remaining -= 2;

        if layer_idc == LAYER_VALUES {
            if obu_xlayer_id.is_global() {
                let xlayer_map = reader.read_bits(32)?;
                muh_xlayer_map = Some(xlayer_map);
                header_remaining -= 4;
                for n in 0..31u32 {
                    if xlayer_map & (1 << n) != 0 {
                        muh_mlayer_maps.push(reader.read_bits_u8(8)?);
                        header_remaining -= 1;
                    }
                }
            } else {
                muh_mlayer_maps.push(reader.read_bits_u8(8)?);
                header_remaining -= 1;
            }
        }
    }

    if header_remaining < 0 {
        return Err(metadata_error(
            reader,
            MetadataErrorKind::GroupHeaderUnderflow,
        ));
    }

    let header_extension_len = header_remaining as usize;
    for _ in 0..header_extension_len {
        reader.read_bits_u8(8)?;
    }

    let unit = match muh_payload_size {
        Some(payload_size) => {
            let payload_size = payload_size as usize;
            let mut unit_reader = reader.take_bytes(payload_size)?;
            Some(parse_metadata_unit(
                &mut unit_reader,
                payload_size,
                metadata_type,
            )?)
        }
        None => None,
    };

    Ok(MetadataGroupUnit {
        metadata_type,
        muh_header_size,
        muh_cancel_flag,
        muh_payload_size,
        muh_layer_idc,
        muh_persistence_idc,
        muh_priority,
        muh_reserved_zero_2bits,
        muh_xlayer_map,
        muh_mlayer_maps,
        header_extension_len,
        unit,
    })
}

/// Parses `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1) from a sub-reader
/// already bounded to exactly `payload_size` bytes.
fn parse_metadata_unit(
    reader: &mut BitReader<'_>,
    payload_size: usize,
    metadata_type: MetadataType,
) -> Result<MetadataUnit> {
    let payload = match parse_metadata_payload(reader, payload_size, metadata_type) {
        Ok(payload) => payload,
        Err(Error::UnexpectedEof { offset, .. }) => {
            return Err(Error::InvalidMetadata {
                offset,
                bit_offset: crate::span::BitOffset::from_bits(0),
                kind: MetadataErrorKind::UnitPayloadUnderflow,
            });
        }
        Err(other) => return Err(other),
    };
    Ok(MetadataUnit {
        metadata_type,
        payload_size,
        payload,
    })
}

fn parse_metadata_payload(
    reader: &mut BitReader<'_>,
    payload_size: usize,
    metadata_type: MetadataType,
) -> Result<MetadataPayload> {
    Ok(match metadata_type {
        MetadataType::ItutT35 => MetadataPayload::ItutT35(parse_itut_t35(reader, payload_size)?),
        MetadataType::HdrCll => MetadataPayload::HdrCll(parse_hdr_cll(reader)?),
        MetadataType::HdrMdcv => MetadataPayload::HdrMdcv(parse_hdr_mdcv(reader)?),
        MetadataType::Timecode => MetadataPayload::Timecode(parse_timecode(reader)?),
        MetadataType::BandingHints => MetadataPayload::BandingHints(parse_banding_hints(reader)?),
        MetadataType::IccProfile => MetadataPayload::IccProfile(MetadataIccProfile {
            payload_len: payload_size,
        }),
        MetadataType::ScanType => MetadataPayload::ScanType(parse_scan_type(reader)?),
        MetadataType::TemporalPointInfo => {
            MetadataPayload::TemporalPointInfo(MetadataTemporalPointInfo {
                frame_presentation_time: reader.read_leb128()?,
            })
        }
        MetadataType::DecodedFrameHash => {
            MetadataPayload::DecodedFrameHash(parse_decoded_frame_hash(reader)?)
        }
        MetadataType::UserDataUnregistered => MetadataPayload::UserDataUnregistered(
            parse_user_data_unregistered(reader, payload_size)?,
        ),
        MetadataType::Reserved(_) => MetadataPayload::UnknownRaw(MetadataUnknownRaw {
            raw_len: payload_size,
        }),
    })
}

fn parse_hdr_cll(reader: &mut BitReader<'_>) -> Result<MetadataHdrCll> {
    Ok(MetadataHdrCll {
        max_cll: reader.read_bits(16)? as u16,
        max_fall: reader.read_bits(16)? as u16,
    })
}

fn parse_hdr_mdcv(reader: &mut BitReader<'_>) -> Result<MetadataHdrMdcv> {
    let mut primary_chromaticity_x = [0u16; 3];
    let mut primary_chromaticity_y = [0u16; 3];
    for i in 0..3 {
        primary_chromaticity_x[i] = reader.read_bits(16)? as u16;
        primary_chromaticity_y[i] = reader.read_bits(16)? as u16;
    }
    Ok(MetadataHdrMdcv {
        primary_chromaticity_x,
        primary_chromaticity_y,
        white_point_chromaticity_x: reader.read_bits(16)? as u16,
        white_point_chromaticity_y: reader.read_bits(16)? as u16,
        luminance_max: reader.read_bits(32)?,
        luminance_min: reader.read_bits(32)?,
    })
}

fn parse_itut_t35(reader: &mut BitReader<'_>, payload_size: usize) -> Result<MetadataItutT35> {
    let itu_t_t35_country_code = reader.read_bits_u8(8)?;
    let mut consumed = 1usize;
    let itu_t_t35_country_code_extension_byte = if itu_t_t35_country_code == 0xFF {
        let byte = reader.read_bits_u8(8)?;
        consumed += 1;
        Some(byte)
    } else {
        None
    };
    let payload_len = payload_size.saturating_sub(consumed);
    Ok(MetadataItutT35 {
        itu_t_t35_country_code,
        itu_t_t35_country_code_extension_byte,
        payload_len,
    })
}

fn parse_timecode(reader: &mut BitReader<'_>) -> Result<MetadataTimecode> {
    let counting_type = reader.read_bits_u8(5)?;
    let full_timestamp_flag = reader.read_flag()?;
    let discontinuity_flag = reader.read_flag()?;
    let cnt_dropped_flag = reader.read_flag()?;
    let n_frames = reader.read_bits(9)? as u16;

    let mut seconds_value = None;
    let mut minutes_value = None;
    let mut hours_value = None;
    if full_timestamp_flag {
        seconds_value = Some(reader.read_bits_u8(6)?);
        minutes_value = Some(reader.read_bits_u8(6)?);
        hours_value = Some(reader.read_bits_u8(5)?);
    } else {
        let seconds_flag = reader.read_flag()?;
        if seconds_flag {
            seconds_value = Some(reader.read_bits_u8(6)?);
            let minutes_flag = reader.read_flag()?;
            if minutes_flag {
                minutes_value = Some(reader.read_bits_u8(6)?);
                let hours_flag = reader.read_flag()?;
                if hours_flag {
                    hours_value = Some(reader.read_bits_u8(5)?);
                }
            }
        }
    }

    let time_offset_length = reader.read_bits_u8(5)?;
    let time_offset_value = if time_offset_length > 0 {
        Some(reader.read_bits(u32::from(time_offset_length))?)
    } else {
        None
    };

    Ok(MetadataTimecode {
        counting_type,
        full_timestamp_flag,
        discontinuity_flag,
        cnt_dropped_flag,
        n_frames,
        seconds_value,
        minutes_value,
        hours_value,
        time_offset_length,
        time_offset_value,
    })
}

fn parse_scan_type(reader: &mut BitReader<'_>) -> Result<MetadataScanType> {
    Ok(MetadataScanType {
        mps_pic_struct_type: reader.read_bits_u8(5)?,
        mps_source_scan_type_idc: reader.read_bits_u8(2)?,
        mps_duplicate_flag: reader.read_flag()?,
    })
}

fn parse_decoded_frame_hash(reader: &mut BitReader<'_>) -> Result<MetadataDecodedFrameHash> {
    let hash_type = reader.read_bits_u8(4)?;
    let per_plane = reader.read_flag()?;
    let has_grain = reader.read_flag()?;
    let is_monochrome = reader.read_flag()?;
    let reserved = reader.read_bits_u8(1)?;

    let mut plane_hashes = Vec::new();
    let mut frame_hash = None;
    if per_plane {
        let num_planes = if is_monochrome { 1 } else { 3 };
        for _ in 0..num_planes {
            plane_hashes.push(read_hash16(reader)?);
        }
    } else {
        frame_hash = Some(read_hash16(reader)?);
    }

    Ok(MetadataDecodedFrameHash {
        hash_type,
        per_plane,
        has_grain,
        is_monochrome,
        reserved,
        plane_hashes,
        frame_hash,
    })
}

/// Reads a 16-byte hash value (`le(16)`), preserving the bytes in bitstream order.
fn read_hash16(reader: &mut BitReader<'_>) -> Result<[u8; 16]> {
    let mut hash = [0u8; 16];
    for byte in &mut hash {
        *byte = reader.read_bits_u8(8)?;
    }
    Ok(hash)
}

fn parse_user_data_unregistered(
    reader: &mut BitReader<'_>,
    payload_size: usize,
) -> Result<MetadataUserDataUnregistered> {
    let mut uuid_iso_iec_11578 = [0u8; 16];
    for byte in &mut uuid_iso_iec_11578 {
        *byte = reader.read_bits_u8(8)?;
    }
    let payload_len = payload_size.saturating_sub(16);
    Ok(MetadataUserDataUnregistered {
        uuid_iso_iec_11578,
        payload_len,
    })
}

fn parse_banding_hints(reader: &mut BitReader<'_>) -> Result<MetadataBandingHints> {
    let coding_banding_present_flag = reader.read_flag()?;
    let source_banding_present_flag = reader.read_flag()?;
    let hints = if coding_banding_present_flag {
        let banding_hints_flag = reader.read_flag()?;
        if banding_hints_flag {
            Some(parse_banding_hints_detail(reader)?)
        } else {
            None
        }
    } else {
        None
    };
    Ok(MetadataBandingHints {
        coding_banding_present_flag,
        source_banding_present_flag,
        hints,
    })
}

fn parse_banding_hints_detail(reader: &mut BitReader<'_>) -> Result<BandingHintsDetail> {
    let three_color_components_flag = reader.read_flag()?;
    let num_components = if three_color_components_flag { 3 } else { 1 };
    let mut components = Vec::with_capacity(num_components);
    for _ in 0..num_components {
        let banding_in_component_present_flag = reader.read_flag()?;
        let (max_band_width_minus_4, max_band_step_minus_1) = if banding_in_component_present_flag {
            (Some(reader.read_bits_u8(6)?), Some(reader.read_bits_u8(4)?))
        } else {
            (None, None)
        };
        components.push(BandingComponent {
            banding_in_component_present_flag,
            max_band_width_minus_4,
            max_band_step_minus_1,
        });
    }

    let band_units_information_present_flag = reader.read_flag()?;
    let band_units = if band_units_information_present_flag {
        Some(parse_band_units(reader)?)
    } else {
        None
    };

    Ok(BandingHintsDetail {
        three_color_components_flag,
        components,
        band_units,
    })
}

fn parse_band_units(reader: &mut BitReader<'_>) -> Result<BandUnits> {
    let num_band_units_rows_minus_1 = reader.read_bits_u8(5)?;
    let num_band_units_cols_minus_1 = reader.read_bits_u8(5)?;
    let varying_size_band_units_flag = reader.read_flag()?;

    let varying_size = if varying_size_band_units_flag {
        let band_block_in_luma_samples = reader.read_bits_u8(3)?;
        let mut vert_size_in_band_blocks_minus_1 = Vec::new();
        for _ in 0..=num_band_units_rows_minus_1 {
            vert_size_in_band_blocks_minus_1.push(reader.read_bits_u8(5)?);
        }
        let mut horz_size_in_band_blocks_minus_1 = Vec::new();
        for _ in 0..=num_band_units_cols_minus_1 {
            horz_size_in_band_blocks_minus_1.push(reader.read_bits_u8(5)?);
        }
        Some(VaryingBandUnits {
            band_block_in_luma_samples,
            vert_size_in_band_blocks_minus_1,
            horz_size_in_band_blocks_minus_1,
        })
    } else {
        None
    };

    let mut banding_in_band_unit_present = Vec::new();
    for _ in 0..=num_band_units_rows_minus_1 {
        for _ in 0..=num_band_units_cols_minus_1 {
            banding_in_band_unit_present.push(reader.read_flag()?);
        }
    }

    Ok(BandUnits {
        num_band_units_rows_minus_1,
        num_band_units_cols_minus_1,
        varying_size,
        banding_in_band_unit_present,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::GLOBAL_XLAYER_ID;

    use crate::test_bits::Bits;

    fn short_reader(payload: &[u8]) -> MetadataShortObu {
        let mut reader = BitReader::new(payload, ByteOffset::new(0));
        parse_metadata_short(&mut reader, payload.len()).unwrap()
    }

    /// Builds a `metadata_short_obu()` payload: the 1-byte header, a 1-byte
    /// `metadata_type` (`< 128`), the metadata unit bytes, and one OBU trailing byte.
    fn short_payload(first: u8, metadata_type: u8, unit: &[u8]) -> Vec<u8> {
        let mut payload = vec![first, metadata_type];
        payload.extend_from_slice(unit);
        payload.push(0x80);
        payload
    }

    const SHORT_HEADER: u8 = 0x00; // is_suffix=0, layer_idc=0, cancel=0, persistence=0

    #[test]
    fn short_header_fields_decode() {
        let payload = [0b1010_1011u8, 0x04, 0x80];
        let parsed = short_reader(&payload);
        assert!(parsed.metadata_is_suffix);
        assert_eq!(parsed.muh_layer_idc, 2);
        assert!(parsed.muh_cancel_flag);
        assert_eq!(parsed.muh_persistence_idc, 3);
        assert_eq!(parsed.metadata_type, MetadataType::Timecode);
        assert_eq!(parsed.metadata_type_leb128_bytes, 1);
        assert!(parsed.unit.is_none());
    }

    #[test]
    fn short_hdr_cll_parses() {
        let unit = [0x12, 0x34, 0x56, 0x78];
        let payload = short_payload(SHORT_HEADER, 1, &unit);
        let parsed = short_reader(&payload);
        let unit = parsed.unit.unwrap();
        assert_eq!(unit.payload_size, 4);
        assert_eq!(
            unit.payload,
            MetadataPayload::HdrCll(MetadataHdrCll {
                max_cll: 0x1234,
                max_fall: 0x5678,
            })
        );
    }

    #[test]
    fn short_hdr_mdcv_parses() {
        let mut unit = Vec::new();
        for v in [10u16, 20, 30, 40, 50, 60, 70, 80] {
            unit.extend_from_slice(&v.to_be_bytes());
        }
        unit.extend_from_slice(&1_000_000u32.to_be_bytes());
        unit.extend_from_slice(&5u32.to_be_bytes());
        let payload = short_payload(SHORT_HEADER, 2, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::HdrMdcv(mdcv) = parsed.unit.unwrap().payload else {
            panic!("expected HdrMdcv");
        };
        assert_eq!(mdcv.primary_chromaticity_x, [10, 30, 50]);
        assert_eq!(mdcv.primary_chromaticity_y, [20, 40, 60]);
        assert_eq!(mdcv.white_point_chromaticity_x, 70);
        assert_eq!(mdcv.white_point_chromaticity_y, 80);
        assert_eq!(mdcv.luminance_max, 1_000_000);
        assert_eq!(mdcv.luminance_min, 5);
    }

    #[test]
    fn short_itut_t35_without_extension() {
        let unit = [0x01, 0xAA, 0xBB, 0xCC];
        let payload = short_payload(SHORT_HEADER, 3, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::ItutT35(t35) = parsed.unit.unwrap().payload else {
            panic!("expected ItutT35");
        };
        assert_eq!(t35.itu_t_t35_country_code, 0x01);
        assert_eq!(t35.itu_t_t35_country_code_extension_byte, None);
        assert_eq!(t35.payload_len, 3);
    }

    #[test]
    fn short_itut_t35_with_extension_byte() {
        let unit = [0xFF, 0x42, 0xAA, 0xBB];
        let payload = short_payload(SHORT_HEADER, 3, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::ItutT35(t35) = parsed.unit.unwrap().payload else {
            panic!("expected ItutT35");
        };
        assert_eq!(t35.itu_t_t35_country_code, 0xFF);
        assert_eq!(t35.itu_t_t35_country_code_extension_byte, Some(0x42));
        assert_eq!(t35.payload_len, 2);
    }

    #[test]
    fn short_itut_t35_too_small_is_underflow() {
        let payload = short_payload(SHORT_HEADER, 3, &[]);
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_short(&mut reader, payload.len()),
            Err(Error::InvalidMetadata {
                kind: MetadataErrorKind::UnitPayloadUnderflow,
                ..
            })
        ));
    }

    #[test]
    fn short_timecode_full_timestamp() {
        let mut bits = Bits::default();
        bits.f(0, 5); // counting_type
        bits.bit(1); // full_timestamp_flag
        bits.bit(0); // discontinuity_flag
        bits.bit(0); // cnt_dropped_flag
        bits.f(7, 9); // n_frames
        bits.f(59, 6); // seconds_value
        bits.f(58, 6); // minutes_value
        bits.f(23, 5); // hours_value
        bits.f(0, 5); // time_offset_length == 0
        let unit = bits.into_bytes();
        let payload = short_payload(SHORT_HEADER, 4, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::Timecode(tc) = parsed.unit.unwrap().payload else {
            panic!("expected Timecode");
        };
        assert!(tc.full_timestamp_flag);
        assert_eq!(tc.n_frames, 7);
        assert_eq!(tc.seconds_value, Some(59));
        assert_eq!(tc.minutes_value, Some(58));
        assert_eq!(tc.hours_value, Some(23));
        assert_eq!(tc.time_offset_length, 0);
        assert_eq!(tc.time_offset_value, None);
    }

    #[test]
    fn short_timecode_partial_with_time_offset() {
        let mut bits = Bits::default();
        bits.f(1, 5); // counting_type
        bits.bit(0); // full_timestamp_flag
        bits.bit(0); // discontinuity_flag
        bits.bit(0); // cnt_dropped_flag
        bits.f(0, 9); // n_frames
        bits.bit(1); // seconds_flag
        bits.f(30, 6); // seconds_value
        bits.bit(0); // minutes_flag -> no minutes/hours
        bits.f(4, 5); // time_offset_length = 4
        bits.f(0b1010, 4); // time_offset_value
        let unit = bits.into_bytes();
        let payload = short_payload(SHORT_HEADER, 4, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::Timecode(tc) = parsed.unit.unwrap().payload else {
            panic!("expected Timecode");
        };
        assert!(!tc.full_timestamp_flag);
        assert_eq!(tc.seconds_value, Some(30));
        assert_eq!(tc.minutes_value, None);
        assert_eq!(tc.hours_value, None);
        assert_eq!(tc.time_offset_length, 4);
        assert_eq!(tc.time_offset_value, Some(0b1010));
    }

    #[test]
    fn short_scan_type_parses() {
        let payload = short_payload(SHORT_HEADER, 8, &[0x63]);
        let parsed = short_reader(&payload);
        let MetadataPayload::ScanType(scan) = parsed.unit.unwrap().payload else {
            panic!("expected ScanType");
        };
        assert_eq!(scan.mps_pic_struct_type, 12);
        assert_eq!(scan.mps_source_scan_type_idc, 1);
        assert!(scan.mps_duplicate_flag);
    }

    #[test]
    fn short_temporal_point_info_parses() {
        let payload = short_payload(SHORT_HEADER, 9, &[0xAC, 0x02, 0x00]);
        let parsed = short_reader(&payload);
        let MetadataPayload::TemporalPointInfo(tpi) = parsed.unit.unwrap().payload else {
            panic!("expected TemporalPointInfo");
        };
        assert_eq!(tpi.frame_presentation_time, 300);
    }

    #[test]
    fn short_decoded_frame_hash_per_plane() {
        let mut unit = vec![0b0000_1000u8];
        for plane in 0..3u8 {
            unit.extend_from_slice(&[plane; 16]);
        }
        let payload = short_payload(SHORT_HEADER, 5, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::DecodedFrameHash(hash) = parsed.unit.unwrap().payload else {
            panic!("expected DecodedFrameHash");
        };
        assert!(hash.per_plane);
        assert_eq!(hash.plane_hashes.len(), 3);
        assert_eq!(hash.plane_hashes[1], [1u8; 16]);
        assert_eq!(hash.frame_hash, None);
    }

    #[test]
    fn short_decoded_frame_hash_single() {
        let mut unit = vec![0b0000_0000u8];
        unit.extend_from_slice(&[0xAB; 16]);
        let payload = short_payload(SHORT_HEADER, 5, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::DecodedFrameHash(hash) = parsed.unit.unwrap().payload else {
            panic!("expected DecodedFrameHash");
        };
        assert!(!hash.per_plane);
        assert!(hash.plane_hashes.is_empty());
        assert_eq!(hash.frame_hash, Some([0xAB; 16]));
    }

    #[test]
    fn short_icc_profile_summarizes_length() {
        let payload = short_payload(SHORT_HEADER, 7, &[0u8; 12]);
        let parsed = short_reader(&payload);
        assert_eq!(
            parsed.unit.unwrap().payload,
            MetadataPayload::IccProfile(MetadataIccProfile { payload_len: 12 })
        );
    }

    #[test]
    fn short_user_data_unregistered_summarizes_length() {
        let mut unit = vec![0u8; 16]; // uuid
        unit.extend_from_slice(&[0xCD; 5]); // 5 user data bytes
        let payload = short_payload(SHORT_HEADER, 10, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::UserDataUnregistered(udu) = parsed.unit.unwrap().payload else {
            panic!("expected UserDataUnregistered");
        };
        assert_eq!(udu.uuid_iso_iec_11578, [0u8; 16]);
        assert_eq!(udu.payload_len, 5);
    }

    #[test]
    fn short_user_data_too_small_for_uuid_is_underflow() {
        let payload = short_payload(SHORT_HEADER, 10, &[0u8; 8]);
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_short(&mut reader, payload.len()),
            Err(Error::InvalidMetadata {
                kind: MetadataErrorKind::UnitPayloadUnderflow,
                ..
            })
        ));
    }

    #[test]
    fn short_banding_hints_basic() {
        let mut bits = Bits::default();
        bits.bit(1); // coding_banding_present_flag
        bits.bit(0); // source_banding_present_flag
        bits.bit(0); // banding_hints_flag = 0 -> no detail
        let unit = bits.into_bytes();
        let payload = short_payload(SHORT_HEADER, 6, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::BandingHints(banding) = parsed.unit.unwrap().payload else {
            panic!("expected BandingHints");
        };
        assert!(banding.coding_banding_present_flag);
        assert!(!banding.source_banding_present_flag);
        assert!(banding.hints.is_none());
    }

    #[test]
    fn short_banding_hints_with_band_units() {
        let mut bits = Bits::default();
        bits.bit(1); // coding_banding_present_flag
        bits.bit(0); // source_banding_present_flag
        bits.bit(1); // banding_hints_flag
        bits.bit(0); // three_color_components_flag = 0 -> 1 component
        bits.bit(1); // banding_in_component_present_flag
        bits.f(5, 6); // max_band_width_minus_4
        bits.f(2, 4); // max_band_step_minus_1
        bits.bit(1); // band_units_information_present_flag
        bits.f(1, 5); // num_band_units_rows_minus_1 = 1 -> 2 rows
        bits.f(0, 5); // num_band_units_cols_minus_1 = 0 -> 1 col
        bits.bit(0); // varying_size_band_units_flag = 0
        bits.bit(1); // banding_in_band_unit_present[0][0]
        bits.bit(0); // banding_in_band_unit_present[1][0]
        let unit = bits.into_bytes();
        let payload = short_payload(SHORT_HEADER, 6, &unit);
        let parsed = short_reader(&payload);
        let MetadataPayload::BandingHints(banding) = parsed.unit.unwrap().payload else {
            panic!("expected BandingHints");
        };
        let detail = banding.hints.unwrap();
        assert_eq!(detail.components.len(), 1);
        assert_eq!(detail.components[0].max_band_width_minus_4, Some(5));
        let band_units = detail.band_units.unwrap();
        assert_eq!(band_units.num_band_units_rows_minus_1, 1);
        assert_eq!(band_units.num_band_units_cols_minus_1, 0);
        assert_eq!(band_units.banding_in_band_unit_present, vec![true, false]);
    }

    #[test]
    fn short_unknown_metadata_type_is_raw() {
        let payload = short_payload(SHORT_HEADER, 0, &[0xDE, 0xAD, 0xBE]);
        let parsed = short_reader(&payload);
        assert_eq!(parsed.metadata_type, MetadataType::Reserved(0));
        assert_eq!(
            parsed.unit.unwrap().payload,
            MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 3 })
        );
    }

    #[test]
    fn short_high_reserved_metadata_type_is_raw() {
        let payload = short_payload(SHORT_HEADER, 100, &[0x00, 0x00]);
        let parsed = short_reader(&payload);
        assert_eq!(parsed.metadata_type, MetadataType::Reserved(100));
        assert!(matches!(
            parsed.unit.unwrap().payload,
            MetadataPayload::UnknownRaw(_)
        ));
    }

    #[test]
    fn short_payload_underflow_when_obu_too_small() {
        let payload = [SHORT_HEADER, 0x01];
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_short(&mut reader, payload.len()),
            Err(Error::InvalidMetadata {
                kind: MetadataErrorKind::UnitPayloadUnderflow,
                ..
            })
        ));
    }

    #[test]
    fn short_truncated_header_is_eof() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_short(&mut reader, 0),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    fn group_reader(payload: &[u8], xlayer: ExtendedLayerId) -> MetadataGroupObu {
        let mut reader = BitReader::new(payload, ByteOffset::new(0));
        parse_metadata_group(&mut reader, xlayer).unwrap()
    }

    #[test]
    fn group_single_cancelled_unit() {
        let payload = [0x00, 0x00, 0x04, 0x01, 0x80];
        let group = group_reader(&payload, ExtendedLayerId::from_bits(0));
        assert_eq!(group.units.len(), 1);
        let unit = &group.units[0];
        assert!(unit.muh_cancel_flag);
        assert_eq!(unit.metadata_type, MetadataType::Timecode);
        assert!(unit.unit.is_none());
    }

    #[test]
    fn group_single_hdr_cll_unit() {
        let payload = [
            0x00, // is_suffix=0, necessity=0, app_id=0
            0x00, // metadata_unit_cnt_minus_1 = 0
            0x01, // metadata_type = HdrCll
            0x06, // muh_header_size=3, cancel=0
            0x04, // muh_payload_size = 4
            0x00, 0x00, // layer_idc=0, persistence=0, priority=0, reserved=0
            0x12, 0x34, 0x56, 0x78, // hdr_cll
            0x80, // OBU trailing byte
        ];
        let group = group_reader(&payload, ExtendedLayerId::from_bits(0));
        assert_eq!(group.units.len(), 1);
        let unit = &group.units[0];
        assert!(!unit.muh_cancel_flag);
        assert_eq!(unit.muh_payload_size, Some(4));
        assert_eq!(unit.muh_reserved_zero_2bits, Some(0));
        assert_eq!(unit.header_extension_len, 0);
        let metadata_unit = unit.unit.as_ref().unwrap();
        assert_eq!(
            metadata_unit.payload,
            MetadataPayload::HdrCll(MetadataHdrCll {
                max_cll: 0x1234,
                max_fall: 0x5678,
            })
        );
    }

    #[test]
    fn group_local_mlayer_map_is_parsed() {
        let mut middle = Bits::default();
        middle.f(3, 3); // muh_layer_idc = LAYER_VALUES
        middle.f(0, 3); // muh_persistence_idc
        middle.f(0, 8); // muh_priority
        middle.f(0, 2); // muh_reserved_zero_2bits
        let middle = middle.into_bytes(); // 2 bytes
        let mut payload = vec![0x00, 0x00, 0x00, 0x08, 0x00];
        payload.extend_from_slice(&middle);
        payload.push(0b0000_0110); // muh_mlayer_map (bits 1 and 2 set)
        payload.push(0x80); // OBU trailing byte
        let group = group_reader(&payload, ExtendedLayerId::from_bits(2));
        let unit = &group.units[0];
        assert_eq!(unit.muh_layer_idc, Some(LAYER_VALUES));
        assert_eq!(unit.muh_xlayer_map, None);
        assert_eq!(unit.muh_mlayer_maps, vec![0b0000_0110]);
    }

    #[test]
    fn group_global_xlayer_map_is_parsed() {
        let mut middle = Bits::default();
        middle.f(3, 3); // muh_layer_idc = LAYER_VALUES
        middle.f(0, 3);
        middle.f(0, 8);
        middle.f(0, 2);
        let middle = middle.into_bytes();
        let mut payload = vec![0x00, 0x00, 0x00, 0x10, 0x00];
        payload.extend_from_slice(&middle);
        payload.extend_from_slice(&1u32.to_be_bytes()); // muh_xlayer_map = bit 0 set
        payload.push(0xAA); // one muh_mlayer_map
        payload.push(0x80);
        let group = group_reader(&payload, GLOBAL_XLAYER_ID);
        let unit = &group.units[0];
        assert_eq!(unit.muh_xlayer_map, Some(1));
        assert_eq!(unit.muh_mlayer_maps, vec![0xAA]);
    }

    #[test]
    fn group_two_units() {
        let payload = [0x00, 0x01, 0x04, 0x01, 0x05, 0x01, 0x80];
        let group = group_reader(&payload, ExtendedLayerId::from_bits(0));
        assert_eq!(group.units.len(), 2);
        assert_eq!(group.units[0].metadata_type, MetadataType::Timecode);
        assert_eq!(group.units[1].metadata_type, MetadataType::DecodedFrameHash);
    }

    #[test]
    fn group_unit_count_too_large_is_rejected() {
        let payload = [0x00, 0xFF, 0x7F, 0x80];
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_group(&mut reader, ExtendedLayerId::from_bits(0),),
            Err(Error::InvalidMetadata {
                kind: MetadataErrorKind::GroupUnitCountTooLarge,
                ..
            })
        ));
    }

    #[test]
    fn group_header_underflow_is_rejected() {
        let payload = [0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x80];
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_group(&mut reader, ExtendedLayerId::from_bits(0),),
            Err(Error::InvalidMetadata {
                kind: MetadataErrorKind::GroupHeaderUnderflow,
                ..
            })
        ));
    }

    #[test]
    fn group_header_extension_bytes_are_consumed() {
        let payload = [
            0x00, 0x00, // group header + cnt
            0x00, // metadata_type = 0 (Reserved -> UnknownRaw, no unit bytes)
            0x08, // muh_header_size = 4, cancel = 0
            0x00, // muh_payload_size = 0
            0x00, 0x00, // fixed 16 bits (layer_idc=0)
            0xEE, // 1 muh_header_extension_byte
            0x80, // OBU trailing byte
        ];
        let group = group_reader(&payload, ExtendedLayerId::from_bits(0));
        let unit = &group.units[0];
        assert_eq!(unit.header_extension_len, 1);
        assert_eq!(unit.muh_payload_size, Some(0));
    }

    #[test]
    fn group_truncated_is_eof() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_metadata_group(&mut reader, ExtendedLayerId::from_bits(0),),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn metadata_type_round_trips() {
        for value in 0u32..=12 {
            assert_eq!(MetadataType::from_value(value).value(), value);
        }
        assert_eq!(MetadataType::from_value(1000).value(), 1000);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::GLOBAL_XLAYER_ID;
    use proptest::prelude::*;

    proptest! {
        /// The metadata parsers must never panic on arbitrary input.
        #[test]
        fn metadata_parsers_never_panic(data in proptest::collection::vec(any::<u8>(), 0..256)) {
            let mut short = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_metadata_short(&mut short, data.len());

            let mut group = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_metadata_group(
                &mut group,
                ExtendedLayerId::from_bits(0),
            );

            let mut global_group = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_metadata_group(
                &mut global_group,
                GLOBAL_XLAYER_ID,
            );
        }
    }
}
