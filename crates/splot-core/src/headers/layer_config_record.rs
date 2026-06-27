// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 layer configuration record OBU syntax model (AV2 v1.0.0 § 5.8).
//!
//! `layer_config_record_obu()` branches on `obu_xlayer_id`: the global scope
//! (`obu_xlayer_id == GLOBAL_XLAYER_ID`) parses `lcr_global_info()` (§ 5.8.1) and the
//! local scope parses `lcr_local_info(obu_xlayer_id)` (§ 5.8.2). This module parses
//! the full § 5.8 syntax — including the nested `lcr_xlayer_info()`,
//! `lcr_embedded_layer_info()`, `lcr_rep_info()`, and `lcr_xlayer_color_info()`
//! structures and the length-bounded `lcr_global_payload()` — and never skips payload
//! bits. Reserved-zero fields are retained so `splot-validate` can surface a non-zero
//! value (AV2 § 6.8); cross-OBU availability (AV2 § 7.3.8.3) is checked there too.

use crate::bitio::BitReader;
use crate::error::{Error, LayerConfigRecordErrorKind, Result};
use crate::headers::sequence::ProfileIdc;
use crate::types::ExtendedLayerId;

/// `MAX_NUM_TLAYERS` (AV2 § 3): the bit width of `lcr_tlayer_map` (`f(n)`,
/// `n == MAX_NUM_TLAYERS`) in `lcr_embedded_layer_info()` (AV2 § 5.8.8).
const MAX_NUM_TLAYERS: u32 = 4;
/// `AUX_LAYER` (AV2 § 3 / § 6.8.9): `lcr_layer_type` value that codes an auxiliary
/// type field.
const AUX_LAYER: u8 = 1;
/// `VIEW_EXPLICIT` (AV2 § 6.8.9, Table for `lcr_view_type`): the value that codes an
/// explicit `lcr_view_id`.
const VIEW_EXPLICIT: u8 = 4;

/// Parsed `layer_config_record_obu()` syntax (AV2 v1.0.0 § 5.8): a global record
/// (`obu_xlayer_id == GLOBAL_XLAYER_ID`) or a per-extended-layer local record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerConfigurationRecord {
    /// `lcr_global_info()` (AV2 § 5.8.1), parsed when `obu_xlayer_id == GLOBAL_XLAYER_ID`.
    Global(LcrGlobalInfo),
    /// `lcr_local_info(obu_xlayer_id)` (AV2 § 5.8.2), parsed otherwise.
    Local(LcrLocalInfo),
}

impl LayerConfigurationRecord {
    /// Returns `true` if any reserved-zero field of the record carries a non-zero
    /// value (AV2 § 6.8: these `shall be equal to 0`, and decoders ignore the value).
    #[must_use]
    pub fn has_nonzero_reserved_bits(&self) -> bool {
        match self {
            Self::Global(info) => info.has_nonzero_reserved_bits(),
            Self::Local(info) => info.has_nonzero_reserved_bits(),
        }
    }
}

/// `lcr_global_info()` (AV2 v1.0.0 § 5.8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrGlobalInfo {
    /// `lcr_global_config_record_id` (`f(3)`); the global LCR id (conformance: 1..7).
    pub global_config_record_id: u8,
    /// `lcr_xlayer_map` (`f(31)`): bitmap of extended layers associated with this
    /// global LCR.
    pub xlayer_map: u32,
    /// `lcr_aggregate_info_present_flag`.
    pub aggregate_info_present: bool,
    /// `lcr_seq_profile_tier_level_info_present_flag`.
    pub seq_ptl_info_present: bool,
    /// `lcr_global_payload_present_flag`.
    pub global_payload_present: bool,
    /// `lcr_dependent_xlayers_flag` (conformance: must be 0).
    pub dependent_xlayers_flag: bool,
    /// `lcr_global_atlas_id_present_flag`.
    pub global_atlas_id_present: bool,
    /// `lcr_global_purpose_id` (`f(7)`).
    pub global_purpose_id: u8,
    /// `lcr_doh_constraint_flag`.
    pub doh_constraint_flag: bool,
    /// `lcr_enforce_tile_alignment_flag`.
    pub enforce_tile_alignment_flag: bool,
    /// `lcr_global_atlas_id` (`f(3)`), present only when
    /// [`Self::global_atlas_id_present`].
    pub global_atlas_id: Option<u8>,
    /// `lcr_global_reserved_zero_3bits` (`f(3)`); `0` when an atlas id was coded
    /// instead.
    pub reserved_zero_3bits: u8,
    /// `lcr_global_reserved_zero_5bits` (`f(5)`).
    pub reserved_zero_5bits: u8,
    /// `lcr_aggregate_info()` (AV2 § 5.8.3), present when
    /// [`Self::aggregate_info_present`].
    pub aggregate_info: Option<LcrAggregateInfo>,
    /// `lcr_seq_profile_tier_level_info()` per xlayer in [`Self::xlayer_map`], present
    /// when [`Self::seq_ptl_info_present`].
    pub seq_ptl_infos: Vec<LcrSeqProfileTierLevelInfo>,
    /// `lcr_global_payload()` per xlayer in [`Self::xlayer_map`], present when
    /// [`Self::global_payload_present`].
    pub payloads: Vec<LcrGlobalPayload>,
}

impl LcrGlobalInfo {
    /// Returns `true` if any reserved-zero field is non-zero (AV2 § 6.8.2 / § 6.8.5).
    #[must_use]
    pub fn has_nonzero_reserved_bits(&self) -> bool {
        self.reserved_zero_3bits != 0
            || self.reserved_zero_5bits != 0
            || self.seq_ptl_infos.iter().any(|p| p.reserved_2bits != 0)
    }
}

