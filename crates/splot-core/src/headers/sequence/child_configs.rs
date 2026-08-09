// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.4.x sequence-header child config structures and their parsers:
//! `sequence_partition_config()` … `sequence_tile_config()` (§ 5.4.2 – § 5.4.10),
//! the derived `SuperblockSize` / `DrlReorder` / `CdefOnSkipTxfm` modes, and the
//! `read_drl_reorder()` helper.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::segment::{SegmentInfo, parse_seg_info};
use crate::tile::{TileParams, TileParamsInput, parse_tile_layout};

/// `MOTION_MODES`: number of motion modes (AV2 § 3).
const MOTION_MODES: usize = 5;
/// `INTERINTRA`: first signalled motion-mode index (AV2 § 3).
const INTERINTRA: usize = 1;
/// `DELTAWARP`: delta-warp motion-mode index (AV2 § 3).
const DELTAWARP: usize = 3;
/// `MAX_REF_MV_STACK_SIZE` (AV2 § 3); `ns(MAX_REF_MV_STACK_SIZE - 1)` width.
const MAX_REF_MV_STACK_SIZE: u32 = 6;
/// `MAX_REF_BV_STACK_SIZE` (AV2 § 3); `ns(MAX_REF_BV_STACK_SIZE - 1)` width.
const MAX_REF_BV_STACK_SIZE: u32 = 4;
/// `SELECT_SCREEN_CONTENT_TOOLS` (AV2 § 3).
const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;
/// `SELECT_INTEGER_MV` (AV2 § 3).
const SELECT_INTEGER_MV: u8 = 2;

/// Sequence superblock size, derived per `get_seq_sb_size()` (AV2 § 5.18.7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperblockSize {
    /// `BLOCK_64X64`.
    Block64x64,
    /// `BLOCK_128X128`.
    Block128x128,
    /// `BLOCK_256X256`.
    Block256x256,
}

/// `sequence_partition_config()` (AV2 v1.0.0 § 5.4.3 / § 6.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePartitionConfig {
    /// `use_256x256_superblock`.
    pub use_256x256_superblock: bool,
    /// `use_128x128_superblock` (inferred `false` when 256×256 superblocks are used).
    pub use_128x128_superblock: bool,
    /// `enable_sdp` (inferred `0` for monochrome).
    pub enable_sdp: bool,
    /// `enable_extended_sdp` (inferred `0` unless SDP is enabled and not a single picture).
    pub enable_extended_sdp: bool,
    /// `enable_ext_partitions`.
    pub enable_ext_partitions: bool,
    /// `enable_uneven_4way_partitions` (inferred `0` unless extended partitions are enabled).
    pub enable_uneven_4way_partitions: bool,
    /// `reduce_pb_aspect_ratio`.
    pub reduce_pb_aspect_ratio: bool,
    /// `MaxPbAspectRatio` (inferred `8` unless `reduce_pb_aspect_ratio`).
    pub max_pb_aspect_ratio: u32,
}

impl SequencePartitionConfig {
    /// Returns `get_seq_sb_size()` (AV2 § 5.18.7.6).
    #[must_use]
    pub const fn seq_sb_size(&self) -> SuperblockSize {
        if self.use_256x256_superblock {
            SuperblockSize::Block256x256
        } else if self.use_128x128_superblock {
            SuperblockSize::Block128x128
        } else {
            SuperblockSize::Block64x64
        }
    }
}

/// Parses `sequence_partition_config()` (AV2 v1.0.0 § 5.4.3).
///
/// `monochrome` is `Monochrome` and `single_picture` is `single_picture_header_flag`
/// from the general header; both gate conditional fields.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_partition_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequencePartitionConfig> {
    let use_256x256_superblock = reader.read_flag()?;
    let use_128x128_superblock = if use_256x256_superblock {
        false
    } else {
        reader.read_flag()?
    };
    let enable_sdp = if monochrome {
        false
    } else {
        reader.read_flag()?
    };
    let enable_extended_sdp = if enable_sdp && !single_picture {
        reader.read_flag()?
    } else {
        false
    };
    let enable_ext_partitions = reader.read_flag()?;
    let enable_uneven_4way_partitions = if enable_ext_partitions {
        reader.read_flag()?
    } else {
        false
    };
    let reduce_pb_aspect_ratio = reader.read_flag()?;
    let max_pb_aspect_ratio = if reduce_pb_aspect_ratio {
        let max_pb_aspect_ratio_log2_minus_1 = reader.read_bits_u8(1)?;
        1u32 << (u32::from(max_pb_aspect_ratio_log2_minus_1) + 1)
    } else {
        8
    };

    Ok(SequencePartitionConfig {
        use_256x256_superblock,
        use_128x128_superblock,
        enable_sdp,
        enable_extended_sdp,
        enable_ext_partitions,
        enable_uneven_4way_partitions,
        reduce_pb_aspect_ratio,
        max_pb_aspect_ratio,
    })
}

