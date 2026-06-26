// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 operating point set OBU syntax model (AV2 v1.0.0 § 5.10, § 5.11,
//! § 5.11.1-§ 5.11.5).
//!
//! [`parse_operating_point_set`] reads `operating_point_set_obu()` (§ 5.10) and its
//! `operating_point_payload()` children (§ 5.11), dispatching to the child syntax
//! structures `ops_aggregate_info()` (§ 5.11.1), `ops_seq_profile_tier_level_info()`
//! (§ 5.11.2), `ops_decoder_model_info()` (§ 5.11.3), `ops_color_info()` (§ 5.11.4),
//! and `ops_mlayer_info()` (§ 5.11.5).
//!
//! The parser preserves the source values that the validator needs for the
//! locally-checkable § 6.10 conformance rules: the local reserved bits, the global
//! `ops_mlayer_info_idc`, each PTL reserved field, the declared `ops_data_size`
//! alongside the computed `opsBytes`, and the inherited-operating-point references.
//! Reserved-nonzero values are retained rather than rejected so the validation layer
//! can report them with byte offsets; only truncated or malformed input produces a
//! typed [`Error`](crate::error::Error).
//!
//! This module models syntax only. Annex A level conformance, Annex E decoder
//! schedule validation, and active-sequence dependency-map agreement (§ 6.10.7) are
//! out of scope here.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::ProfileIdc;
use crate::types::ExtendedLayerId;

/// `ops_id` is a 4-bit field (`f(4)`); the OPS identifier within an extended layer.
const OPS_ID_BITS: u32 = 4;
/// `ops_cnt` is a 3-bit field (`f(3)`); 0 signals reset, 1..7 operating points.
const OPS_COUNT_BITS: u32 = 3;
/// `ops_priority` is a 4-bit field (`f(4)`).
const OPS_PRIORITY_BITS: u32 = 4;
/// `ops_intent` / `ops_op_intent` is a 7-bit field (`f(7)`).
const OPS_INTENT_BITS: u32 = 7;
/// Shared 2-bit field width (`f(2)`) for `ops_mlayer_info_idc`, `ops_reserved_2bits`,
/// and `ops_ptl_reserved_2bits`.
const OPS_RESERVED_2BITS: u32 = 2;
/// `ops_mlayer_info_idc == 3` is reserved and must not appear (§ 6.10.2).
const OPS_MLAYER_INFO_IDC_RESERVED: u8 = 3;
/// `ops_xlayer_map` is a 31-bit field (`f(31)`, `MAX_NUM_XLAYERS - 1`); bit `j`
/// selects extended layer `j` (0..30). `GLOBAL_XLAYER_ID` (31) is excluded.
const OPS_XLAYER_MAP_BITS: u32 = 31;
/// `ops_mlayer_map` is an 8-bit field (`f(8)`, `MAX_NUM_MLAYERS`).
const OPS_MLAYER_MAP_BITS: u32 = 8;
/// `ops_tlayer_map` is a 4-bit field (`f(4)`, `MAX_NUM_TLAYERS`).
const OPS_TLAYER_MAP_BITS: u32 = 4;
/// `ops_embedded_ops_id` is a 4-bit field (`f(4)`).
const OPS_EMBEDDED_OPS_ID_BITS: u32 = 4;
/// `ops_embedded_op_index` is a 3-bit field (`f(3)`).
const OPS_EMBEDDED_OP_INDEX_BITS: u32 = 3;
/// `ops_initial_display_delay_minus_1` is a 4-bit field (`f(4)`).
const OPS_INITIAL_DISPLAY_DELAY_BITS: u32 = 4;