/// `lcr_local_info(xlayerId)` (AV2 v1.0.0 § 5.8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrLocalInfo {
    /// `obu_xlayer_id` of the OBU carrying this local record.
    pub xlayer_id: u8,
    /// `lcr_global_id[xlayerId]` (`f(3)`): the global LCR id this local record refers
    /// to, or `0` for none.
    pub global_id: u8,
    /// `lcr_local_id[xlayerId]` (`f(3)`): the local LCR id (conformance: non-zero).
    pub local_id: u8,
    /// `lcr_profile_tier_level_info_present_flag[xlayerId]`.
    pub profile_tier_level_info_present: bool,
    /// `lcr_local_atlas_id_present_flag[xlayerId]`.
    pub local_atlas_id_present: bool,
    /// `lcr_seq_profile_tier_level_info(xlayerId)` (AV2 § 5.8.4), present when
    /// [`Self::profile_tier_level_info_present`].
    pub seq_ptl_info: Option<LcrSeqProfileTierLevelInfo>,
    /// `lcr_local_atlas_id[xlayerId]` (`f(3)`), present when
    /// [`Self::local_atlas_id_present`].
    pub local_atlas_id: Option<u8>,
    /// `lcr_local_reserved_zero_3bits[xlayerId]` (`f(3)`); `0` when an atlas id was
    /// coded instead.
    pub reserved_zero_3bits: u8,
    /// `lcr_local_reserved_zero_5bits[xlayerId]` (`f(5)`).
    pub reserved_zero_5bits: u8,
    /// `lcr_xlayer_info(0, xlayerId)` (AV2 § 5.8.6).
    pub xlayer_info: LcrXlayerInfo,
}

impl LcrLocalInfo {
    /// Returns `true` if any reserved-zero field is non-zero (AV2 § 6.8.3 / § 6.8.5).
    #[must_use]
    pub fn has_nonzero_reserved_bits(&self) -> bool {
        self.reserved_zero_3bits != 0
            || self.reserved_zero_5bits != 0
            || self
                .seq_ptl_info
                .as_ref()
                .is_some_and(|p| p.reserved_2bits != 0)
    }
}

/// `lcr_aggregate_info()` (AV2 v1.0.0 § 5.8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcrAggregateInfo {
    /// `lcr_config_idc` (`f(6)`).
    pub config_idc: u8,
    /// `lcr_aggregate_level_idx` (`f(5)`).
    pub aggregate_level_idx: u8,
    /// `lcr_max_tier_flag` (`f(1)`).
    pub max_tier_flag: bool,
    /// `lcr_max_interop` (`f(4)`).
    pub max_interop: u8,
}

/// `lcr_seq_profile_tier_level_info(i)` (AV2 v1.0.0 § 5.8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcrSeqProfileTierLevelInfo {
    /// The `i` / `xId` argument: which xlayer this PTL describes.
    pub xlayer_id: u8,
    /// `lcr_seq_profile_idc[i]` (`f(5)`; Annex A Table A.1 value space).
    pub seq_profile_idc: ProfileIdc,
    /// `lcr_max_level_idx[i]` (`f(5)`).
    pub max_level_idx: u8,
    /// `lcr_tier_flag[i]` (`f(1)`).
    pub tier_flag: bool,
    /// `lcr_max_mlayer_count[i]` (`f(3)`).
    pub max_mlayer_count: u8,
    /// `lsptli_reserved_2bits` (`f(2)`; conformance: must be 0).
    pub reserved_2bits: u8,
}

/// `lcr_global_payload(n, sz)` (AV2 v1.0.0 § 5.8.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrGlobalPayload {
    /// `n`: the xlayer id this payload describes.
    pub xlayer_id: u8,
    /// `sz`: `lcr_data_size[i]` (`leb128()`), the payload size in bytes.
    pub data_size: u32,
    /// `lcr_num_dependent_xlayer_map[n]` (`f(n)`), present only when
    /// `lcr_dependent_xlayers_flag && n > 0`.
    pub num_dependent_xlayer_map: Option<u32>,
    /// `lcr_xlayer_info(1, n)` (AV2 § 5.8.6).
    pub xlayer_info: LcrXlayerInfo,
    /// Number of trailing `lcr_remaining_payload_bit` bits consumed to fill `sz * 8`.
    pub remaining_payload_bits: u64,
}

/// `lcr_xlayer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrXlayerInfo {
    /// `lcr_rep_info(isGlobal, xId)` (AV2 § 5.8.7), present when its flag is set.
    pub rep_info: Option<LcrRepInfo>,
    /// `lcr_xlayer_purpose_id[isGlobal][xId]` (`f(7)`), present when its flag is set.
    pub purpose_id: Option<u8>,
    /// `lcr_xlayer_color_info(isGlobal, xId)` (AV2 § 5.8.9), present when its flag is
    /// set.
    pub color_info: Option<LcrXlayerColorInfo>,
    /// `lcr_embedded_layer_info(isGlobal, xId)` (AV2 § 5.8.8), present when its flag is
    /// set.
    pub embedded_layer_info: Option<LcrEmbeddedLayerInfo>,
    /// The else-branch atlas reference, present only when
    /// `isGlobal && lcr_global_atlas_id_present_flag` and no embedded-layer info.
    pub xlayer_atlas: Option<LcrXlayerAtlasRef>,
}

/// `lcr_rep_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrRepInfo {
    /// `lcr_max_pic_width[isGlobal][xId]` (`uvlc()`).
    pub max_pic_width: u32,
    /// `lcr_max_pic_height[isGlobal][xId]` (`uvlc()`).
    pub max_pic_height: u32,
    /// `lcr_bit_depth_idc` / `lcr_chroma_format_idc`, present when
    /// `lcr_format_info_present_flag`.
    pub format_info: Option<LcrFormatInfo>,
    /// `lcr_cropping_win_*_offset`, present when `lcr_cropping_window_present_flag`.
    pub cropping_window: Option<LcrCroppingWindow>,
}