/// `sequence_segment_config()` (AV2 v1.0.0 § 5.4.4 / § 6.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSegmentConfig {
    /// `enable_ext_seg`.
    pub enable_ext_seg: bool,
    /// `MaxSegments` (`16` when extended segmentation is enabled, else `8`).
    pub max_segments: u8,
    /// `seq_seg_info_present_flag`.
    pub seq_seg_info_present_flag: bool,
    /// `seq_allow_seg_info_change`, present when segment info is signalled.
    pub seq_allow_seg_info_change: Option<bool>,
    /// Parsed `seg_info(MaxSegments)` (§ 5.4.9), present when segment info is signalled.
    pub segment_info: Option<SegmentInfo>,
}

/// Parses `sequence_segment_config()` (AV2 § 5.4.4), including `seg_info(MaxSegments)`
/// (§ 5.4.9) when `seq_seg_info_present_flag` is set.
///
/// # Errors
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field,
/// and [`Error::InvalidTileParams`] when a non-uniform layout exceeds the § 6.17.7.2
/// tile-count limits.
pub fn parse_sequence_segment_config(reader: &mut BitReader<'_>) -> Result<SequenceSegmentConfig> {
    let enable_ext_seg = reader.read_flag()?;
    let max_segments = if enable_ext_seg { 16 } else { 8 };
    let seq_seg_info_present_flag = reader.read_flag()?;
    let (seq_allow_seg_info_change, segment_info) = if seq_seg_info_present_flag {
        let allow = reader.read_flag()?;
        let info = parse_seg_info(reader, max_segments)?;
        (Some(allow), Some(info))
    } else {
        (None, None)
    };

    Ok(SequenceSegmentConfig {
        enable_ext_seg,
        max_segments,
        seq_seg_info_present_flag,
        seq_allow_seg_info_change,
        segment_info,
    })
}

/// `sequence_intra_config()` (AV2 v1.0.0 § 5.4.5 / § 6.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceIntraConfig {
    /// `enable_dip`.
    pub enable_dip: bool,
    /// `enable_intra_edge_filter`.
    pub enable_intra_edge_filter: bool,
    /// `enable_mrls`.
    pub enable_mrls: bool,
    /// `enable_cfl_intra`.
    pub enable_cfl_intra: bool,
    /// `cfl_ds_filter_index` (inferred `0` for monochrome).
    pub cfl_ds_filter_index: u8,
    /// `enable_mhccp`.
    pub enable_mhccp: bool,
    /// `enable_ibp`.
    pub enable_ibp: bool,
}

/// Parses `sequence_intra_config()` (AV2 v1.0.0 § 5.4.5).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_intra_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
) -> Result<SequenceIntraConfig> {
    let enable_dip = reader.read_flag()?;
    let enable_intra_edge_filter = reader.read_flag()?;
    let enable_mrls = reader.read_flag()?;
    let enable_cfl_intra = reader.read_flag()?;
    let cfl_ds_filter_index = if monochrome {
        0
    } else {
        reader.read_bits_u8(2)?
    };
    let enable_mhccp = reader.read_flag()?;
    let enable_ibp = reader.read_flag()?;

    Ok(SequenceIntraConfig {
        enable_dip,
        enable_intra_edge_filter,
        enable_mrls,
        enable_cfl_intra,
        cfl_ds_filter_index,
        enable_mhccp,
        enable_ibp,
    })
}

/// DRL reordering mode derived from `disable_drl_reorder` / `constrain_drl_reorder` (AV2 § 5.4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrlReorder {
    /// `DRL_REORDER_DISABLED`.
    Disabled,
    /// `DRL_REORDER_CONSTRAINT`.
    Constraint,
    /// `DRL_REORDER_ALWAYS`.
    Always,
}