/// Parsed `operating_point_set_obu()` syntax (AV2 v1.0.0 § 5.10).
///
/// The OPS is identified by `(xlayer_id, ops_id)`. `ops_cnt == 0` is a reset rather
/// than an active zero-operating-point set, so `payloads` is empty and the optional
/// header fields are absent. When `ops_cnt > 0`, the header fields are present and
/// `payloads` holds exactly `ops_cnt` parsed operating points.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperatingPointSet {
    /// `obu_xlayer_id` of the OBU carrying this OPS. `GLOBAL_XLAYER_ID` marks a
    /// global OPS that applies to the whole multistream.
    pub xlayer_id: ExtendedLayerId,
    /// `ops_reset_flag` (`f(1)`): with `ops_cnt`, selects reset/update behavior.
    pub reset_flag: bool,
    /// `ops_id` (`f(4)`): OPS identifier within the extended layer (0..15).
    pub ops_id: u8,
    /// `ops_cnt` (`f(3)`): number of operating points (0 = reset, else 1..7).
    pub ops_cnt: u8,
    /// `ops_priority` (`f(4)`): present only when `ops_cnt > 0`.
    pub priority: Option<u8>,
    /// `ops_intent` (`f(7)`): present only when `ops_cnt > 0`.
    pub intent: Option<u8>,
    /// `ops_intent_present_flag` (`f(1)`): whether each payload carries
    /// `ops_op_intent`. `false` when `ops_cnt == 0`.
    pub intent_present: bool,
    /// `ops_ptl_present_flag` (`f(1)`): whether profile/tier/level info is present.
    /// `false` when `ops_cnt == 0`.
    pub ptl_present: bool,
    /// `ops_color_info_present_flag` (`f(1)`): whether each payload carries
    /// `ops_color_info()`. `false` when `ops_cnt == 0`.
    pub color_info_present: bool,
    /// `ops_mlayer_info_idc` (`f(2)`): present only for a global OPS with
    /// `ops_cnt > 0`. A value of `OPS_MLAYER_INFO_IDC_RESERVED` is reserved.
    pub mlayer_info_idc: Option<u8>,
    /// `ops_reserved_2bits` (`f(2)`): present only for a local OPS with
    /// `ops_cnt > 0`. Conformance requires it to be zero (§ 6.10.2).
    pub local_reserved_2bits: Option<u8>,
    /// The `ops_cnt` parsed `operating_point_payload()` structures (§ 5.11).
    pub payloads: Vec<OperatingPointPayload>,
}

impl OperatingPointSet {
    /// Returns `true` if this OPS is carried by a global (`GLOBAL_XLAYER_ID`) OBU.
    #[must_use]
    pub fn is_global(&self) -> bool {
        self.xlayer_id.is_global()
    }

    /// Returns `true` if a local OPS carries a nonzero `ops_reserved_2bits`
    /// (a § 6.10.2 conformance violation).
    #[must_use]
    pub fn has_nonzero_local_reserved_bits(&self) -> bool {
        matches!(self.local_reserved_2bits, Some(bits) if bits != 0)
    }

    /// Returns `true` if a global OPS carries the reserved `ops_mlayer_info_idc == 3`
    /// (a § 6.10.2 conformance violation).
    #[must_use]
    pub fn has_reserved_mlayer_info_idc(&self) -> bool {
        self.mlayer_info_idc == Some(OPS_MLAYER_INFO_IDC_RESERVED)
    }
}

/// Parsed `operating_point_payload()` syntax (AV2 v1.0.0 § 5.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperatingPointPayload {
    /// Operating point index `i` within the OPS (0..`ops_cnt - 1`).
    pub index: u8,
    /// `ops_data_size` (`leb128()`): the declared payload size in bytes.
    pub declared_size_bytes: u32,
    /// `opsBytes`: the byte count actually parsed for this payload, measured from
    /// after `ops_data_size` through the closing `byte_alignment()`. Conformance
    /// requires it to equal [`OperatingPointPayload::declared_size_bytes`]
    /// (§ 6.10.2).
    pub computed_size_bytes: u32,
    /// `ops_op_intent` (`f(7)`): present when `ops_intent_present_flag` is set.
    pub op_intent: Option<u8>,
    /// `ops_aggregate_info()` (§ 5.11.1): present for a global OPS payload when
    /// `ops_ptl_present_flag` is set.
    pub aggregate_info: Option<OpsAggregateInfo>,
    /// `ops_color_info()` (§ 5.11.4): present when `ops_color_info_present_flag` is
    /// set.
    pub color_info: Option<OpsColorInfo>,
    /// `ops_decoder_model_info()` (§ 5.11.3): present when
    /// `ops_decoder_model_info_for_this_op_present_flag` is set.
    pub decoder_model_info: Option<OpsDecoderModelInfo>,
    /// `ops_initial_display_delay_minus_1` (`f(4)`): present when
    /// `ops_initial_display_delay_present_flag` is set.
    pub initial_display_delay_minus_1: Option<u8>,
    /// `ops_xlayer_map` (`f(31)`): present only for a global OPS payload.
    pub xlayer_map: Option<u32>,
    /// One entry per included extended layer (`OpsxLayerId`): the global xlayer-map
    /// bits in ascending order, or the single local layer.
    pub xlayer_entries: Vec<OpsXlayerEntry>,
}

impl OperatingPointPayload {
    /// Returns `true` if the computed `opsBytes` differs from the declared
    /// `ops_data_size` (a § 6.10.2 conformance violation).
    #[must_use]
    pub fn has_size_mismatch(&self) -> bool {
        self.declared_size_bytes != self.computed_size_bytes
    }
}

/// One included extended layer within an `operating_point_payload()` (§ 5.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsXlayerEntry {
    /// `OpsxLayerId`: the included extended layer (a global xlayer-map bit `j`, or
    /// the local OBU's own `obu_xlayer_id`).
    pub xlayer_id: ExtendedLayerId,
    /// `ops_seq_profile_tier_level_info()` (§ 5.11.2): present when
    /// `ops_ptl_present_flag` is set.
    pub ptl_info: Option<OpsSeqProfileTierLevelInfo>,
    /// The embedded/temporal layer information source for this extended layer.
    pub mlayer: OpsMlayerSource,
}