/// `lcr_format_info` fields of `lcr_rep_info()` (AV2 v1.0.0 § 5.8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcrFormatInfo {
    /// `lcr_bit_depth_idc[isGlobal][xId]` (`uvlc()`).
    pub bit_depth_idc: u32,
    /// `lcr_chroma_format_idc[isGlobal][xId]` (`uvlc()`).
    pub chroma_format_idc: u32,
}

/// `lcr_cropping_window` fields of `lcr_rep_info()` (AV2 v1.0.0 § 5.8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcrCroppingWindow {
    /// `lcr_cropping_win_left_offset` (`uvlc()`).
    pub left_offset: u32,
    /// `lcr_cropping_win_right_offset` (`uvlc()`).
    pub right_offset: u32,
    /// `lcr_cropping_win_top_offset` (`uvlc()`).
    pub top_offset: u32,
    /// `lcr_cropping_win_bottom_offset` (`uvlc()`).
    pub bottom_offset: u32,
}

/// `lcr_xlayer_color_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrXlayerColorInfo {
    /// `layer_color_description_idc[isGlobal][xId]` (`rg(2)`).
    pub color_description_idc: u32,
    /// `(layer_color_primaries, layer_transfer_characteristics,
    /// layer_matrix_coefficients)` (`f(8)` each), present only when
    /// `layer_color_description_idc == 0`.
    pub primaries: Option<(u8, u8, u8)>,
    /// `layer_full_range_flag` (`f(1)`).
    pub full_range_flag: bool,
}

/// The else-branch atlas reference of `lcr_xlayer_info()` (AV2 v1.0.0 § 5.8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcrXlayerAtlasRef {
    /// `lcr_xlayer_atlas_segment_id[xId]` (`f(8)`).
    pub atlas_segment_id: u8,
    /// `lcr_xlayer_priority_order[xId]` (`f(8)`).
    pub priority_order: u8,
    /// `lcr_xlayer_rendering_method[xId]` (`f(8)`).
    pub rendering_method: u8,
}

/// `lcr_embedded_layer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrEmbeddedLayerInfo {
    /// `lcr_mlayer_map[isGlobal][xId]` (`f(8)`): bitmap selecting the embedded layers.
    pub mlayer_map: u8,
    /// One [`LcrEmbeddedLayer`] per set bit of [`Self::mlayer_map`].
    pub layers: Vec<LcrEmbeddedLayer>,
}

/// One embedded layer (a set bit `j` of `lcr_mlayer_map`) of
/// `lcr_embedded_layer_info()` (AV2 v1.0.0 § 5.8.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LcrEmbeddedLayer {
    /// The bit index `j` (the `obu_mlayer_id`) this layer describes.
    pub mlayer_index: u8,
    /// `lcr_tlayer_map[isGlobal][xId][j]` (`f(MAX_NUM_TLAYERS)`).
    pub tlayer_map: u8,
    /// `lcr_layer_atlas_segment_id[isGlobal][xId][j]` (`f(8)`), present when an atlas
    /// is associated.
    pub atlas_segment_id: Option<u8>,
    /// `lcr_priority_order[isGlobal][xId][j]` (`f(8)`), present with the atlas id.
    pub priority_order: Option<u8>,
    /// `lcr_rendering_method[isGlobal][xId][j]` (`f(8)`), present with the atlas id.
    pub rendering_method: Option<u8>,
    /// `lcr_layer_type[isGlobal][xId][j]` (`f(8)`).
    pub layer_type: u8,
    /// `lcr_auxiliary_type[isGlobal][xId][j]` (`f(8)`), present when
    /// `lcr_layer_type == AUX_LAYER`.
    pub auxiliary_type: Option<u8>,
    /// `lcr_view_type[isGlobal][xId][j]` (`f(8)`).
    pub view_type: u8,
    /// `lcr_view_id[isGlobal][xId][j]` (`f(8)`), present when
    /// `lcr_view_type == VIEW_EXPLICIT`.
    pub view_id: Option<u8>,
    /// `lcr_dependent_layer_map[isGlobal][xId][j]` (`f(j)`), present when `j > 0`.
    pub dependent_layer_map: Option<u32>,
    /// `lcr_same_sh_max_resolution_flag[isGlobal][xId][j]` (`f(1)`).
    pub same_sh_max_resolution_flag: bool,
    /// `lcr_max_expected_width[isGlobal][xId][j]` (`uvlc()`), present when the flag is
    /// clear.
    pub max_expected_width: Option<u32>,
    /// `lcr_max_expected_height[isGlobal][xId][j]` (`uvlc()`), present when the flag is
    /// clear.
    pub max_expected_height: Option<u32>,
}

/// Shared context threaded into `lcr_xlayer_info()` parsing: the atlas-presence flags
/// that select the else-branch atlas reference (AV2 § 5.8.6) and per-embedded-layer
/// atlas fields (AV2 § 5.8.8).
struct XlayerAtlasContext {
    /// `isGlobal` argument: `true` for `lcr_global_payload()`, `false` for
    /// `lcr_local_info()`.
    is_global: bool,
    /// `lcr_global_atlas_id_present_flag` from `lcr_global_info()`.
    global_atlas_id_present: bool,
    /// `lcr_local_atlas_id_present_flag[xId]` from `lcr_local_info()`.
    local_atlas_id_present: bool,
}

impl XlayerAtlasContext {
    /// `atlasSegmentPresent` per AV2 § 5.8.8: the global flag for a global record,
    /// otherwise the per-xlayer local flag.
    const fn atlas_segment_present(&self) -> bool {
        if self.is_global {
            self.global_atlas_id_present
        } else {
            self.local_atlas_id_present
        }
    }
}