/// `sequence_inter_config()` (AV2 v1.0.0 § 5.4.6 / § 6.4.6).
///
/// Fields not read on the `single_picture_header_flag` branch carry the spec's
/// inferred values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceInterConfig {
    /// `seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]`; index `0` (`SIMPLE`) is always `0`.
    pub seq_enabled_motion_modes: [bool; MOTION_MODES],
    /// `seq_frame_motion_modes_present_flag`.
    pub seq_frame_motion_modes_present_flag: bool,
    /// `enable_six_param_warp_delta`.
    pub enable_six_param_warp_delta: bool,
    /// `enable_masked_compound`.
    pub enable_masked_compound: bool,
    /// `enable_ref_frame_mvs`.
    pub enable_ref_frame_mvs: bool,
    /// `reduced_ref_frame_mvs_mode`.
    pub reduced_ref_frame_mvs_mode: bool,
    /// `OrderHintBits`.
    pub order_hint_bits: u8,
    /// `enable_refmvbank`.
    pub enable_refmvbank: bool,
    /// `DrlReorder`.
    pub drl_reorder: DrlReorder,
    /// `explicit_ref_frame_map`.
    pub explicit_ref_frame_map: bool,
    /// `NumRefFrames`.
    pub num_ref_frames: u8,
    /// `long_term_frame_id_bits`.
    pub long_term_frame_id_bits: u8,
    /// `seq_max_drl_bits_minus_1` (inferred `0` for single pictures).
    pub seq_max_drl_bits_minus_1: u32,
    /// `allow_frame_max_drl_bits` (inferred `0` for single pictures).
    pub allow_frame_max_drl_bits: bool,
    /// `seq_max_bvp_drl_bits_minus_1`.
    pub seq_max_bvp_drl_bits_minus_1: u32,
    /// `allow_frame_max_bvp_drl_bits`.
    pub allow_frame_max_bvp_drl_bits: bool,
    /// `num_same_ref_compound` (inferred `0` for single pictures).
    pub num_same_ref_compound: u8,
    /// `enable_tip`.
    pub enable_tip: bool,
    /// `EnableTipOutput`.
    pub enable_tip_output: bool,
    /// `enable_tip_hole_fill`.
    pub enable_tip_hole_fill: bool,
    /// `enable_mv_traj`.
    pub enable_mv_traj: bool,
    /// `enable_bawp`.
    pub enable_bawp: bool,
    /// `enable_cwp` (inferred `0` for single pictures).
    pub enable_cwp: bool,
    /// `enable_imp_msk_bld`.
    pub enable_imp_msk_bld: bool,
    /// `enable_df_sub_pu` (inferred `0` for single pictures).
    pub enable_df_sub_pu: bool,
    /// `enable_tip_explicit_qp`.
    pub enable_tip_explicit_qp: bool,
    /// `enable_opfl_refine` (`REFINE_NONE` = `0` for single pictures).
    pub enable_opfl_refine: u8,
    /// `enable_refinemv` (inferred `0` for single pictures).
    pub enable_refinemv: bool,
    /// `enable_tip_refinemv`.
    pub enable_tip_refinemv: bool,
    /// `enable_bru` (inferred `0` for single pictures).
    pub enable_bru: bool,
    /// `enable_adaptive_mvd` (inferred `0` for single pictures).
    pub enable_adaptive_mvd: bool,
    /// `enable_mvd_sign_derive` (inferred `0` for single pictures).
    pub enable_mvd_sign_derive: bool,
    /// `enable_flex_mvres` (inferred `0` for single pictures).
    pub enable_flex_mvres: bool,
    /// `enable_global_motion`.
    pub enable_global_motion: bool,
    /// `enable_short_refresh_frame_flags` (inferred `0` for single pictures).
    pub enable_short_refresh_frame_flags: bool,
}

fn read_drl_reorder(reader: &mut BitReader<'_>) -> Result<DrlReorder> {
    let disable_drl_reorder = reader.read_flag()?;
    if disable_drl_reorder {
        Ok(DrlReorder::Disabled)
    } else {
        let constrain_drl_reorder = reader.read_flag()?;
        Ok(if constrain_drl_reorder {
            DrlReorder::Constraint
        } else {
            DrlReorder::Always
        })
    }
}

