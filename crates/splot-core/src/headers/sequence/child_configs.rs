// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.4.x sequence-header child config structures and their parsers:
//! `sequence_partition_config()` … `sequence_tile_config()` (§ 5.4.2 – § 5.4.10),
//! the derived `SuperblockSize` / `DrlReorder` / `CdefOnSkipTxfm` modes, and the
//! `read_drl_reorder()` helper.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::segment::{SegmentInfo, parse_seg_info_with_reader};
use crate::syntax::{NoopObserver, SyntaxObserver, SyntaxReader};
use crate::tile::{TileParams, TileParamsInput, parse_tile_layout_with_reader};

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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_partition_config_with_reader(&mut sr, monochrome, single_picture)
}

pub(crate) fn parse_sequence_partition_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequencePartitionConfig> {
    sr.in_struct("sequence_partition_config", Some("5.4.3"), |sr| {
        let use_256x256_superblock = sr.read_flag("use_256x256_superblock")?;
        let use_128x128_superblock = if use_256x256_superblock {
            sr.inferred_bool("use_128x128_superblock", "use_256x256_superblock", false);
            false
        } else {
            sr.read_flag("use_128x128_superblock")?
        };
        let enable_sdp = if monochrome {
            sr.inferred_bool("enable_sdp", "monochrome", false);
            false
        } else {
            sr.read_flag("enable_sdp")?
        };
        let enable_extended_sdp = if enable_sdp && !single_picture {
            sr.read_flag("enable_extended_sdp")?
        } else {
            sr.inferred_bool(
                "enable_extended_sdp",
                "!(enable_sdp && !single_picture_header_flag)",
                false,
            );
            false
        };
        let enable_ext_partitions = sr.read_flag("enable_ext_partitions")?;
        let enable_uneven_4way_partitions = if enable_ext_partitions {
            sr.read_flag("enable_uneven_4way_partitions")?
        } else {
            sr.inferred_bool(
                "enable_uneven_4way_partitions",
                "!enable_ext_partitions",
                false,
            );
            false
        };
        let reduce_pb_aspect_ratio = sr.read_flag("reduce_pb_aspect_ratio")?;
        let max_pb_aspect_ratio = if reduce_pb_aspect_ratio {
            let max_pb_aspect_ratio_log2_minus_1 =
                sr.read_bits_u8("max_pb_aspect_ratio_log2_minus_1", 1)?;
            1u32 << (u32::from(max_pb_aspect_ratio_log2_minus_1) + 1)
        } else {
            sr.absent(
                "max_pb_aspect_ratio_log2_minus_1",
                "!reduce_pb_aspect_ratio",
            );
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
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_segment_config(reader: &mut BitReader<'_>) -> Result<SequenceSegmentConfig> {
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_segment_config_with_reader(&mut sr)
}

pub(crate) fn parse_sequence_segment_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
) -> Result<SequenceSegmentConfig> {
    sr.in_struct("sequence_segment_config", Some("5.4.4"), |sr| {
        let enable_ext_seg = sr.read_flag("enable_ext_seg")?;
        let max_segments = if enable_ext_seg { 16 } else { 8 };
        let seq_seg_info_present_flag = sr.read_flag("seq_seg_info_present_flag")?;
        let (seq_allow_seg_info_change, segment_info) = if seq_seg_info_present_flag {
            let allow = sr.read_flag("seq_allow_seg_info_change")?;
            let info = parse_seg_info_with_reader(sr, max_segments)?;
            (Some(allow), Some(info))
        } else {
            sr.absent("seq_allow_seg_info_change", "!seq_seg_info_present_flag");
            (None, None)
        };

        Ok(SequenceSegmentConfig {
            enable_ext_seg,
            max_segments,
            seq_seg_info_present_flag,
            seq_allow_seg_info_change,
            segment_info,
        })
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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_intra_config_with_reader(&mut sr, monochrome)
}

pub(crate) fn parse_sequence_intra_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    monochrome: bool,
) -> Result<SequenceIntraConfig> {
    sr.in_struct("sequence_intra_config", Some("5.4.5"), |sr| {
        let enable_dip = sr.read_flag("enable_dip")?;
        let enable_intra_edge_filter = sr.read_flag("enable_intra_edge_filter")?;
        let enable_mrls = sr.read_flag("enable_mrls")?;
        let enable_cfl_intra = sr.read_flag("enable_cfl_intra")?;
        let cfl_ds_filter_index = if monochrome {
            sr.inferred_u64("cfl_ds_filter_index", "monochrome", 0);
            0
        } else {
            sr.read_bits_u8("cfl_ds_filter_index", 2)?
        };
        let enable_mhccp = sr.read_flag("enable_mhccp")?;
        let enable_ibp = sr.read_flag("enable_ibp")?;

        Ok(SequenceIntraConfig {
            enable_dip,
            enable_intra_edge_filter,
            enable_mrls,
            enable_cfl_intra,
            cfl_ds_filter_index,
            enable_mhccp,
            enable_ibp,
        })
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

fn read_drl_reorder<O: SyntaxObserver>(sr: &mut SyntaxReader<'_, '_, O>) -> Result<DrlReorder> {
    let disable_drl_reorder = sr.read_flag("disable_drl_reorder")?;
    if disable_drl_reorder {
        sr.absent("constrain_drl_reorder", "disable_drl_reorder");
        Ok(DrlReorder::Disabled)
    } else {
        let constrain_drl_reorder = sr.read_flag("constrain_drl_reorder")?;
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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_inter_config_with_reader(&mut sr, single_picture)
}

pub(crate) fn parse_sequence_inter_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    single_picture: bool,
) -> Result<SequenceInterConfig> {
    sr.in_struct("sequence_inter_config", Some("5.4.6"), |sr| {
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
            config.enable_refmvbank = sr.read_flag("enable_refmvbank")?;
            config.drl_reorder = read_drl_reorder(sr)?;
            config.seq_max_bvp_drl_bits_minus_1 =
                sr.read_ns("seq_max_bvp_drl_bits_minus_1", MAX_REF_BV_STACK_SIZE - 1)?;
            config.allow_frame_max_bvp_drl_bits = sr.read_flag("allow_frame_max_bvp_drl_bits")?;
            config.enable_bawp = sr.read_flag("enable_bawp")?;
            return Ok(config);
        }

        let mut motion_mode_enabled = false;
        for mode in INTERINTRA..MOTION_MODES {
            let enabled =
                sr.in_loop_item("mode", mode, |sr| sr.read_flag("seq_enabled_motion_modes"))?;
            config.seq_enabled_motion_modes[mode] = enabled;
            motion_mode_enabled |= enabled;
        }
        config.seq_frame_motion_modes_present_flag = if motion_mode_enabled {
            sr.read_flag("seq_frame_motion_modes_present_flag")?
        } else {
            sr.inferred_bool(
                "seq_frame_motion_modes_present_flag",
                "!motion_mode_enabled",
                false,
            );
            false
        };
        config.enable_six_param_warp_delta = if config.seq_enabled_motion_modes[DELTAWARP] {
            sr.read_flag("enable_six_param_warp_delta")?
        } else {
            sr.inferred_bool(
                "enable_six_param_warp_delta",
                "!seq_enabled_motion_modes[DELTAWARP]",
                false,
            );
            false
        };
        config.enable_masked_compound = sr.read_flag("enable_masked_compound")?;
        config.enable_ref_frame_mvs = sr.read_flag("enable_ref_frame_mvs")?;
        config.reduced_ref_frame_mvs_mode = if config.enable_ref_frame_mvs {
            sr.read_flag("reduced_ref_frame_mvs_mode")?
        } else {
            sr.inferred_bool("reduced_ref_frame_mvs_mode", "!enable_ref_frame_mvs", false);
            false
        };
        let order_hint_bits_minus_1 = sr.read_bits_u8("order_hint_bits_minus_1", 4)?;
        config.order_hint_bits = order_hint_bits_minus_1 + 1;
        config.enable_refmvbank = sr.read_flag("enable_refmvbank")?;
        config.drl_reorder = read_drl_reorder(sr)?;
        config.explicit_ref_frame_map = sr.read_flag("explicit_ref_frame_map")?;
        let explicit_num_ref_frames = sr.read_flag("explicit_num_ref_frames")?;
        config.num_ref_frames = if explicit_num_ref_frames {
            sr.read_bits_u8("num_ref_frames_minus_1", 4)? + 1
        } else {
            sr.inferred_u64("num_ref_frames_minus_1", "!explicit_num_ref_frames", 7);
            8
        };
        config.long_term_frame_id_bits = sr.read_bits_u8("long_term_frame_id_bits", 3)?;
        config.seq_max_drl_bits_minus_1 =
            sr.read_ns("seq_max_drl_bits_minus_1", MAX_REF_MV_STACK_SIZE - 1)?;
        config.allow_frame_max_drl_bits = sr.read_flag("allow_frame_max_drl_bits")?;
        config.seq_max_bvp_drl_bits_minus_1 =
            sr.read_ns("seq_max_bvp_drl_bits_minus_1", MAX_REF_BV_STACK_SIZE - 1)?;
        config.allow_frame_max_bvp_drl_bits = sr.read_flag("allow_frame_max_bvp_drl_bits")?;
        config.num_same_ref_compound = sr.read_bits_u8("num_same_ref_compound", 2)?;
        config.enable_tip = sr.read_flag("enable_tip")?;
        if config.enable_tip {
            let disable_tip_output = sr.read_flag("disable_tip_output")?;
            config.enable_tip_output = !disable_tip_output;
            config.enable_tip_hole_fill = sr.read_flag("enable_tip_hole_fill")?;
        } else {
            sr.inferred_bool("disable_tip_output", "!enable_tip", true);
            sr.inferred_bool("enable_tip_hole_fill", "!enable_tip", false);
        }
        config.enable_mv_traj = sr.read_flag("enable_mv_traj")?;
        config.enable_bawp = sr.read_flag("enable_bawp")?;
        config.enable_cwp = sr.read_flag("enable_cwp")?;
        config.enable_imp_msk_bld = sr.read_flag("enable_imp_msk_bld")?;
        config.enable_df_sub_pu = sr.read_flag("enable_df_sub_pu")?;
        config.enable_tip_explicit_qp = if config.enable_tip_output && config.enable_df_sub_pu {
            sr.read_flag("enable_tip_explicit_qp")?
        } else {
            sr.inferred_bool(
                "enable_tip_explicit_qp",
                "!(EnableTipOutput && enable_df_sub_pu)",
                false,
            );
            false
        };
        config.enable_opfl_refine = sr.read_bits_u8("enable_opfl_refine", 2)?;
        config.enable_refinemv = sr.read_flag("enable_refinemv")?;
        config.enable_tip_refinemv =
            if config.enable_tip && (config.enable_opfl_refine != 0 || config.enable_refinemv) {
                sr.read_flag("enable_tip_refinemv")?
            } else {
                sr.inferred_bool(
                    "enable_tip_refinemv",
                    "!(enable_tip && (enable_opfl_refine != 0 || enable_refinemv))",
                    false,
                );
                false
            };
        config.enable_bru = sr.read_flag("enable_bru")?;
        config.enable_adaptive_mvd = sr.read_flag("enable_adaptive_mvd")?;
        config.enable_mvd_sign_derive = sr.read_flag("enable_mvd_sign_derive")?;
        config.enable_flex_mvres = sr.read_flag("enable_flex_mvres")?;
        config.enable_global_motion = sr.read_flag("enable_global_motion")?;
        config.enable_short_refresh_frame_flags =
            sr.read_flag("enable_short_refresh_frame_flags")?;

        Ok(config)
    })
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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_scc_config_with_reader(&mut sr, single_picture)
}

pub(crate) fn parse_sequence_scc_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    single_picture: bool,
) -> Result<SequenceSccConfig> {
    sr.in_struct("sequence_scc_config", Some("5.4.7"), |sr| {
        if single_picture {
            sr.inferred_u64(
                "seq_force_screen_content_tools",
                "single_picture_header_flag",
                u64::from(SELECT_SCREEN_CONTENT_TOOLS),
            );
            sr.inferred_u64(
                "seq_force_integer_mv",
                "single_picture_header_flag",
                u64::from(SELECT_INTEGER_MV),
            );
            return Ok(SequenceSccConfig {
                seq_force_screen_content_tools: SELECT_SCREEN_CONTENT_TOOLS,
                seq_force_integer_mv: SELECT_INTEGER_MV,
            });
        }

        let seq_choose_screen_content_tools = sr.read_flag("seq_choose_screen_content_tools")?;
        let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
            sr.inferred_u64(
                "seq_force_screen_content_tools",
                "seq_choose_screen_content_tools",
                u64::from(SELECT_SCREEN_CONTENT_TOOLS),
            );
            SELECT_SCREEN_CONTENT_TOOLS
        } else {
            sr.read_bits_u8("seq_force_screen_content_tools", 1)?
        };

        let seq_force_integer_mv = if seq_force_screen_content_tools > 0 {
            let seq_choose_integer_mv = sr.read_flag("seq_choose_integer_mv")?;
            if seq_choose_integer_mv {
                sr.inferred_u64(
                    "seq_force_integer_mv",
                    "seq_choose_integer_mv",
                    u64::from(SELECT_INTEGER_MV),
                );
                SELECT_INTEGER_MV
            } else {
                sr.read_bits_u8("seq_force_integer_mv", 1)?
            }
        } else {
            sr.inferred_u64(
                "seq_force_integer_mv",
                "seq_force_screen_content_tools == 0",
                u64::from(SELECT_INTEGER_MV),
            );
            SELECT_INTEGER_MV
        };

        Ok(SequenceSccConfig {
            seq_force_screen_content_tools,
            seq_force_integer_mv,
        })
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
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_transform_quant_entropy_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequenceTqEntropyConfig> {
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_transform_quant_entropy_config_with_reader(&mut sr, monochrome, single_picture)
}

pub(crate) fn parse_sequence_transform_quant_entropy_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequenceTqEntropyConfig> {
    sr.in_struct(
        "sequence_transform_quant_entropy_config",
        Some("5.4.8"),
        |sr| {
            let enable_fsc = sr.read_flag("enable_fsc")?;
            let enable_idtx_intra = if enable_fsc {
                sr.inferred_bool("enable_idtx_intra", "enable_fsc", true);
                true
            } else {
                sr.read_flag("enable_idtx_intra")?
            };
            let enable_intra_ist = sr.read_flag("enable_intra_ist")?;
            let enable_inter_ist = sr.read_flag("enable_inter_ist")?;
            let enable_chroma_dctonly = if monochrome {
                sr.inferred_bool("enable_chroma_dctonly", "monochrome", false);
                false
            } else {
                sr.read_flag("enable_chroma_dctonly")?
            };
            let enable_inter_ddt = if single_picture {
                sr.inferred_bool("enable_inter_ddt", "single_picture_header_flag", false);
                false
            } else {
                sr.read_flag("enable_inter_ddt")?
            };
            let reduced_tx_part_set = sr.read_flag("reduced_tx_part_set")?;
            let enable_cctx = if monochrome {
                sr.inferred_bool("enable_cctx", "monochrome", false);
                false
            } else {
                sr.read_flag("enable_cctx")?
            };
            let enable_tcq = sr.read_flag("enable_tcq")?;
            let choose_tcq_per_frame = if enable_tcq && !single_picture {
                sr.read_flag("choose_tcq_per_frame")?
            } else {
                sr.inferred_bool(
                    "choose_tcq_per_frame",
                    "!(enable_tcq && !single_picture_header_flag)",
                    false,
                );
                false
            };
            let enable_parity_hiding = if enable_tcq && !choose_tcq_per_frame {
                sr.inferred_bool(
                    "enable_parity_hiding",
                    "enable_tcq && !choose_tcq_per_frame",
                    false,
                );
                false
            } else {
                sr.read_flag("enable_parity_hiding")?
            };
            let (enable_avg_cdf, avg_cdf_type) = if single_picture {
                sr.inferred_bool("enable_avg_cdf", "single_picture_header_flag", true);
                sr.inferred_u64("avg_cdf_type", "single_picture_header_flag", 1);
                (true, 1)
            } else {
                let enable_avg_cdf = sr.read_flag("enable_avg_cdf")?;
                let avg_cdf_type = if enable_avg_cdf {
                    sr.read_bits_u8("avg_cdf_type", 1)?
                } else {
                    sr.inferred_u64("avg_cdf_type", "!enable_avg_cdf", 0);
                    0
                };
                (enable_avg_cdf, avg_cdf_type)
            };
            let separate_uv_delta_q = if monochrome {
                sr.inferred_bool("separate_uv_delta_q", "monochrome", false);
                false
            } else {
                sr.read_flag("separate_uv_delta_q")?
            };

            let equal_ac_dc_q = sr.read_flag("equal_ac_dc_q")?;
            let mut base_y_dc_delta_q = 0;
            let mut y_dc_delta_q_enabled = false;
            if !equal_ac_dc_q {
                base_y_dc_delta_q = sr.read_bits_u8("base_y_dc_delta_q", 5)?;
                y_dc_delta_q_enabled = sr.read_flag("y_dc_delta_q_enabled")?;
            } else {
                sr.inferred_u64("base_y_dc_delta_q", "equal_ac_dc_q", 0);
                sr.inferred_bool("y_dc_delta_q_enabled", "equal_ac_dc_q", false);
            }
            let mut base_uv_dc_delta_q = 0;
            let mut uv_dc_delta_q_enabled = false;
            let mut base_uv_ac_delta_q = 0;
            let mut uv_ac_delta_q_enabled = false;
            if !monochrome {
                if !equal_ac_dc_q {
                    base_uv_dc_delta_q = sr.read_bits_u8("base_uv_dc_delta_q", 5)?;
                    uv_dc_delta_q_enabled = sr.read_flag("uv_dc_delta_q_enabled")?;
                }
                base_uv_ac_delta_q = sr.read_bits_u8("base_uv_ac_delta_q", 5)?;
                uv_ac_delta_q_enabled = sr.read_flag("uv_ac_delta_q_enabled")?;
                if equal_ac_dc_q {
                    base_uv_dc_delta_q = base_uv_ac_delta_q;
                    sr.inferred_u64(
                        "base_uv_dc_delta_q",
                        "equal_ac_dc_q",
                        u64::from(base_uv_dc_delta_q),
                    );
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
        },
    )
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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_filter_config_with_reader(&mut sr, single_picture, seq_sb_size)
}

pub(crate) fn parse_sequence_filter_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    single_picture: bool,
    seq_sb_size: SuperblockSize,
) -> Result<SequenceFilterConfig> {
    sr.in_struct("sequence_filter_config", Some("5.4.10"), |sr| {
        let disable_loopfilters_across_tiles = sr.read_flag("disable_loopfilters_across_tiles")?;
        let enable_cdef = sr.read_flag("enable_cdef")?;
        let enable_gdf = sr.read_flag("enable_gdf")?;
        let gdf_unit_matches_sb_size = if enable_gdf && seq_sb_size == SuperblockSize::Block64x64 {
            sr.read_flag("gdf_unit_matches_sb_size")?
        } else {
            sr.inferred_bool(
                "gdf_unit_matches_sb_size",
                "!(enable_gdf && SeqSbSize == BLOCK_64X64)",
                false,
            );
            false
        };
        let enable_restoration = sr.read_flag("enable_restoration")?;
        let mut lr_pc_wiener_disabled = false;
        let mut lr_wiener_nonsep_disabled = false;
        let mut lr_tools_uv_present = false;
        let mut lr_uv_wiener_nonsep_disabled = false;
        let lr_uv_pc_wiener_disabled = enable_restoration;
        if enable_restoration {
            lr_pc_wiener_disabled = sr.read_flag("lr_tools_disable[0][RESTORE_PC_WIENER]")?;
            lr_wiener_nonsep_disabled =
                sr.read_flag("lr_tools_disable[0][RESTORE_WIENER_NONSEP]")?;
            lr_tools_uv_present = sr.read_flag("lr_tools_uv_present")?;
            lr_uv_wiener_nonsep_disabled = if lr_tools_uv_present {
                sr.read_flag("lr_tools_disable[1][RESTORE_WIENER_NONSEP]")?
            } else {
                sr.inferred_bool(
                    "lr_tools_disable[1][RESTORE_WIENER_NONSEP]",
                    "!lr_tools_uv_present",
                    lr_wiener_nonsep_disabled,
                );
                lr_wiener_nonsep_disabled
            };
        }
        let enable_ccso = sr.read_flag("enable_ccso")?;
        let ccso_unit_matches_sb_size = if enable_ccso {
            sr.read_flag("ccso_unit_matches_sb_size")?
        } else {
            sr.inferred_bool("ccso_unit_matches_sb_size", "!enable_ccso", false);
            false
        };
        let cdef_on_skip_txfm = if single_picture {
            sr.absent("cdef_on_skip_txfm_always_on", "single_picture_header_flag");
            CdefOnSkipTxfm::Adaptive
        } else {
            let cdef_on_skip_txfm_always_on = sr.read_flag("cdef_on_skip_txfm_always_on")?;
            if cdef_on_skip_txfm_always_on {
                sr.absent("cdef_on_skip_txfm_disabled", "cdef_on_skip_txfm_always_on");
                CdefOnSkipTxfm::AlwaysOn
            } else {
                let cdef_on_skip_txfm_disabled = sr.read_flag("cdef_on_skip_txfm_disabled")?;
                if cdef_on_skip_txfm_disabled {
                    CdefOnSkipTxfm::Disabled
                } else {
                    CdefOnSkipTxfm::Adaptive
                }
            }
        };
        let df_par_bits_minus_2 = sr.read_bits_u8("df_par_bits_minus_2", 2)?;

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
    pub seq_sb_col_starts: std::sync::Arc<[u32]>,
    /// `SeqSbRowStarts[0..SeqTileRows]` (AV2 § 5.4.2; the `sbRowStarts` companion of
    /// [`Self::seq_sb_col_starts`]). Bounded by `MAX_TILE_ROWS`.
    pub seq_sb_row_starts: std::sync::Arc<[u32]>,
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
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_sequence_tile_config_with_reader(&mut sr, tile_params_input)
}

pub(crate) fn parse_sequence_tile_config_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    tile_params_input: TileParamsInput,
) -> Result<SequenceTileConfig> {
    sr.in_struct("sequence_tile_config", Some("5.4.2"), |sr| {
        let seq_tile_info_present_flag = sr.read_flag("seq_tile_info_present_flag")?;
        if !seq_tile_info_present_flag {
            sr.absent("allow_tile_info_change", "!seq_tile_info_present_flag");
            return Ok(SequenceTileConfig {
                seq_tile_info_present_flag: false,
                allow_tile_info_change: None,
                params: None,
                seq_sb_col_starts: std::sync::Arc::from(Vec::new()),
                seq_sb_row_starts: std::sync::Arc::from(Vec::new()),
            });
        }

        let allow_tile_info_change = sr.read_flag("allow_tile_info_change")?;
        let (params, seq_sb_col_starts, seq_sb_row_starts) =
            match parse_tile_layout_with_reader(sr, tile_params_input) {
                Ok(layout) => (
                    Some(layout.params),
                    layout.sb_col_starts,
                    layout.sb_row_starts,
                ),
                Err(Error::Unimplemented { .. }) => (
                    None,
                    crate::tile::TileStarts::default(),
                    crate::tile::TileStarts::default(),
                ),
                Err(error) => return Err(error),
            };

        Ok(SequenceTileConfig {
            seq_tile_info_present_flag: true,
            allow_tile_info_change: Some(allow_tile_info_change),
            params,
            seq_sb_col_starts: std::sync::Arc::from(&*seq_sb_col_starts),
            seq_sb_row_starts: std::sync::Arc::from(&*seq_sb_row_starts),
        })
    })
}

/// The largest `NumRefFrames` a sequence header can signal (AV2 v1.0.0 § 5.5.2):
/// `num_ref_frames_minus_1` is `f(4)`, so the reference store never holds more
/// than sixteen slots.
pub const MAX_REF_FRAMES: usize = 16;