/// Parses `layer_config_record_obu()` (AV2 v1.0.0 § 5.8) for an OBU whose header
/// carries `xlayer_id`.
///
/// The full § 5.8 syntax is read; the parser never skips payload bits. Reserved-zero
/// fields are retained rather than rejected, so `splot-validate` can report a non-zero
/// value (AV2 § 6.8).
///
/// # Errors
/// Returns descriptor errors (`uvlc`/`rg`/`leb128`), [`Error::InvalidByteAlignment`]
/// for non-zero alignment bits, [`Error::InvalidLayerConfigRecord`] if a global
/// payload's parsed content exceeds its declared size, or [`Error::UnexpectedEof`] if
/// the payload ends mid-field.
pub fn parse_layer_config_record(
    reader: &mut BitReader<'_>,
    xlayer_id: ExtendedLayerId,
) -> Result<LayerConfigurationRecord> {
    if xlayer_id.is_global() {
        Ok(LayerConfigurationRecord::Global(parse_lcr_global_info(
            reader,
        )?))
    } else {
        Ok(LayerConfigurationRecord::Local(parse_lcr_local_info(
            reader, xlayer_id,
        )?))
    }
}

/// Parses `lcr_global_info()` (AV2 v1.0.0 § 5.8.1).
fn parse_lcr_global_info(reader: &mut BitReader<'_>) -> Result<LcrGlobalInfo> {
    let global_config_record_id = reader.read_bits_u8(3)?;
    let xlayer_map = reader.read_bits(31)?;

    // AV2 § 5.8.1: derive LcrXLayerID[] / LcrMaxNumXLayerCount from the set bits of
    // lcr_xlayer_map; these drive the PTL and payload loops below.
    let xlayer_ids = derive_xlayer_ids(xlayer_map);

    let aggregate_info_present = reader.read_flag()?;
    let seq_ptl_info_present = reader.read_flag()?;
    let global_payload_present = reader.read_flag()?;
    let dependent_xlayers_flag = reader.read_flag()?;
    let global_atlas_id_present = reader.read_flag()?;
    let global_purpose_id = reader.read_bits_u8(7)?;
    let doh_constraint_flag = reader.read_flag()?;
    let enforce_tile_alignment_flag = reader.read_flag()?;

    let (global_atlas_id, reserved_zero_3bits) = if global_atlas_id_present {
        (Some(reader.read_bits_u8(3)?), 0)
    } else {
        (None, reader.read_bits_u8(3)?)
    };
    let reserved_zero_5bits = reader.read_bits_u8(5)?;

    let aggregate_info = if aggregate_info_present {
        Some(parse_lcr_aggregate_info(reader)?)
    } else {
        None
    };

    let mut seq_ptl_infos = Vec::new();
    if seq_ptl_info_present {
        for &xlayer in &xlayer_ids {
            seq_ptl_infos.push(parse_lcr_seq_profile_tier_level_info(reader, xlayer)?);
        }
    }

    let mut payloads = Vec::new();
    if global_payload_present {
        for &xlayer in &xlayer_ids {
            let data_size = reader.read_leb128()?;
            payloads.push(parse_lcr_global_payload(
                reader,
                xlayer,
                data_size,
                dependent_xlayers_flag,
                global_atlas_id_present,
            )?);
        }
    }

    Ok(LcrGlobalInfo {
        global_config_record_id,
        xlayer_map,
        aggregate_info_present,
        seq_ptl_info_present,
        global_payload_present,
        dependent_xlayers_flag,
        global_atlas_id_present,
        global_purpose_id,
        doh_constraint_flag,
        enforce_tile_alignment_flag,
        global_atlas_id,
        reserved_zero_3bits,
        reserved_zero_5bits,
        aggregate_info,
        seq_ptl_infos,
        payloads,
    })
}

/// Derives `LcrXLayerID[]` from `lcr_xlayer_map` (AV2 § 5.8.1): the bit indices set in
/// the 31-bit map, in ascending order.
fn derive_xlayer_ids(xlayer_map: u32) -> Vec<u8> {
    let mut ids = Vec::new();
    for i in 0u8..31 {
        if xlayer_map & (1u32 << u32::from(i)) != 0 {
            ids.push(i);
        }
    }
    ids
}

/// Parses `lcr_local_info(xlayerId)` (AV2 v1.0.0 § 5.8.2).
fn parse_lcr_local_info(
    reader: &mut BitReader<'_>,
    xlayer_id: ExtendedLayerId,
) -> Result<LcrLocalInfo> {
    let global_id = reader.read_bits_u8(3)?;
    let local_id = reader.read_bits_u8(3)?;
    let profile_tier_level_info_present = reader.read_flag()?;
    let local_atlas_id_present = reader.read_flag()?;

    let seq_ptl_info = if profile_tier_level_info_present {
        Some(parse_lcr_seq_profile_tier_level_info(
            reader,
            xlayer_id.get(),
        )?)
    } else {
        None
    };

    let (local_atlas_id, reserved_zero_3bits) = if local_atlas_id_present {
        (Some(reader.read_bits_u8(3)?), 0)
    } else {
        (None, reader.read_bits_u8(3)?)
    };
    let reserved_zero_5bits = reader.read_bits_u8(5)?;

    let context = XlayerAtlasContext {
        is_global: false,
        global_atlas_id_present: false,
        local_atlas_id_present,
    };
    let xlayer_info = parse_lcr_xlayer_info(reader, &context)?;

    Ok(LcrLocalInfo {
        xlayer_id: xlayer_id.get(),
        global_id,
        local_id,
        profile_tier_level_info_present,
        local_atlas_id_present,
        seq_ptl_info,
        local_atlas_id,
        reserved_zero_3bits,
        reserved_zero_5bits,
        xlayer_info,
    })
}