/// Parses `sequence_inter_config()` (AV2 v1.0.0 § 5.4.6).
///
/// This reads sequence-level inter tool flags only; it does not model motion
/// estimation, reference management, or any decoding process.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] or descriptor errors if the payload is malformed.
pub fn parse_sequence_inter_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
) -> Result<SequenceInterConfig> {
    let mut config = SequenceInterConfig {
        seq_enabled_motion_modes: [false; MOTION_MODES],
        seq_frame_motion_modes_present_flag: false,
        enable_six_param_warp_delta: false,
        enable_masked_compound: false,
        enable_ref_frame_mvs: false,
        reduced_ref_frame_mvs_mode: false,
        order_hint_bits: 0,
        enable_refmvbank: false,
        drl_reorder: DrlReorder::Disabled,
        explicit_ref_frame_map: false,
        num_ref_frames: 2,
        long_term_frame_id_bits: 0,
        seq_max_drl_bits_minus_1: 0,
        allow_frame_max_drl_bits: false,
        seq_max_bvp_drl_bits_minus_1: 0,
        allow_frame_max_bvp_drl_bits: false,
        num_same_ref_compound: 0,
        enable_tip: false,
        enable_tip_output: false,
        enable_tip_hole_fill: false,
        enable_mv_traj: false,
        enable_bawp: false,
        enable_cwp: false,
        enable_imp_msk_bld: false,
        enable_df_sub_pu: false,
        enable_tip_explicit_qp: false,
        enable_opfl_refine: 0,
        enable_refinemv: false,
        enable_tip_refinemv: false,
        enable_bru: false,
        enable_adaptive_mvd: false,
        enable_mvd_sign_derive: false,
        enable_flex_mvres: false,
        enable_global_motion: false,
        enable_short_refresh_frame_flags: false,
    };

    if single_picture {
        config.enable_refmvbank = reader.read_flag()?;
        config.drl_reorder = read_drl_reorder(reader)?;
        config.seq_max_bvp_drl_bits_minus_1 = reader.read_ns(MAX_REF_BV_STACK_SIZE - 1)?;
        config.allow_frame_max_bvp_drl_bits = reader.read_flag()?;
        config.enable_bawp = reader.read_flag()?;
        return Ok(config);
    }

    let mut motion_mode_enabled = false;
    for mode in INTERINTRA..MOTION_MODES {
        let enabled = reader.read_flag()?;
        config.seq_enabled_motion_modes[mode] = enabled;
        motion_mode_enabled |= enabled;
    }
    config.seq_frame_motion_modes_present_flag = if motion_mode_enabled {
        reader.read_flag()?
    } else {
        false
    };
    config.enable_six_param_warp_delta = if config.seq_enabled_motion_modes[DELTAWARP] {
        reader.read_flag()?
    } else {
        false
    };
    config.enable_masked_compound = reader.read_flag()?;
    config.enable_ref_frame_mvs = reader.read_flag()?;
    config.reduced_ref_frame_mvs_mode = if config.enable_ref_frame_mvs {
        reader.read_flag()?
    } else {
        false
    };
    let order_hint_bits_minus_1 = reader.read_bits_u8(4)?;
    config.order_hint_bits = order_hint_bits_minus_1 + 1;
    config.enable_refmvbank = reader.read_flag()?;
    config.drl_reorder = read_drl_reorder(reader)?;
    config.explicit_ref_frame_map = reader.read_flag()?;
    let explicit_num_ref_frames = reader.read_flag()?;
    config.num_ref_frames = if explicit_num_ref_frames {
        reader.read_bits_u8(4)? + 1
    } else {
        8
    };
    config.long_term_frame_id_bits = reader.read_bits_u8(3)?;
    config.seq_max_drl_bits_minus_1 = reader.read_ns(MAX_REF_MV_STACK_SIZE - 1)?;
    config.allow_frame_max_drl_bits = reader.read_flag()?;
    config.seq_max_bvp_drl_bits_minus_1 = reader.read_ns(MAX_REF_BV_STACK_SIZE - 1)?;
    config.allow_frame_max_bvp_drl_bits = reader.read_flag()?;
    config.num_same_ref_compound = reader.read_bits_u8(2)?;
    config.enable_tip = reader.read_flag()?;
    if config.enable_tip {
        let disable_tip_output = reader.read_flag()?;
        config.enable_tip_output = !disable_tip_output;
        config.enable_tip_hole_fill = reader.read_flag()?;
    }
    config.enable_mv_traj = reader.read_flag()?;
    config.enable_bawp = reader.read_flag()?;
    config.enable_cwp = reader.read_flag()?;
    config.enable_imp_msk_bld = reader.read_flag()?;
    config.enable_df_sub_pu = reader.read_flag()?;
    config.enable_tip_explicit_qp = if config.enable_tip_output && config.enable_df_sub_pu {
        reader.read_flag()?
    } else {
        false
    };
    config.enable_opfl_refine = reader.read_bits_u8(2)?;
    config.enable_refinemv = reader.read_flag()?;
    config.enable_tip_refinemv =
        if config.enable_tip && (config.enable_opfl_refine != 0 || config.enable_refinemv) {
            reader.read_flag()?
        } else {
            false
        };
    config.enable_bru = reader.read_flag()?;
    config.enable_adaptive_mvd = reader.read_flag()?;
    config.enable_mvd_sign_derive = reader.read_flag()?;
    config.enable_flex_mvres = reader.read_flag()?;
    config.enable_global_motion = reader.read_flag()?;
    config.enable_short_refresh_frame_flags = reader.read_flag()?;

    Ok(config)
}