/// How an extended layer's `ops_mlayer_info()` is provided (§ 5.11, § 5.11.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpsMlayerSource {
    /// No mlayer info is coded for this layer (global `ops_mlayer_info_idc == 0`, or
    /// the reserved value `3` whose syntax codes nothing).
    Absent,
    /// `ops_mlayer_info()` is explicitly coded (every local OPS layer; a global
    /// layer when `ops_mlayer_info_idc == 1`, or `== 2` with the explicit flag set).
    Explicit(OpsMlayerInfo),
    /// The embedded/temporal layer structure is inherited from another operating
    /// point (`ops_mlayer_info_idc == 2` with the explicit flag clear).
    Inherited {
        /// `ops_embedded_ops_id` (`f(4)`): the OPS the configuration is inherited
        /// from.
        embedded_ops_id: u8,
        /// `ops_embedded_op_index` (`f(3)`): the operating point index within the
        /// referenced OPS.
        embedded_op_index: u8,
    },
}

/// Parsed `ops_aggregate_info()` syntax (AV2 v1.0.0 § 5.11.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsAggregateInfo {
    /// `ops_config_idc` (`f(6)`): aggregate profile identifier.
    pub config_idc: u8,
    /// `ops_aggregate_level_idx` (`f(5)`): aggregate level index.
    pub aggregate_level_idx: u8,
    /// `ops_max_tier_flag` (`f(1)`).
    pub max_tier_flag: bool,
    /// `ops_max_interop` (`f(4)`): maximum interoperability point.
    pub max_interop: u8,
}

/// Parsed `ops_seq_profile_tier_level_info()` syntax (AV2 v1.0.0 § 5.11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsSeqProfileTierLevelInfo {
    /// The extended layer this PTL info targets (`xId` locally, `j` globally).
    pub target_xlayer_id: ExtendedLayerId,
    /// `ops_seq_profile_idc` (`f(5)`; Annex A Table A.1 value space).
    pub seq_profile_idc: ProfileIdc,
    /// `ops_level_idx` (`f(5)`).
    pub level_idx: u8,
    /// `ops_tier_flag` (`f(1)`).
    pub tier_flag: bool,
    /// `ops_mlayer_count` (`f(3)`).
    pub mlayer_count: u8,
    /// `ops_ptl_reserved_2bits` (`f(2)`): conformance requires it to be zero
    /// (§ 6.10.4).
    pub reserved_2bits: u8,
}

/// Parsed `ops_decoder_model_info()` syntax (AV2 v1.0.0 § 5.11.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsDecoderModelInfo {
    /// `ops_decoder_buffer_delay` (`uvlc()`).
    pub decoder_buffer_delay: u32,
    /// `ops_encoder_buffer_delay` (`uvlc()`).
    pub encoder_buffer_delay: u32,
    /// `ops_low_delay_mode_flag` (`f(1)`).
    pub low_delay_mode_flag: bool,
}

/// Parsed `ops_color_info()` syntax (AV2 v1.0.0 § 5.11.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsColorInfo {
    /// `ops_color_description_idc` (`rg(2)`). When `0`, the explicit color triple
    /// follows.
    pub color_description_idc: u32,
    /// `ops_color_primaries` (`f(8)`): present when `color_description_idc == 0`.
    pub color_primaries: Option<u8>,
    /// `ops_transfer_characteristics` (`f(8)`): present when
    /// `color_description_idc == 0`.
    pub transfer_characteristics: Option<u8>,
    /// `ops_matrix_coefficients` (`f(8)`): present when `color_description_idc == 0`.
    pub matrix_coefficients: Option<u8>,
    /// `ops_full_range_flag` (`f(1)`).
    pub full_range_flag: bool,
}

/// Parsed `ops_mlayer_info()` syntax (AV2 v1.0.0 § 5.11.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpsMlayerInfo {
    /// `ops_mlayer_map` (`f(8)`): bit `j` selects embedded layer `j`.
    pub mlayer_map: u8,
    /// `(embedded layer index, ops_tlayer_map)` pairs (`f(4)` each), one per set bit
    /// of `mlayer_map` in ascending order.
    pub tlayer_maps: Vec<(u8, u8)>,
}