/// Parses `lcr_aggregate_info()` (AV2 v1.0.0 § 5.8.3).
fn parse_lcr_aggregate_info(reader: &mut BitReader<'_>) -> Result<LcrAggregateInfo> {
    Ok(LcrAggregateInfo {
        config_idc: reader.read_bits_u8(6)?,
        aggregate_level_idx: reader.read_bits_u8(5)?,
        max_tier_flag: reader.read_flag()?,
        max_interop: reader.read_bits_u8(4)?,
    })
}

/// Parses `lcr_seq_profile_tier_level_info(i)` (AV2 v1.0.0 § 5.8.4).
fn parse_lcr_seq_profile_tier_level_info(
    reader: &mut BitReader<'_>,
    xlayer_id: u8,
) -> Result<LcrSeqProfileTierLevelInfo> {
    Ok(LcrSeqProfileTierLevelInfo {
        xlayer_id,
        seq_profile_idc: ProfileIdc::from_bits(reader.read_bits_u8(5)?),
        max_level_idx: reader.read_bits_u8(5)?,
        tier_flag: reader.read_flag()?,
        max_mlayer_count: reader.read_bits_u8(3)?,
        reserved_2bits: reader.read_bits_u8(2)?,
    })
}

/// Parses `lcr_global_payload(n, sz)` (AV2 v1.0.0 § 5.8.5).
///
/// The payload is exactly `sz * 8` bits long. After the optional dependent-xlayer map
/// and the embedded `lcr_xlayer_info(1, n)`, the remaining bits up to `sz * 8` are
/// consumed as reserved `lcr_remaining_payload_bit` bits.
fn parse_lcr_global_payload(
    reader: &mut BitReader<'_>,
    xlayer_id: u8,
    data_size: u32,
    dependent_xlayers_flag: bool,
    global_atlas_id_present: bool,
) -> Result<LcrGlobalPayload> {
    let start_bits = reader.consumed_bits();

    let num_dependent_xlayer_map = if dependent_xlayers_flag && xlayer_id > 0 {
        Some(reader.read_bits(u32::from(xlayer_id))?)
    } else {
        None
    };

    let context = XlayerAtlasContext {
        is_global: true,
        global_atlas_id_present,
        local_atlas_id_present: false,
    };
    let xlayer_info = parse_lcr_xlayer_info(reader, &context)?;

    let parsed_bits = reader.consumed_bits().saturating_sub(start_bits);
    let total_bits = u64::from(data_size).saturating_mul(8);
    if parsed_bits > total_bits {
        // AV2 § 5.8.5 / § 6.8.6: RemainingLcrPayloadBits would be negative.
        return Err(Error::InvalidLayerConfigRecord {
            offset: reader.byte_offset(),
            bit_offset: reader.bit_offset(),
            kind: LayerConfigRecordErrorKind::PayloadSizeOverflow,
        });
    }

    let remaining_payload_bits = total_bits - parsed_bits;
    if remaining_payload_bits > reader.remaining_bits() {
        let deficit = remaining_payload_bits - reader.remaining_bits();
        return Err(Error::UnexpectedEof {
            offset: reader.byte_offset(),
            needed: usize::try_from(deficit.div_ceil(8)).unwrap_or(usize::MAX),
        });
    }

    // Consume the reserved lcr_remaining_payload_bit bits (value ignored) in 32-bit
    // chunks; the count is bounded by the declared, already-validated payload size.
    let mut left = remaining_payload_bits;
    while left >= 32 {
        let _ = reader.read_bits(32)?;
        left -= 32;
    }
    if left > 0 {
        let _ = reader.read_bits(u32::try_from(left).unwrap_or(0))?;
    }

    Ok(LcrGlobalPayload {
        xlayer_id,
        data_size,
        num_dependent_xlayer_map,
        xlayer_info,
        remaining_payload_bits,
    })
}

/// Parses `lcr_xlayer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.6).
fn parse_lcr_xlayer_info(
    reader: &mut BitReader<'_>,
    context: &XlayerAtlasContext,
) -> Result<LcrXlayerInfo> {
    let rep_info_present = reader.read_flag()?;
    let xlayer_purpose_present = reader.read_flag()?;
    let xlayer_color_info_present = reader.read_flag()?;
    let embedded_layer_info_present = reader.read_flag()?;

    let rep_info = if rep_info_present {
        Some(parse_lcr_rep_info(reader)?)
    } else {
        None
    };
    let purpose_id = if xlayer_purpose_present {
        Some(reader.read_bits_u8(7)?)
    } else {
        None
    };
    let color_info = if xlayer_color_info_present {
        Some(parse_lcr_xlayer_color_info(reader)?)
    } else {
        None
    };

    // AV2 § 5.8.6: byte_alignment() before the embedded-layer / atlas block.
    reader.byte_align_zero()?;

    let (embedded_layer_info, xlayer_atlas) = if embedded_layer_info_present {
        (Some(parse_lcr_embedded_layer_info(reader, context)?), None)
    } else if context.is_global && context.global_atlas_id_present {
        let atlas = LcrXlayerAtlasRef {
            atlas_segment_id: reader.read_bits_u8(8)?,
            priority_order: reader.read_bits_u8(8)?,
            rendering_method: reader.read_bits_u8(8)?,
        };
        (None, Some(atlas))
    } else {
        (None, None)
    };

    Ok(LcrXlayerInfo {
        rep_info,
        purpose_id,
        color_info,
        embedded_layer_info,
        xlayer_atlas,
    })
}