/// `sequence_scc_config()` (AV2 v1.0.0 § 5.4.7 / § 6.4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSccConfig {
    /// `seq_force_screen_content_tools` (`SELECT_SCREEN_CONTENT_TOOLS` = 2 when chosen).
    pub seq_force_screen_content_tools: u8,
    /// `seq_force_integer_mv` (`SELECT_INTEGER_MV` = 2 when chosen).
    pub seq_force_integer_mv: u8,
}

/// Parses `sequence_scc_config()` (AV2 v1.0.0 § 5.4.7).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_scc_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
) -> Result<SequenceSccConfig> {
    if single_picture {
        return Ok(SequenceSccConfig {
            seq_force_screen_content_tools: SELECT_SCREEN_CONTENT_TOOLS,
            seq_force_integer_mv: SELECT_INTEGER_MV,
        });
    }

    let seq_choose_screen_content_tools = reader.read_flag()?;
    let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
        SELECT_SCREEN_CONTENT_TOOLS
    } else {
        reader.read_bits_u8(1)?
    };

    let seq_force_integer_mv = if seq_force_screen_content_tools > 0 {
        let seq_choose_integer_mv = reader.read_flag()?;
        if seq_choose_integer_mv {
            SELECT_INTEGER_MV
        } else {
            reader.read_bits_u8(1)?
        }
    } else {
        SELECT_INTEGER_MV
    };

    Ok(SequenceSccConfig {
        seq_force_screen_content_tools,
        seq_force_integer_mv,
    })
}

/// `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.8 / § 6.4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceTqEntropyConfig {
    /// `enable_fsc`.
    pub enable_fsc: bool,
    /// `enable_idtx_intra` (inferred `1` when `enable_fsc`).
    pub enable_idtx_intra: bool,
    /// `enable_intra_ist`.
    pub enable_intra_ist: bool,
    /// `enable_inter_ist`.
    pub enable_inter_ist: bool,
    /// `enable_chroma_dctonly` (inferred `0` for monochrome).
    pub enable_chroma_dctonly: bool,
    /// `enable_inter_ddt` (only signalled for non-single-picture headers).
    pub enable_inter_ddt: bool,
    /// `reduced_tx_part_set`.
    pub reduced_tx_part_set: bool,
    /// `enable_cctx` (inferred `0` for monochrome).
    pub enable_cctx: bool,
    /// `enable_tcq`.
    pub enable_tcq: bool,
    /// `choose_tcq_per_frame`.
    pub choose_tcq_per_frame: bool,
    /// `enable_parity_hiding`.
    pub enable_parity_hiding: bool,
    /// `enable_avg_cdf` (inferred `1` for single pictures).
    pub enable_avg_cdf: bool,
    /// `avg_cdf_type` (inferred `1` for single pictures, `0` when averaging disabled).
    pub avg_cdf_type: u8,
    /// `separate_uv_delta_q` (inferred `0` for monochrome).
    pub separate_uv_delta_q: bool,
    /// `equal_ac_dc_q`.
    pub equal_ac_dc_q: bool,
    /// `base_y_dc_delta_q` (raw 5-bit field, only present when `!equal_ac_dc_q`).
    pub base_y_dc_delta_q: u8,
    /// `y_dc_delta_q_enabled`.
    pub y_dc_delta_q_enabled: bool,
    /// `base_uv_dc_delta_q` (chroma only; the raw 5-bit field when `!equal_ac_dc_q`,
    /// otherwise mirrored from `base_uv_ac_delta_q` per AV2 § 5.4.8).
    pub base_uv_dc_delta_q: u8,
    /// `uv_dc_delta_q_enabled`.
    pub uv_dc_delta_q_enabled: bool,
    /// `base_uv_ac_delta_q` (raw 5-bit field, chroma only).
    pub base_uv_ac_delta_q: u8,
    /// `uv_ac_delta_q_enabled`.
    pub uv_ac_delta_q_enabled: bool,
}