/// Parses an `operating_point_set_obu()` (AV2 v1.0.0 § 5.10).
///
/// `xlayer_id` is the OBU's `obu_xlayer_id`, which selects the global vs local
/// syntax branches. The parser reads the full known OPS syntax; the caller is
/// responsible for the extensible OBU tail via `finish_obu_payload()`.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) and the
/// other descriptor errors from [`BitReader`] when the input is truncated or a
/// variable-length code is malformed. Reserved-nonzero values are preserved for the
/// validator and are not parse errors.
pub fn parse_operating_point_set(
    reader: &mut BitReader<'_>,
    xlayer_id: ExtendedLayerId,
) -> Result<OperatingPointSet> {
    let reset_flag = reader.read_bit()? != 0;
    let ops_id = reader.read_bits_u8(OPS_ID_BITS)?;
    let ops_cnt = reader.read_bits_u8(OPS_COUNT_BITS)?;

    let is_global = xlayer_id.is_global();
    let mut priority = None;
    let mut intent = None;
    let mut intent_present = false;
    let mut ptl_present = false;
    let mut color_info_present = false;
    let mut mlayer_info_idc = None;
    let mut local_reserved_2bits = None;
    let mut payloads = Vec::new();

    if ops_cnt > 0 {
        priority = Some(reader.read_bits_u8(OPS_PRIORITY_BITS)?);
        intent = Some(reader.read_bits_u8(OPS_INTENT_BITS)?);
        intent_present = reader.read_bit()? != 0;
        ptl_present = reader.read_bit()? != 0;
        color_info_present = reader.read_bit()? != 0;
        if is_global {
            mlayer_info_idc = Some(reader.read_bits_u8(OPS_RESERVED_2BITS)?);
        } else {
            local_reserved_2bits = Some(reader.read_bits_u8(OPS_RESERVED_2BITS)?);
        }

        let header = OpsPayloadFlags {
            intent_present,
            ptl_present,
            color_info_present,
            mlayer_info_idc,
        };
        for index in 0..ops_cnt {
            payloads.push(parse_operating_point_payload(
                reader, xlayer_id, index, &header,
            )?);
        }
    }

    Ok(OperatingPointSet {
        xlayer_id,
        reset_flag,
        ops_id,
        ops_cnt,
        priority,
        intent,
        intent_present,
        ptl_present,
        color_info_present,
        mlayer_info_idc,
        local_reserved_2bits,
        payloads,
    })
}

/// The OPS-header presence flags that drive `operating_point_payload()` parsing.
struct OpsPayloadFlags {
    intent_present: bool,
    ptl_present: bool,
    color_info_present: bool,
    mlayer_info_idc: Option<u8>,
}

/// Parses one `operating_point_payload()` (AV2 v1.0.0 § 5.11).
fn parse_operating_point_payload(
    reader: &mut BitReader<'_>,
    xlayer_id: ExtendedLayerId,
    index: u8,
    flags: &OpsPayloadFlags,
) -> Result<OperatingPointPayload> {
    let declared_size_bytes = reader.read_leb128()?;
    // `startPos = get_position()` immediately after `ops_data_size` (§ 5.11).
    let start_bits = reader.consumed_bits();
    let is_global = xlayer_id.is_global();

    let op_intent = if flags.intent_present {
        Some(reader.read_bits_u8(OPS_INTENT_BITS)?)
    } else {
        None
    };

    // For a local OPS the top-level PTL targets the OBU's own layer; it is stored on
    // the single xlayer entry below. For a global OPS the top-level structure is
    // `ops_aggregate_info()` and the per-layer PTL is read inside the xlayer loop.
    let mut aggregate_info = None;
    let mut local_ptl_info = None;
    if flags.ptl_present {
        if is_global {
            aggregate_info = Some(parse_ops_aggregate_info(reader)?);
        } else {
            local_ptl_info = Some(parse_ops_seq_profile_tier_level_info(reader, xlayer_id)?);
        }
    }

    let color_info = if flags.color_info_present {
        Some(parse_ops_color_info(reader)?)
    } else {
        None
    };

    let decoder_model_present = reader.read_bit()? != 0;
    let decoder_model_info = if decoder_model_present {
        Some(parse_ops_decoder_model_info(reader)?)
    } else {
        None
    };

    let initial_display_delay_present = reader.read_bit()? != 0;
    let initial_display_delay_minus_1 = if initial_display_delay_present {
        Some(reader.read_bits_u8(OPS_INITIAL_DISPLAY_DELAY_BITS)?)
    } else {
        None
    };

    let mut xlayer_map = None;
    let mut xlayer_entries = Vec::new();
    if is_global {
        let map = reader.read_bits(OPS_XLAYER_MAP_BITS)?;
        xlayer_map = Some(map);
        for j in 0u8..31 {
            if map & (1u32 << u32::from(j)) == 0 {
                continue;
            }
            let entry_xlayer = ExtendedLayerId::from_bits(j);
            let ptl_info = if flags.ptl_present {
                Some(parse_ops_seq_profile_tier_level_info(reader, entry_xlayer)?)
            } else {
                None
            };
            let mlayer = parse_global_mlayer_source(reader, flags.mlayer_info_idc)?;
            xlayer_entries.push(OpsXlayerEntry {
                xlayer_id: entry_xlayer,
                ptl_info,
                mlayer,
            });
        }
    } else {
        // Local OPS: `XCount == 1`, `OpsxLayerId[0] == xId`, and `ops_mlayer_info()`
        // is always coded.
        let mlayer = OpsMlayerSource::Explicit(parse_ops_mlayer_info(reader)?);
        xlayer_entries.push(OpsXlayerEntry {
            xlayer_id,
            ptl_info: local_ptl_info,
            mlayer,
        });
    }

    // `byte_alignment()` then `opsBytes = (get_position() - startPos) >> 3`.
    reader.byte_align_zero()?;
    let consumed = reader.consumed_bits().saturating_sub(start_bits);
    // `0` (rather than `u32::MAX`) is the safe fallback for the practically-impossible
    // overflow: an under-count trips `ops/payload-size-mismatch` instead of silently
    // matching a declared `ops_data_size` of `u32::MAX`.
    let computed_size_bytes = u32::try_from(consumed / 8).unwrap_or(0);

    Ok(OperatingPointPayload {
        index,
        declared_size_bytes,
        computed_size_bytes,
        op_intent,
        aggregate_info,
        color_info,
        decoder_model_info,
        initial_display_delay_minus_1,
        xlayer_map,
        xlayer_entries,
    })
}