/// Parses `lcr_rep_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.7).
fn parse_lcr_rep_info(reader: &mut BitReader<'_>) -> Result<LcrRepInfo> {
    let max_pic_width = reader.read_uvlc()?;
    let max_pic_height = reader.read_uvlc()?;
    let format_info_present = reader.read_flag()?;
    let cropping_window_present = reader.read_flag()?;

    let format_info = if format_info_present {
        Some(LcrFormatInfo {
            bit_depth_idc: reader.read_uvlc()?,
            chroma_format_idc: reader.read_uvlc()?,
        })
    } else {
        None
    };
    let cropping_window = if cropping_window_present {
        Some(LcrCroppingWindow {
            left_offset: reader.read_uvlc()?,
            right_offset: reader.read_uvlc()?,
            top_offset: reader.read_uvlc()?,
            bottom_offset: reader.read_uvlc()?,
        })
    } else {
        None
    };

    Ok(LcrRepInfo {
        max_pic_width,
        max_pic_height,
        format_info,
        cropping_window,
    })
}

/// Parses `lcr_xlayer_color_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.9).
fn parse_lcr_xlayer_color_info(reader: &mut BitReader<'_>) -> Result<LcrXlayerColorInfo> {
    let color_description_idc = reader.read_rg(2)?;
    let primaries = if color_description_idc == 0 {
        Some((
            reader.read_bits_u8(8)?,
            reader.read_bits_u8(8)?,
            reader.read_bits_u8(8)?,
        ))
    } else {
        None
    };
    let full_range_flag = reader.read_flag()?;
    Ok(LcrXlayerColorInfo {
        color_description_idc,
        primaries,
        full_range_flag,
    })
}