/// Parses `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.8).
///
/// Only sequence-level transform/quant/entropy tool flags are read; no transform,
/// quantizer, or entropy coder is implemented.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_transform_quant_entropy_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequenceTqEntropyConfig> {
    let enable_fsc = reader.read_flag()?;
    let enable_idtx_intra = if enable_fsc {
        true
    } else {
        reader.read_flag()?
    };
    let enable_intra_ist = reader.read_flag()?;
    let enable_inter_ist = reader.read_flag()?;
    let enable_chroma_dctonly = if monochrome {
        false
    } else {
        reader.read_flag()?
    };
    let enable_inter_ddt = if single_picture {
        false
    } else {
        reader.read_flag()?
    };
    let reduced_tx_part_set = reader.read_flag()?;
    let enable_cctx = if monochrome {
        false
    } else {
        reader.read_flag()?
    };
    let enable_tcq = reader.read_flag()?;
    let choose_tcq_per_frame = if enable_tcq && !single_picture {
        reader.read_flag()?
    } else {
        false
    };
    let enable_parity_hiding = if enable_tcq && !choose_tcq_per_frame {
        false
    } else {
        reader.read_flag()?
    };
    let (enable_avg_cdf, avg_cdf_type) = if single_picture {
        (true, 1)
    } else {
        let enable_avg_cdf = reader.read_flag()?;
        let avg_cdf_type = if enable_avg_cdf {
            reader.read_bits_u8(1)?
        } else {
            0
        };
        (enable_avg_cdf, avg_cdf_type)
    };
    let separate_uv_delta_q = if monochrome {
        false
    } else {
        reader.read_flag()?
    };

    let equal_ac_dc_q = reader.read_flag()?;
    let mut base_y_dc_delta_q = 0;
    let mut y_dc_delta_q_enabled = false;
    if !equal_ac_dc_q {
        base_y_dc_delta_q = reader.read_bits_u8(5)?;
        y_dc_delta_q_enabled = reader.read_flag()?;
    }
    let mut base_uv_dc_delta_q = 0;
    let mut uv_dc_delta_q_enabled = false;
    let mut base_uv_ac_delta_q = 0;
    let mut uv_ac_delta_q_enabled = false;
    if !monochrome {
        if !equal_ac_dc_q {
            base_uv_dc_delta_q = reader.read_bits_u8(5)?;
            uv_dc_delta_q_enabled = reader.read_flag()?;
        }
        base_uv_ac_delta_q = reader.read_bits_u8(5)?;
        uv_ac_delta_q_enabled = reader.read_flag()?;
        if equal_ac_dc_q {
            base_uv_dc_delta_q = base_uv_ac_delta_q;
        }
    }

    Ok(SequenceTqEntropyConfig {
        enable_fsc,
        enable_idtx_intra,
        enable_intra_ist,
        enable_inter_ist,
        enable_chroma_dctonly,
        enable_inter_ddt,
        reduced_tx_part_set,
        enable_cctx,
        enable_tcq,
        choose_tcq_per_frame,
        enable_parity_hiding,
        enable_avg_cdf,
        avg_cdf_type,
        separate_uv_delta_q,
        equal_ac_dc_q,
        base_y_dc_delta_q,
        y_dc_delta_q_enabled,
        base_uv_dc_delta_q,
        uv_dc_delta_q_enabled,
        base_uv_ac_delta_q,
        uv_ac_delta_q_enabled,
    })
}