/// Reads the per-layer mlayer info source for a global OPS payload, following the
/// `ops_mlayer_info_idc` branches of § 5.11. `idc` values `0` and the reserved `3`
/// code nothing.
fn parse_global_mlayer_source(
    reader: &mut BitReader<'_>,
    mlayer_info_idc: Option<u8>,
) -> Result<OpsMlayerSource> {
    match mlayer_info_idc {
        Some(1) => Ok(OpsMlayerSource::Explicit(parse_ops_mlayer_info(reader)?)),
        Some(2) => {
            let explicit = reader.read_bit()? != 0;
            if explicit {
                Ok(OpsMlayerSource::Explicit(parse_ops_mlayer_info(reader)?))
            } else {
                let embedded_ops_id = reader.read_bits_u8(OPS_EMBEDDED_OPS_ID_BITS)?;
                let embedded_op_index = reader.read_bits_u8(OPS_EMBEDDED_OP_INDEX_BITS)?;
                Ok(OpsMlayerSource::Inherited {
                    embedded_ops_id,
                    embedded_op_index,
                })
            }
        }
        _ => Ok(OpsMlayerSource::Absent),
    }
}

/// Parses `ops_aggregate_info()` (AV2 v1.0.0 § 5.11.1).
fn parse_ops_aggregate_info(reader: &mut BitReader<'_>) -> Result<OpsAggregateInfo> {
    let config_idc = reader.read_bits_u8(6)?;
    let aggregate_level_idx = reader.read_bits_u8(5)?;
    let max_tier_flag = reader.read_bit()? != 0;
    let max_interop = reader.read_bits_u8(4)?;
    Ok(OpsAggregateInfo {
        config_idc,
        aggregate_level_idx,
        max_tier_flag,
        max_interop,
    })
}

/// Parses `ops_seq_profile_tier_level_info()` (AV2 v1.0.0 § 5.11.2) targeting
/// `target_xlayer_id`.
fn parse_ops_seq_profile_tier_level_info(
    reader: &mut BitReader<'_>,
    target_xlayer_id: ExtendedLayerId,
) -> Result<OpsSeqProfileTierLevelInfo> {
    let seq_profile_idc = ProfileIdc::from_bits(reader.read_bits_u8(5)?);
    let level_idx = reader.read_bits_u8(5)?;
    let tier_flag = reader.read_bit()? != 0;
    let mlayer_count = reader.read_bits_u8(3)?;
    let reserved_2bits = reader.read_bits_u8(OPS_RESERVED_2BITS)?;
    Ok(OpsSeqProfileTierLevelInfo {
        target_xlayer_id,
        seq_profile_idc,
        level_idx,
        tier_flag,
        mlayer_count,
        reserved_2bits,
    })
}

/// Parses `ops_decoder_model_info()` (AV2 v1.0.0 § 5.11.3).
fn parse_ops_decoder_model_info(reader: &mut BitReader<'_>) -> Result<OpsDecoderModelInfo> {
    let decoder_buffer_delay = reader.read_uvlc()?;
    let encoder_buffer_delay = reader.read_uvlc()?;
    let low_delay_mode_flag = reader.read_bit()? != 0;
    Ok(OpsDecoderModelInfo {
        decoder_buffer_delay,
        encoder_buffer_delay,
        low_delay_mode_flag,
    })
}