/// Parses `lcr_embedded_layer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.8).
fn parse_lcr_embedded_layer_info(
    reader: &mut BitReader<'_>,
    context: &XlayerAtlasContext,
) -> Result<LcrEmbeddedLayerInfo> {
    let mlayer_map = reader.read_bits_u8(8)?;
    let atlas_segment_present = context.atlas_segment_present();

    let mut layers = Vec::new();
    for j in 0u8..8 {
        if mlayer_map & (1u8 << j) == 0 {
            continue;
        }

        let tlayer_map = reader.read_bits_u8(MAX_NUM_TLAYERS)?;

        let (atlas_segment_id, priority_order, rendering_method) = if atlas_segment_present {
            (
                Some(reader.read_bits_u8(8)?),
                Some(reader.read_bits_u8(8)?),
                Some(reader.read_bits_u8(8)?),
            )
        } else {
            (None, None, None)
        };

        let layer_type = reader.read_bits_u8(8)?;
        let auxiliary_type = if layer_type == AUX_LAYER {
            Some(reader.read_bits_u8(8)?)
        } else {
            None
        };

        let view_type = reader.read_bits_u8(8)?;
        let view_id = if view_type == VIEW_EXPLICIT {
            Some(reader.read_bits_u8(8)?)
        } else {
            None
        };

        let dependent_layer_map = if j > 0 {
            Some(reader.read_bits(u32::from(j))?)
        } else {
            None
        };

        let same_sh_max_resolution_flag = reader.read_flag()?;
        let (max_expected_width, max_expected_height) = if same_sh_max_resolution_flag {
            (None, None)
        } else {
            (Some(reader.read_uvlc()?), Some(reader.read_uvlc()?))
        };

        // AV2 § 5.8.8: byte_alignment() at the end of each set-bit iteration.
        reader.byte_align_zero()?;

        layers.push(LcrEmbeddedLayer {
            mlayer_index: j,
            tlayer_map,
            atlas_segment_id,
            priority_order,
            rendering_method,
            layer_type,
            auxiliary_type,
            view_type,
            view_id,
            dependent_layer_map,
            same_sh_max_resolution_flag,
            max_expected_width,
            max_expected_height,
        });
    }

    Ok(LcrEmbeddedLayerInfo { mlayer_map, layers })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::GLOBAL_XLAYER_ID;

    use crate::test_bits::Bits;

    /// The fixed `lcr_global_info()` prefix with every optional section absent.
    ///
    /// `xlayer_map` selects the associated extended layers; the three present flags and
    /// the atlas flag are caller-controlled, the rest are zero.
    fn global_prefix(
        global_id: u32,
        xlayer_map: u32,
        agg: bool,
        ptl: bool,
        payload: bool,
        atlas: bool,
    ) -> Bits {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(xlayer_map, 31); // lcr_xlayer_map
        bits.bit(u8::from(agg)); // lcr_aggregate_info_present_flag
        bits.bit(u8::from(ptl)); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(u8::from(payload)); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(u8::from(atlas)); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        if atlas {
            bits.f(0, 3); // lcr_global_atlas_id
        } else {
            bits.f(0, 3); // lcr_global_reserved_zero_3bits
        }
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        bits
    }

    /// A minimal `lcr_xlayer_info()` with all present flags clear (one byte: four flag
    /// bits then four alignment bits). When `global_atlas` is set, the else-branch
    /// `f(8)` atlas triple follows for the global case.
    fn minimal_xlayer_info(global_atlas: bool) -> Bits {
        let mut bits = Bits::default();
        bits.bit(0); // lcr_rep_info_present_flag
        bits.bit(0); // lcr_xlayer_purpose_present_flag
        bits.bit(0); // lcr_xlayer_color_info_present_flag
        bits.bit(0); // lcr_embedded_layer_info_present_flag
        bits.align(); // byte_alignment()
        if global_atlas {
            bits.f(0, 8); // lcr_xlayer_atlas_segment_id
            bits.f(0, 8); // lcr_xlayer_priority_order
            bits.f(0, 8); // lcr_xlayer_rendering_method
        }
        bits
    }

    fn parse(bytes: &[u8], xlayer: ExtendedLayerId) -> Result<LayerConfigurationRecord> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_layer_config_record(&mut reader, xlayer)
    }

    #[test]
    fn parses_minimal_global_lcr() {
        let data = global_prefix(1, 0b1, false, false, false, false).into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected a global LCR");
        };
        assert_eq!(info.global_config_record_id, 1);
        assert_eq!(info.xlayer_map, 0b1);
        assert!(!info.global_atlas_id_present);
        assert_eq!(info.global_atlas_id, None);
        assert!(info.aggregate_info.is_none());
        assert!(info.seq_ptl_infos.is_empty());
        assert!(info.payloads.is_empty());
        assert!(!info.has_nonzero_reserved_bits());
    }

    #[test]
    fn parses_global_lcr_with_aggregate_info() {
        let mut bits = global_prefix(7, 0b101, true, false, false, false);
        bits.f(0b10_1010, 6); // lcr_config_idc
        bits.f(0b1_0101, 5); // lcr_aggregate_level_idx
        bits.bit(1); // lcr_max_tier_flag
        bits.f(0b1001, 4); // lcr_max_interop
        let data = bits.into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected a global LCR");
        };
        let aggregate = info.aggregate_info.unwrap();
        assert_eq!(aggregate.config_idc, 0b10_1010);
        assert_eq!(aggregate.aggregate_level_idx, 0b1_0101);
        assert!(aggregate.max_tier_flag);
        assert_eq!(aggregate.max_interop, 0b1001);
    }

    #[test]
    fn parses_global_lcr_with_payload_and_remaining_bits() {
        // One xlayer (bit 0), payload present. data_size = 2 bytes: the minimal
        // xlayer_info is exactly 1 byte, leaving 8 remaining payload bits.
        let mut bits = global_prefix(2, 0b1, false, false, true, false);
        bits.leb128_byte(2); // lcr_data_size[0] = 2
        bits.bits.extend(minimal_xlayer_info(false).bits); // lcr_xlayer_info(1, 0)
        bits.f(0, 8); // 8 lcr_remaining_payload_bit
        let data = bits.into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected a global LCR");
        };
        assert_eq!(info.payloads.len(), 1);
        let payload = &info.payloads[0];
        assert_eq!(payload.xlayer_id, 0);
        assert_eq!(payload.data_size, 2);
        assert_eq!(payload.num_dependent_xlayer_map, None);
        assert_eq!(payload.remaining_payload_bits, 8);
    }

    #[test]
    fn global_payload_exact_size_has_no_remaining_bits() {
        let mut bits = global_prefix(2, 0b1, false, false, true, false);
        bits.leb128_byte(1); // data_size = 1 byte == the minimal xlayer_info
        bits.bits.extend(minimal_xlayer_info(false).bits);
        let data = bits.into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected a global LCR");
        };
        assert_eq!(info.payloads[0].remaining_payload_bits, 0);
    }

    #[test]
    fn global_payload_too_small_size_is_overflow() {
        // data_size = 0 cannot contain the 1-byte xlayer_info.
        let mut bits = global_prefix(2, 0b1, false, false, true, false);
        bits.leb128_byte(0);
        bits.bits.extend(minimal_xlayer_info(false).bits);
        let data = bits.into_bytes();
        assert!(matches!(
            parse(&data, GLOBAL_XLAYER_ID),
            Err(Error::InvalidLayerConfigRecord {
                kind: LayerConfigRecordErrorKind::PayloadSizeOverflow,
                ..
            })
        ));
    }

    #[test]
    fn parses_global_lcr_with_atlas_id() {
        let mut bits = Bits::default();
        bits.f(3, 3); // lcr_global_config_record_id
        bits.f(0b1, 31); // lcr_xlayer_map
        bits.bit(0); // aggregate
        bits.bit(0); // ptl
        bits.bit(0); // payload
        bits.bit(0); // dependent
        bits.bit(1); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // purpose
        bits.bit(0); // doh
        bits.bit(0); // tile alignment
        bits.f(5, 3); // lcr_global_atlas_id = 5
        bits.f(0, 5); // reserved_zero_5bits
        let data = bits.into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected a global LCR");
        };
        assert!(info.global_atlas_id_present);
        assert_eq!(info.global_atlas_id, Some(5));
        assert_eq!(info.reserved_zero_3bits, 0);
    }

    #[test]
    fn parses_minimal_local_lcr_referencing_global() {
        let mut bits = Bits::default();
        bits.f(3, 3); // lcr_global_id = 3
        bits.f(1, 3); // lcr_local_id = 1
        bits.bit(0); // lcr_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_local_atlas_id_present_flag
        bits.f(0, 3); // lcr_local_reserved_zero_3bits
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        bits.bits.extend(minimal_xlayer_info(false).bits); // lcr_xlayer_info(0, xId)
        let data = bits.into_bytes();
        let record = parse(&data, ExtendedLayerId::from_bits(2)).unwrap();
        let LayerConfigurationRecord::Local(info) = record else {
            panic!("expected a local LCR");
        };
        assert_eq!(info.xlayer_id, 2);
        assert_eq!(info.global_id, 3);
        assert_eq!(info.local_id, 1);
        assert_eq!(info.local_atlas_id, None);
        assert!(!info.has_nonzero_reserved_bits());
    }

    #[test]
    fn parses_local_lcr_with_local_atlas_id() {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id = 0 (no global association)
        bits.f(2, 3); // lcr_local_id = 2
        bits.bit(0); // ptl present
        bits.bit(1); // lcr_local_atlas_id_present_flag
        bits.f(4, 3); // lcr_local_atlas_id = 4
        bits.f(0, 5); // reserved_zero_5bits
        bits.bits.extend(minimal_xlayer_info(false).bits);
        let data = bits.into_bytes();
        let record = parse(&data, ExtendedLayerId::from_bits(1)).unwrap();
        let LayerConfigurationRecord::Local(info) = record else {
            panic!("expected a local LCR");
        };
        assert!(info.local_atlas_id_present);
        assert_eq!(info.local_atlas_id, Some(4));
        assert_eq!(info.global_id, 0);
    }

    #[test]
    fn local_lcr_embedded_layer_with_atlas_and_color() {
        // A local LCR with a local atlas, whose xlayer_info carries color info and an
        // embedded layer map selecting mlayer 0 with an atlas segment.
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(1, 3); // lcr_local_id
        bits.bit(0); // ptl present
        bits.bit(1); // local atlas present
        bits.f(1, 3); // lcr_local_atlas_id = 1
        bits.f(0, 5); // reserved_zero_5bits
        // lcr_xlayer_info(0, xId):
        bits.bit(0); // rep_info present
        bits.bit(0); // purpose present
        bits.bit(1); // color info present
        bits.bit(1); // embedded layer info present
        bits.rg(0, 2); // layer_color_description_idc = 0 -> primaries present
        bits.f(1, 8); // layer_color_primaries
        bits.f(13, 8); // layer_transfer_characteristics
        bits.f(6, 8); // layer_matrix_coefficients
        bits.bit(1); // layer_full_range_flag
        bits.align(); // byte_alignment()
        // lcr_embedded_layer_info(0, xId): mlayer_map = 0b0000_0001 -> only j=0.
        bits.f(0b0000_0001, 8);
        // j = 0 (atlas present because local atlas present):
        bits.f(0b0101, MAX_NUM_TLAYERS); // lcr_tlayer_map
        bits.f(7, 8); // lcr_layer_atlas_segment_id
        bits.f(2, 8); // lcr_priority_order
        bits.f(0, 8); // lcr_rendering_method
        bits.f(AUX_LAYER as u32, 8); // lcr_layer_type = AUX_LAYER -> auxiliary type follows
        bits.f(9, 8); // lcr_auxiliary_type
        bits.f(VIEW_EXPLICIT as u32, 8); // lcr_view_type = VIEW_EXPLICIT -> view id follows
        bits.f(3, 8); // lcr_view_id
        // j == 0 so no lcr_dependent_layer_map.
        bits.bit(0); // lcr_same_sh_max_resolution_flag = 0 -> max expected follows
        bits.uvlc(1920); // lcr_max_expected_width
        bits.uvlc(1080); // lcr_max_expected_height
        bits.align(); // per-iteration byte_alignment()
        let data = bits.into_bytes();
        let record = parse(&data, ExtendedLayerId::from_bits(1)).unwrap();
        let LayerConfigurationRecord::Local(info) = record else {
            panic!("expected a local LCR");
        };
        let color = info.xlayer_info.color_info.unwrap();
        assert_eq!(color.primaries, Some((1, 13, 6)));
        assert!(color.full_range_flag);
        let embedded = info.xlayer_info.embedded_layer_info.unwrap();
        assert_eq!(embedded.mlayer_map, 0b0000_0001);
        assert_eq!(embedded.layers.len(), 1);
        let layer = embedded.layers[0];
        assert_eq!(layer.mlayer_index, 0);
        assert_eq!(layer.tlayer_map, 0b0101);
        assert_eq!(layer.atlas_segment_id, Some(7));
        assert_eq!(layer.layer_type, AUX_LAYER);
        assert_eq!(layer.auxiliary_type, Some(9));
        assert_eq!(layer.view_type, VIEW_EXPLICIT);
        assert_eq!(layer.view_id, Some(3));
        assert_eq!(layer.max_expected_width, Some(1920));
        assert_eq!(layer.max_expected_height, Some(1080));
    }

    #[test]
    fn detects_nonzero_reserved_bits() {
        let mut bits = global_prefix(1, 0b1, false, false, false, false);
        // Overwrite the trailing reserved_zero_5bits (last 5 bits appended) with a
        // non-zero pattern by rebuilding the prefix with a nonzero reserved field.
        // Simpler: build the prefix manually with reserved_zero_5bits != 0.
        bits.bits.clear();
        bits.f(1, 3); // id
        bits.f(0b1, 31); // map
        bits.bit(0); // aggregate
        bits.bit(0); // ptl
        bits.bit(0); // payload
        bits.bit(0); // dependent
        bits.bit(0); // atlas present
        bits.f(0, 7); // purpose
        bits.bit(0); // doh
        bits.bit(0); // tile
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0b1_0001, 5); // reserved_zero_5bits != 0
        let data = bits.into_bytes();
        let record = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        assert!(record.has_nonzero_reserved_bits());
    }

    #[test]
    fn reports_eof_in_global_prefix() {
        assert!(matches!(
            parse(&[0x00], GLOBAL_XLAYER_ID),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn reports_eof_in_local_prefix() {
        assert!(matches!(
            parse(&[0x00], ExtendedLayerId::from_bits(1)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn rejects_nonzero_byte_alignment_bits() {
        // A local LCR whose xlayer_info has a non-zero byte-alignment bit.
        let mut bits = Bits::default();
        bits.f(0, 3); // global_id
        bits.f(1, 3); // local_id
        bits.bit(0); // ptl present
        bits.bit(0); // local atlas present
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // reserved_zero_5bits
        // xlayer_info: 4 present flags clear, then a non-zero alignment bit.
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // alignment bit must be 0 -> violation
        bits.f(0, 3); // pad to a byte
        let data = bits.into_bytes();
        assert!(matches!(
            parse(&data, ExtendedLayerId::from_bits(1)),
            Err(Error::InvalidByteAlignment { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::{ExtendedLayerId, GLOBAL_XLAYER_ID};
    use proptest::prelude::*;

    proptest! {
        /// The layer-config-record parser must never panic on arbitrary input, for
        /// both the global and local branches.
        #[test]
        fn layer_config_record_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut global = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_layer_config_record(&mut global, GLOBAL_XLAYER_ID);

            let mut local = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_layer_config_record(&mut local, ExtendedLayerId::from_bits(1));
        }
    }
}