/// `CdefOnSkipTxfm` mode derived in `sequence_filter_config()` (AV2 § 5.4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdefOnSkipTxfm {
    /// `CDEF_ON_SKIP_TXFM_ADAPTIVE`.
    Adaptive,
    /// `CDEF_ON_SKIP_TXFM_ALWAYS_ON`.
    AlwaysOn,
    /// `CDEF_ON_SKIP_TXFM_DISABLED`.
    Disabled,
}

/// `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10 / § 6.4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceFilterConfig {
    /// `disable_loopfilters_across_tiles`.
    pub disable_loopfilters_across_tiles: bool,
    /// `enable_cdef`.
    pub enable_cdef: bool,
    /// `enable_gdf`.
    pub enable_gdf: bool,
    /// `gdf_unit_matches_sb_size` (only signalled when GDF is on and superblocks are 64×64).
    pub gdf_unit_matches_sb_size: bool,
    /// `enable_restoration`.
    pub enable_restoration: bool,
    /// `lr_tools_disable[0][RESTORE_PC_WIENER]`.
    pub lr_pc_wiener_disabled: bool,
    /// `lr_tools_disable[0][RESTORE_WIENER_NONSEP]`.
    pub lr_wiener_nonsep_disabled: bool,
    /// `lr_tools_disable[1][RESTORE_PC_WIENER]` (inferred `1`/`true` when restoration is enabled).
    pub lr_uv_pc_wiener_disabled: bool,
    /// `lr_tools_uv_present`.
    pub lr_tools_uv_present: bool,
    /// `lr_tools_disable[1][RESTORE_WIENER_NONSEP]`.
    pub lr_uv_wiener_nonsep_disabled: bool,
    /// `enable_ccso`.
    pub enable_ccso: bool,
    /// `ccso_unit_matches_sb_size`.
    pub ccso_unit_matches_sb_size: bool,
    /// `CdefOnSkipTxfm`.
    pub cdef_on_skip_txfm: CdefOnSkipTxfm,
    /// `df_par_bits_minus_2`.
    pub df_par_bits_minus_2: u8,
}

/// Parses `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10).
///
/// `seq_sb_size` is `get_seq_sb_size()` from the partition config; it gates
/// `gdf_unit_matches_sb_size`. Only sequence-level filter tool flags are read.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_filter_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
    seq_sb_size: SuperblockSize,
) -> Result<SequenceFilterConfig> {
    let disable_loopfilters_across_tiles = reader.read_flag()?;
    let enable_cdef = reader.read_flag()?;
    let enable_gdf = reader.read_flag()?;
    let gdf_unit_matches_sb_size = if enable_gdf && seq_sb_size == SuperblockSize::Block64x64 {
        reader.read_flag()?
    } else {
        false
    };
    let enable_restoration = reader.read_flag()?;
    let mut lr_pc_wiener_disabled = false;
    let mut lr_wiener_nonsep_disabled = false;
    let mut lr_tools_uv_present = false;
    let mut lr_uv_wiener_nonsep_disabled = false;
    let lr_uv_pc_wiener_disabled = enable_restoration;
    if enable_restoration {
        lr_pc_wiener_disabled = reader.read_flag()?;
        lr_wiener_nonsep_disabled = reader.read_flag()?;
        lr_tools_uv_present = reader.read_flag()?;
        lr_uv_wiener_nonsep_disabled = if lr_tools_uv_present {
            reader.read_flag()?
        } else {
            lr_wiener_nonsep_disabled
        };
    }
    let enable_ccso = reader.read_flag()?;
    let ccso_unit_matches_sb_size = if enable_ccso {
        reader.read_flag()?
    } else {
        false
    };
    let cdef_on_skip_txfm = if single_picture {
        CdefOnSkipTxfm::Adaptive
    } else {
        let cdef_on_skip_txfm_always_on = reader.read_flag()?;
        if cdef_on_skip_txfm_always_on {
            CdefOnSkipTxfm::AlwaysOn
        } else {
            let cdef_on_skip_txfm_disabled = reader.read_flag()?;
            if cdef_on_skip_txfm_disabled {
                CdefOnSkipTxfm::Disabled
            } else {
                CdefOnSkipTxfm::Adaptive
            }
        }
    };
    let df_par_bits_minus_2 = reader.read_bits_u8(2)?;

    Ok(SequenceFilterConfig {
        disable_loopfilters_across_tiles,
        enable_cdef,
        enable_gdf,
        gdf_unit_matches_sb_size,
        enable_restoration,
        lr_pc_wiener_disabled,
        lr_wiener_nonsep_disabled,
        lr_uv_pc_wiener_disabled,
        lr_tools_uv_present,
        lr_uv_wiener_nonsep_disabled,
        enable_ccso,
        ccso_unit_matches_sb_size,
        cdef_on_skip_txfm,
        df_par_bits_minus_2,
    })
}