/// Parses `ops_color_info()` (AV2 v1.0.0 § 5.11.4).
fn parse_ops_color_info(reader: &mut BitReader<'_>) -> Result<OpsColorInfo> {
    let color_description_idc = reader.read_rg(2)?;
    let (color_primaries, transfer_characteristics, matrix_coefficients) =
        if color_description_idc == 0 {
            (
                Some(reader.read_bits_u8(8)?),
                Some(reader.read_bits_u8(8)?),
                Some(reader.read_bits_u8(8)?),
            )
        } else {
            (None, None, None)
        };
    let full_range_flag = reader.read_bit()? != 0;
    Ok(OpsColorInfo {
        color_description_idc,
        color_primaries,
        transfer_characteristics,
        matrix_coefficients,
        full_range_flag,
    })
}

/// Parses `ops_mlayer_info()` (AV2 v1.0.0 § 5.11.5).
fn parse_ops_mlayer_info(reader: &mut BitReader<'_>) -> Result<OpsMlayerInfo> {
    let mlayer_map = reader.read_bits_u8(OPS_MLAYER_MAP_BITS)?;
    let mut tlayer_maps = Vec::new();
    for j in 0u8..8 {
        if mlayer_map & (1u8 << j) == 0 {
            continue;
        }
        let tlayer_map = reader.read_bits_u8(OPS_TLAYER_MAP_BITS)?;
        tlayer_maps.push((j, tlayer_map));
    }
    Ok(OpsMlayerInfo {
        mlayer_map,
        tlayer_maps,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::{ExtendedLayerId, GLOBAL_XLAYER_ID};

    use crate::test_bits::Bits;

    fn parse(bytes: &[u8], xlayer: ExtendedLayerId) -> Result<OperatingPointSet> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_operating_point_set(&mut reader, xlayer)
    }

    /// Builds the body of one `operating_point_payload()` (everything after
    /// `ops_data_size`), so the caller can prepend a correct or deliberately-wrong
    /// `ops_data_size`. Returns `(body, byte_len)` where `byte_len` is the aligned
    /// `opsBytes`.
    fn local_payload_body(mlayer_map: u8) -> (Bits, u32) {
        let mut body = Bits::default();
        // No intent / ptl / color (the OPS header below clears those flags).
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(u32::from(mlayer_map), 8); // ops_mlayer_info(): ops_mlayer_map
        // No set bits in mlayer_map -> no tlayer maps when mlayer_map == 0.
        body.align();
        let byte_len = (body.bit_len() / 8) as u32;
        (body, byte_len)
    }

    #[test]
    fn ops_reset_only_local() {
        // ops_reset_flag=0, ops_id=3, ops_cnt=0 -> one byte, no payloads.
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(3, 4); // ops_id
        bits.f(0, 3); // ops_cnt
        let data = bits.into_bytes();
        let ops = parse(&data, ExtendedLayerId::from_bits(2)).unwrap();
        assert!(!ops.reset_flag);
        assert_eq!(ops.ops_id, 3);
        assert_eq!(ops.ops_cnt, 0);
        assert!(ops.payloads.is_empty());
        assert!(ops.priority.is_none());
        assert!(ops.local_reserved_2bits.is_none());
        assert!(!ops.is_global());
    }

    #[test]
    fn ops_global_one_payload_no_optional_fields() {
        // Global OPS, ops_cnt=1, all present flags 0, idc=0, one xlayer (layer 0),
        // no PTL/mlayer because idc==0.
        let mut payload = Bits::default();
        payload.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        payload.bit(0); // ops_initial_display_delay_present_flag
        payload.f(0b1, 31); // ops_xlayer_map -> only layer 0
        // idc == 0 => no PTL (ptl flag clear) and no mlayer info for layer 0.
        payload.align();
        let payload_bytes = (payload.bit_len() / 8) as u32;

        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_mlayer_info_idc (global) = 0
        bits.f(payload_bytes, 8); // ops_data_size (single-byte leb128)
        bits.append(&payload);

        let data = bits.into_bytes();
        let ops = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        assert!(ops.is_global());
        assert_eq!(ops.ops_cnt, 1);
        assert_eq!(ops.mlayer_info_idc, Some(0));
        assert_eq!(ops.payloads.len(), 1);
        let payload = &ops.payloads[0];
        assert_eq!(payload.xlayer_map, Some(0b1));
        assert_eq!(payload.xlayer_entries.len(), 1);
        assert_eq!(
            payload.xlayer_entries[0].xlayer_id,
            ExtendedLayerId::from_bits(0)
        );
        assert!(matches!(
            payload.xlayer_entries[0].mlayer,
            OpsMlayerSource::Absent
        ));
        assert!(!payload.has_size_mismatch());
    }

    #[test]
    fn ops_local_reserved_bits_nonzero_is_preserved_for_validator() {
        let (body, body_len) = local_payload_body(0);
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(1, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0b10, 2); // ops_reserved_2bits (local) = 2, nonzero
        bits.f(body_len, 8); // ops_data_size
        bits.append(&body);

        let data = bits.into_bytes();
        let ops = parse(&data, ExtendedLayerId::from_bits(4)).unwrap();
        assert_eq!(ops.local_reserved_2bits, Some(0b10));
        assert!(ops.has_nonzero_local_reserved_bits());
        assert!(!ops.has_reserved_mlayer_info_idc());
    }

    #[test]
    fn ops_mlayer_info_idc_reserved_is_preserved_for_validator() {
        // Global OPS with ops_mlayer_info_idc == 3 (reserved). idc 3 codes no mlayer
        // info, so a single-layer payload parses cleanly.
        let mut payload = Bits::default();
        payload.bit(0); // decoder model present
        payload.bit(0); // initial display delay present
        payload.f(0b1, 31); // ops_xlayer_map -> layer 0
        payload.align();
        let payload_bytes = (payload.bit_len() / 8) as u32;

        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // intent present
        bits.bit(0); // ptl present
        bits.bit(0); // color present
        bits.f(3, 2); // ops_mlayer_info_idc = 3 (reserved)
        bits.f(payload_bytes, 8); // ops_data_size
        bits.append(&payload);

        let data = bits.into_bytes();
        let ops = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        assert_eq!(ops.mlayer_info_idc, Some(3));
        assert!(ops.has_reserved_mlayer_info_idc());
        assert!(matches!(
            ops.payloads[0].xlayer_entries[0].mlayer,
            OpsMlayerSource::Absent
        ));
    }

    #[test]
    fn ops_payload_size_mismatch_is_detected() {
        let (body, body_len) = local_payload_body(0);
        let declared = body_len + 1; // deliberately wrong ops_data_size
        let mut bits = Bits::default();
        bits.bit(0); // reset
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // priority
        bits.f(0, 7); // intent
        bits.bit(0); // intent present
        bits.bit(0); // ptl present
        bits.bit(0); // color present
        bits.f(0, 2); // local reserved
        bits.f(declared, 8); // ops_data_size (wrong)
        bits.append(&body);

        let data = bits.into_bytes();
        let ops = parse(&data, ExtendedLayerId::from_bits(1)).unwrap();
        let payload = &ops.payloads[0];
        assert_eq!(payload.declared_size_bytes, declared);
        assert_eq!(payload.computed_size_bytes, body_len);
        assert!(payload.has_size_mismatch());
    }

    #[test]
    fn ops_ptl_reserved_bits_nonzero_is_detected() {
        // Local OPS, ptl present, with ops_ptl_reserved_2bits != 0.
        let mut body = Bits::default();
        // ops_seq_profile_tier_level_info(): profile, level, tier, mlayer_count, rsvd
        body.f(0, 5); // seq_profile_idc
        body.f(0, 5); // level_idx
        body.bit(0); // tier_flag
        body.f(0, 3); // mlayer_count
        body.f(0b01, 2); // ops_ptl_reserved_2bits = 1 (nonzero)
        body.bit(0); // decoder model present
        body.bit(0); // initial display delay present
        body.f(0, 8); // ops_mlayer_info(): mlayer_map = 0
        body.align();
        let body_len = (body.bit_len() / 8) as u32;

        let mut bits = Bits::default();
        bits.bit(0); // reset
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // priority
        bits.f(0, 7); // intent
        bits.bit(0); // intent present
        bits.bit(1); // ptl present
        bits.bit(0); // color present
        bits.f(0, 2); // local reserved
        bits.f(body_len, 8); // ops_data_size
        bits.append(&body);

        let data = bits.into_bytes();
        let ops = parse(&data, ExtendedLayerId::from_bits(2)).unwrap();
        let entry = &ops.payloads[0].xlayer_entries[0];
        let ptl = entry.ptl_info.as_ref().expect("local ptl info present");
        assert_eq!(ptl.reserved_2bits, 0b01);
        assert_eq!(ptl.target_xlayer_id, ExtendedLayerId::from_bits(2));
    }

    #[test]
    fn ops_inherited_op_index_out_of_range_is_detected() {
        // Global OPS, idc=2, two included layers. Layer 0 carries explicit mlayer
        // info; layer 1 inherits from (ops_id, op_index) with an out-of-range index.
        let mut payload = Bits::default();
        payload.bit(0); // decoder model present
        payload.bit(0); // initial display delay present
        payload.f(0b11, 31); // ops_xlayer_map -> layers 0 and 1
        // layer 0: ops_mlayer_explicit_info_flag = 1 -> explicit mlayer info
        payload.bit(1);
        payload.f(0, 8); // ops_mlayer_map = 0
        // layer 1: ops_mlayer_explicit_info_flag = 0 -> inherited
        payload.bit(0);
        payload.f(0, 4); // ops_embedded_ops_id = 0 (self)
        payload.f(5, 3); // ops_embedded_op_index = 5 (>= ops_cnt and >= j=1)
        payload.align();
        let payload_bytes = (payload.bit_len() / 8) as u32;

        let mut bits = Bits::default();
        bits.bit(0); // reset
        bits.f(0, 4); // ops_id = 0
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // priority
        bits.f(0, 7); // intent
        bits.bit(0); // intent present
        bits.bit(0); // ptl present
        bits.bit(0); // color present
        bits.f(2, 2); // ops_mlayer_info_idc = 2
        bits.f(payload_bytes, 8); // ops_data_size
        bits.append(&payload);

        let data = bits.into_bytes();
        let ops = parse(&data, GLOBAL_XLAYER_ID).unwrap();
        let entries = &ops.payloads[0].xlayer_entries;
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].mlayer, OpsMlayerSource::Explicit(_)));
        match entries[1].mlayer {
            OpsMlayerSource::Inherited {
                embedded_ops_id,
                embedded_op_index,
            } => {
                assert_eq!(embedded_ops_id, 0);
                assert_eq!(embedded_op_index, 5);
                // Reference is to the current OPS (ops_id 0) with op index 5 >=
                // ops_cnt 1, which the validator flags.
                assert!(u32::from(embedded_op_index) >= u32::from(ops.ops_cnt));
            }
            _ => panic!("expected inherited mlayer source"),
        }
    }

    #[test]
    fn ops_local_color_and_decoder_model_info_parse() {
        // Local OPS exercising ops_color_info() (rg + explicit triple),
        // ops_decoder_model_info() (uvlc), initial display delay, and mlayer info.
        let mut body = Bits::default();
        body.rg(0, 2); // ops_color_description_idc = 0 -> explicit triple
        body.f(1, 8); // ops_color_primaries
        body.f(13, 8); // ops_transfer_characteristics
        body.f(6, 8); // ops_matrix_coefficients
        body.bit(1); // ops_full_range_flag
        body.bit(1); // ops_decoder_model_info_for_this_op_present_flag
        body.uvlc(10); // ops_decoder_buffer_delay
        body.uvlc(20); // ops_encoder_buffer_delay
        body.bit(1); // ops_low_delay_mode_flag
        body.bit(1); // ops_initial_display_delay_present_flag
        body.f(3, 4); // ops_initial_display_delay_minus_1
        body.f(0b1, 8); // ops_mlayer_info(): ops_mlayer_map -> embedded layer 0
        body.f(0b101, 4); // ops_tlayer_map for embedded layer 0
        body.align();
        let body_len = (body.bit_len() / 8) as u32;

        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(2, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(1); // ops_color_info_present_flag
        bits.f(0, 2); // ops_reserved_2bits
        bits.f(body_len, 8); // ops_data_size
        bits.append(&body);

        let data = bits.into_bytes();
        let ops = parse(&data, ExtendedLayerId::from_bits(1)).unwrap();
        let payload = &ops.payloads[0];
        assert!(!payload.has_size_mismatch());
        let color = payload.color_info.as_ref().expect("color info present");
        assert_eq!(color.color_description_idc, 0);
        assert_eq!(color.color_primaries, Some(1));
        assert_eq!(color.transfer_characteristics, Some(13));
        assert_eq!(color.matrix_coefficients, Some(6));
        assert!(color.full_range_flag);
        let dm = payload
            .decoder_model_info
            .as_ref()
            .expect("decoder model info present");
        assert_eq!(dm.decoder_buffer_delay, 10);
        assert_eq!(dm.encoder_buffer_delay, 20);
        assert!(dm.low_delay_mode_flag);
        assert_eq!(payload.initial_display_delay_minus_1, Some(3));
        let OpsMlayerSource::Explicit(mlayer) = &payload.xlayer_entries[0].mlayer else {
            panic!("expected explicit local mlayer info");
        };
        assert_eq!(mlayer.mlayer_map, 0b1);
        assert_eq!(mlayer.tlayer_maps, vec![(0, 0b101)]);
    }

    #[test]
    fn truncated_input_is_error_not_panic() {
        // ops_cnt > 0 but no header/payload bytes follow.
        let mut bits = Bits::default();
        bits.bit(0);
        bits.f(0, 4);
        bits.f(1, 3); // ops_cnt = 1, but nothing follows
        let data = bits.into_bytes();
        assert!(parse(&data, ExtendedLayerId::from_bits(0)).is_err());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::types::{ExtendedLayerId, GLOBAL_XLAYER_ID};
    use proptest::prelude::*;

    proptest! {
        /// The OPS parser must never panic on arbitrary input.
        #[test]
        fn operating_point_set_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut global = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_operating_point_set(&mut global, GLOBAL_XLAYER_ID);

            let mut local = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_operating_point_set(&mut local, ExtendedLayerId::from_bits(2));
        }
    }
}