/// `sequence_tile_config()` (AV2 v1.0.0 § 5.4.2 / § 6.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceTileConfig {
    /// `seq_tile_info_present_flag`.
    pub seq_tile_info_present_flag: bool,
    /// `allow_tile_info_change`, present when tile info is signalled.
    pub allow_tile_info_change: Option<bool>,
    /// Parsed `tile_params()` (§ 5.18.7.3), present when tile info is signalled and the
    /// sequence level is not reserved.
    pub params: Option<TileParams>,
    /// `SeqSbColStarts[0..SeqTileCols]` (AV2 § 5.4.2: the `sbColStarts` returned by the
    /// `tile_params()` call at mirror
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2` :654-656). Recorded at parse
    /// time so the frame § 5.18.7.4 non-uniform `reuse_tile_params()` branch can rebuild
    /// the layout. Empty when tile info is not signalled or the level is reserved (no
    /// parsed [`Self::params`]). Bounded by `MAX_TILE_COLS`.
    pub seq_sb_col_starts: Vec<u32>,
    /// `SeqSbRowStarts[0..SeqTileRows]` (AV2 § 5.4.2; the `sbRowStarts` companion of
    /// [`Self::seq_sb_col_starts`]). Bounded by `MAX_TILE_ROWS`.
    pub seq_sb_row_starts: Vec<u32>,
}

impl SequenceTileConfig {
    /// Returns the owning Feature ID if tile info is present but `tile_params()` could
    /// not be parsed because `seq_level_idx` is a reserved (non-conformant) level with
    /// no defined tile bit layout. `None` for any valid sequence header.
    #[must_use]
    pub const fn unimplemented_at(&self) -> Option<&'static str> {
        if self.seq_tile_info_present_flag && self.params.is_none() {
            Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG")
        } else {
            None
        }
    }
}

/// Parses `sequence_tile_config()` (AV2 § 5.4.2), including `tile_params()`
/// (§ 5.18.7.3) when `seq_tile_info_present_flag` is set.
///
/// `tile_params_input` carries the frame dimensions, superblock size, tier, and level
/// that `tile_params()` needs; the caller builds it from the parsed general header and
/// partition config (`is_bridge` is `false` at the sequence call site). A reserved
/// `seq_level_idx` has no defined tile bit layout, so the params are left `None` (a
/// bounded residual reported by [`SequenceTileConfig::unimplemented_at`]).
///
/// The `tile_params()` call at AV2 § 5.4.2 returns `SeqSbColStarts` / `SeqSbRowStarts`
/// (mirror `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2` :654-656); these are
/// retained on [`SequenceTileConfig`] for the frame § 5.18.7.4 non-uniform
/// `reuse_tile_params()` branch.
///
/// # Errors
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_tile_config(
    reader: &mut BitReader<'_>,
    tile_params_input: TileParamsInput,
) -> Result<SequenceTileConfig> {
    let seq_tile_info_present_flag = reader.read_flag()?;
    if !seq_tile_info_present_flag {
        return Ok(SequenceTileConfig {
            seq_tile_info_present_flag: false,
            allow_tile_info_change: None,
            params: None,
            seq_sb_col_starts: Vec::new(),
            seq_sb_row_starts: Vec::new(),
        });
    }

    let allow_tile_info_change = reader.read_flag()?;
    let (params, seq_sb_col_starts, seq_sb_row_starts) =
        match parse_tile_layout(reader, tile_params_input) {
            Ok(layout) => (
                Some(layout.params),
                layout.sb_col_starts,
                layout.sb_row_starts,
            ),
            Err(Error::Unimplemented { .. }) => (None, Vec::new(), Vec::new()),
            Err(error) => return Err(error),
        };

    Ok(SequenceTileConfig {
        seq_tile_info_present_flag: true,
        allow_tile_info_change: Some(allow_tile_info_change),
        params,
        seq_sb_col_starts,
        seq_sb_row_starts,
    })
}
