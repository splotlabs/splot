// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header **child-config** writers — the inverse of the § 5.4.3 – § 5.4.8
//! config cascade parsers in [`crate::headers::sequence`] (`ENC-BITSTREAM-WRITER`):
//!
//! - [`write_sequence_partition_config`] — `sequence_partition_config()` (§ 5.4.3,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-3`).
//! - [`write_sequence_segment_config`] — `sequence_segment_config()` (§ 5.4.4,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-4`), which delegates to
//!   [`crate::write::write_seg_info`] for the `seg_info()` body (§ 5.4.9).
//! - [`write_sequence_intra_config`] — `sequence_intra_config()` (§ 5.4.5,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-5`).
//! - [`write_sequence_inter_config`] — `sequence_inter_config()` (§ 5.4.6,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6`).
//! - [`write_sequence_scc_config`] — `sequence_scc_config()` (§ 5.4.7,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-7`).
//! - [`write_sequence_transform_quant_entropy_config`] —
//!   `sequence_transform_quant_entropy_config()` (§ 5.4.8,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8`).
//!
//! These configs are read **mid-byte** inside `sequence_header_obu()` (immediately after
//! the general fields / preceding config), so — unlike the byte-aligned § 5.4.1 prefix —
//! the writers do **not** require byte alignment at entry; they mirror the parser's bit
//! position exactly. The top-level `write_sequence_header` (which composes them in
//! § 5.4.1 read order) and the filter/tile configs land in later changes.
//!
//! This module is additive: it depends on the model/parser read-only and serializes a
//! parsed config back to bits via [`BitWriter`]. The universal contract is semantic
//! `read(write(x)) == x` for every model the parser can produce, e.g.
//! `parse_sequence_partition_config(write_sequence_partition_config(c, ..), ..) == c`.
//!
//! Several config fields are stored *derived* rather than as raw bits — gated tools that
//! a `Monochrome` / `single_picture_header_flag` header infers, `minus_1` recoveries, the
//! mirrored `BaseUVDcDeltaQ = BaseUVAcDeltaQ`, and the `MaxSegments` / `MaxPbAspectRatio`
//! constants that are never signaled. A model carrying a value the parser could never have
//! emitted is rejected up front with a typed [`WriteError`] *before any bit is written*
//! (reject-before-write), so the writer emits bits only from values the parser would have
//! signaled and the round-trip property is provable. See
//! [`WriteError::NonCanonicalSequenceValue`].

use crate::headers::sequence::{
    DrlReorder, SequenceInterConfig, SequenceIntraConfig, SequencePartitionConfig,
    SequenceSccConfig, SequenceSegmentConfig, SequenceTqEntropyConfig,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::segment::{check_seg_info_encodable, write_seg_info};

/// `MOTION_MODES` (AV2 § 3): number of motion modes; index `0` (`SIMPLE`) is never
/// signaled. Duplicated locally because the parser's copy is private.
const MOTION_MODES: usize = 5;
/// `INTERINTRA` (AV2 § 3): first signaled motion-mode index.
const INTERINTRA: usize = 1;
/// `DELTAWARP` (AV2 § 3): delta-warp motion-mode index, gates `enable_six_param_warp_delta`.
const DELTAWARP: usize = 3;
/// `MAX_REF_MV_STACK_SIZE` (AV2 § 3); `seq_max_drl_bits_minus_1` is `ns(MAX_REF_MV_STACK_SIZE - 1)`.
const MAX_REF_MV_STACK_SIZE: u32 = 6;
/// `MAX_REF_BV_STACK_SIZE` (AV2 § 3); `seq_max_bvp_drl_bits_minus_1` is `ns(MAX_REF_BV_STACK_SIZE - 1)`.
const MAX_REF_BV_STACK_SIZE: u32 = 4;
/// `SELECT_SCREEN_CONTENT_TOOLS` (AV2 § 3): the "choose per frame" sentinel value `2`.
const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;
/// `SELECT_INTEGER_MV` (AV2 § 3): the "choose per frame" sentinel value `2`.
const SELECT_INTEGER_MV: u8 = 2;

/// Returns `Ok(())` if `value` fits in `width_bits`, else [`WriteError::ValueTooWide`] —
/// the same bound the `f(n)` write enforces, checked up front so a rejected config never
/// leaves a partial encoding in the writer.
fn check_field_width(value: u64, width_bits: u32) -> WriteResult<()> {
    let fits = width_bits >= 64 || value < (1u64 << width_bits);
    if fits {
        Ok(())
    } else {
        Err(WriteError::ValueTooWide { value, width_bits })
    }
}

// =============================================================================
// § 5.4.3 sequence_partition_config()
// =============================================================================

/// Writes `sequence_partition_config()` (AV2 v1.0.0 § 5.4.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-3`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_partition_config`].
///
/// `monochrome` (`Monochrome`) and `single_picture` (`single_picture_header_flag`) are
/// threaded in from the general header — never re-derived — and gate the same conditional
/// fields the parser reads: `enable_sdp` is signaled only when `!monochrome`,
/// `enable_extended_sdp` only when `enable_sdp && !single_picture`.
///
/// Field writes (in § 5.4.3 read order): `use_256x256_superblock` `f(1)`;
/// `use_128x128_superblock` `f(1)` (only when `!use_256x256_superblock`); `enable_sdp`
/// `f(1)` (only when `!monochrome`); `enable_extended_sdp` `f(1)` (only when
/// `enable_sdp && !single_picture`); `enable_ext_partitions` `f(1)`;
/// `enable_uneven_4way_partitions` `f(1)` (only when `enable_ext_partitions`);
/// `reduce_pb_aspect_ratio` `f(1)`; `max_pb_aspect_ratio_log2_minus_1` `f(1)` (only when
/// `reduce_pb_aspect_ratio`). `seq_sb_size()` and `MaxPbAspectRatio` are derived, never
/// signaled.
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalSequenceValue`] if a derived/inferred field disagrees with
/// the § 5.4.3 re-derivation: `use_128x128_superblock` set while `use_256x256_superblock`,
/// `enable_sdp`/`enable_extended_sdp`/`enable_uneven_4way_partitions` set while their gate
/// is false, or `max_pb_aspect_ratio` not equal to the value the bitstream would yield
/// (`8` when `!reduce_pb_aspect_ratio`, else `2` or `4`).
pub fn write_sequence_partition_config(
    writer: &mut BitWriter,
    config: &SequencePartitionConfig,
    monochrome: bool,
    single_picture: bool,
) -> WriteResult<()> {
    check_partition_encodable(config, monochrome, single_picture)?;

    writer.write_bit(u8::from(config.use_256x256_superblock))?;
    if !config.use_256x256_superblock {
        writer.write_bit(u8::from(config.use_128x128_superblock))?;
    }
    if !monochrome {
        writer.write_bit(u8::from(config.enable_sdp))?;
    }
    if config.enable_sdp && !single_picture {
        writer.write_bit(u8::from(config.enable_extended_sdp))?;
    }
    writer.write_bit(u8::from(config.enable_ext_partitions))?;
    if config.enable_ext_partitions {
        writer.write_bit(u8::from(config.enable_uneven_4way_partitions))?;
    }
    writer.write_bit(u8::from(config.reduce_pb_aspect_ratio))?;
    if config.reduce_pb_aspect_ratio {
        // MaxPbAspectRatio = 1 << (log2_minus_1 + 1); log2_minus_1 is f(1) in {0, 1},
        // so MaxPbAspectRatio is 2 or 4. Recover log2_minus_1 (validated up front).
        let log2_minus_1 = partition_aspect_log2_minus_1(config.max_pb_aspect_ratio)?;
        writer.write_bits_u8(log2_minus_1, 1)?;
    }
    Ok(())
}

/// Recovers `max_pb_aspect_ratio_log2_minus_1` (a 1-bit field) from `MaxPbAspectRatio`,
/// the inverse of `1 << (log2_minus_1 + 1)`. Valid results are `2` (log2_minus_1 = 0) and
/// `4` (log2_minus_1 = 1); any other value is non-canonical.
fn partition_aspect_log2_minus_1(max_pb_aspect_ratio: u32) -> WriteResult<u8> {
    match max_pb_aspect_ratio {
        2 => Ok(0),
        4 => Ok(1),
        _ => Err(WriteError::NonCanonicalSequenceValue {
            what: "max_pb_aspect_ratio",
        }),
    }
}

/// Validates that `config` is a model the § 5.4.3 parser could have produced.
fn check_partition_encodable(
    config: &SequencePartitionConfig,
    monochrome: bool,
    single_picture: bool,
) -> WriteResult<()> {
    // use_128x128_superblock is only read when !use_256x256_superblock; otherwise the
    // parser infers it false.
    if config.use_256x256_superblock && config.use_128x128_superblock {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "use_128x128_superblock",
        });
    }
    // enable_sdp is inferred 0 for Monochrome.
    if monochrome && config.enable_sdp {
        return Err(WriteError::NonCanonicalSequenceValue { what: "enable_sdp" });
    }
    // enable_extended_sdp is inferred 0 unless enable_sdp && !single_picture.
    let extended_sdp_signaled = config.enable_sdp && !single_picture;
    if !extended_sdp_signaled && config.enable_extended_sdp {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_extended_sdp",
        });
    }
    // enable_uneven_4way_partitions is inferred 0 unless enable_ext_partitions.
    if !config.enable_ext_partitions && config.enable_uneven_4way_partitions {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_uneven_4way_partitions",
        });
    }
    // MaxPbAspectRatio: 8 when !reduce, else 2 or 4 (the only `1 << (k+1)` results).
    if config.reduce_pb_aspect_ratio {
        partition_aspect_log2_minus_1(config.max_pb_aspect_ratio)?;
    } else if config.max_pb_aspect_ratio != 8 {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "max_pb_aspect_ratio",
        });
    }
    Ok(())
}

// =============================================================================
// § 5.4.4 sequence_segment_config()
// =============================================================================

/// Writes `sequence_segment_config()` (AV2 v1.0.0 § 5.4.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-4`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_segment_config`]. It delegates to
/// [`crate::write::write_seg_info`] for the `seg_info(MaxSegments)` body (§ 5.4.9).
///
/// Field writes (in § 5.4.4 read order): `enable_ext_seg` `f(1)`;
/// `seq_seg_info_present_flag` `f(1)`; when the flag is set, `seq_allow_seg_info_change`
/// `f(1)` then `seg_info(MaxSegments)`. `MaxSegments` (`16`/`8`) is derived from
/// `enable_ext_seg`, never signaled.
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] if `max_segments` is not the derived
///   `enable_ext_seg ? 16 : 8`, or if the `Option` payloads
///   (`seq_allow_seg_info_change` / `segment_info`) disagree with `seq_seg_info_present_flag`.
/// - the `seg_info` [`WriteError`] variants propagated from [`crate::write::write_seg_info`].
pub fn write_sequence_segment_config(
    writer: &mut BitWriter,
    config: &SequenceSegmentConfig,
) -> WriteResult<()> {
    check_segment_encodable(config)?;

    writer.write_bit(u8::from(config.enable_ext_seg))?;
    writer.write_bit(u8::from(config.seq_seg_info_present_flag))?;
    if config.seq_seg_info_present_flag {
        // Both Options are present iff the flag is set (checked up front).
        let allow =
            config
                .seq_allow_seg_info_change
                .ok_or(WriteError::NonCanonicalSequenceValue {
                    what: "seq_allow_seg_info_change",
                })?;
        let info = config
            .segment_info
            .as_ref()
            .ok_or(WriteError::NonCanonicalSequenceValue {
                what: "segment_info",
            })?;
        writer.write_bit(u8::from(allow))?;
        write_seg_info(writer, info, config.max_segments)?;
    }
    Ok(())
}

/// Validates that `config` is a model the § 5.4.4 parser could have produced.
fn check_segment_encodable(config: &SequenceSegmentConfig) -> WriteResult<()> {
    // MaxSegments is derived, never signaled.
    let expected_max = if config.enable_ext_seg { 16 } else { 8 };
    if config.max_segments != expected_max {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "max_segments",
        });
    }
    // The two Option payloads are present iff seq_seg_info_present_flag is set.
    let allow_present = config.seq_allow_seg_info_change.is_some();
    let info_present = config.segment_info.is_some();
    if allow_present != config.seq_seg_info_present_flag
        || info_present != config.seq_seg_info_present_flag
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_seg_info_present_flag",
        });
    }
    // Pre-validate the nested seg_info() body up front, so a bad body is rejected before
    // the writer emits the leading enable_ext_seg / present / allow flags
    // (reject-before-write for the composite § 5.4.4 structure).
    if let Some(info) = &config.segment_info {
        check_seg_info_encodable(info, config.max_segments)?;
    }
    Ok(())
}

// =============================================================================
// § 5.4.5 sequence_intra_config()
// =============================================================================

/// Writes `sequence_intra_config()` (AV2 v1.0.0 § 5.4.5,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-5`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_intra_config`].
///
/// `monochrome` (`Monochrome`) is threaded in from the general header and gates
/// `cfl_ds_filter_index`, which is signaled (`f(2)`) only when `!monochrome` (else
/// inferred `0`).
///
/// Field writes (in § 5.4.5 read order): `enable_dip`, `enable_intra_edge_filter`,
/// `enable_mrls`, `enable_cfl_intra` each `f(1)`; `cfl_ds_filter_index` `f(2)` (only when
/// `!monochrome`); `enable_mhccp`, `enable_ibp` each `f(1)`.
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalSequenceValue`] if `cfl_ds_filter_index` is non-zero while
/// `monochrome` (the parser would have inferred `0`); [`WriteError::ValueTooWide`] if
/// `cfl_ds_filter_index` does not fit `f(2)`.
pub fn write_sequence_intra_config(
    writer: &mut BitWriter,
    config: &SequenceIntraConfig,
    monochrome: bool,
) -> WriteResult<()> {
    check_intra_encodable(config, monochrome)?;

    writer.write_bit(u8::from(config.enable_dip))?;
    writer.write_bit(u8::from(config.enable_intra_edge_filter))?;
    writer.write_bit(u8::from(config.enable_mrls))?;
    writer.write_bit(u8::from(config.enable_cfl_intra))?;
    if !monochrome {
        writer.write_bits_u8(config.cfl_ds_filter_index, 2)?;
    }
    writer.write_bit(u8::from(config.enable_mhccp))?;
    writer.write_bit(u8::from(config.enable_ibp))?;
    Ok(())
}

/// Validates that `config` is a model the § 5.4.5 parser could have produced.
fn check_intra_encodable(config: &SequenceIntraConfig, monochrome: bool) -> WriteResult<()> {
    if monochrome {
        // cfl_ds_filter_index is inferred 0 for Monochrome (no bits read).
        if config.cfl_ds_filter_index != 0 {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "cfl_ds_filter_index",
            });
        }
    } else {
        check_field_width(u64::from(config.cfl_ds_filter_index), 2)?;
    }
    Ok(())
}

// =============================================================================
// § 5.4.6 sequence_inter_config()
// =============================================================================

/// Returns the `disable_drl_reorder` / `constrain_drl_reorder` bit pattern for a
/// [`DrlReorder`] (AV2 § 5.4.6): `Disabled` -> `disable = 1` (one bit); `Constraint` ->
/// `disable = 0, constrain = 1`; `Always` -> `disable = 0, constrain = 0`.
fn write_drl_reorder(writer: &mut BitWriter, reorder: DrlReorder) -> WriteResult<()> {
    match reorder {
        DrlReorder::Disabled => writer.write_bit(1),
        DrlReorder::Constraint => {
            writer.write_bit(0)?;
            writer.write_bit(1)
        }
        DrlReorder::Always => {
            writer.write_bit(0)?;
            writer.write_bit(0)
        }
    }
}

/// Writes `sequence_inter_config()` (AV2 v1.0.0 § 5.4.6,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_inter_config`].
///
/// `single_picture` (`single_picture_header_flag`) is threaded in from the general header
/// and selects between the two disjoint branches: the single-picture branch signals only
/// `enable_refmvbank`, the DRL-reorder pattern, `seq_max_bvp_drl_bits_minus_1`
/// (`ns(MAX_REF_BV_STACK_SIZE - 1)`), `allow_frame_max_bvp_drl_bits`, and `enable_bawp`
/// (every other field is inferred to its § 5.4.6 default); the full branch signals the
/// complete tool set with its nested gates (`seq_frame_motion_modes_present_flag` only
/// when a motion mode is enabled, `enable_six_param_warp_delta` only when `DELTAWARP` is
/// enabled, `order_hint_bits` as `order_hint_bits_minus_1`, the `explicit_ref_frame_map`
/// /`NumRefFrames` cascade, the `enable_tip` cascade, and `enable_global_motion`).
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] if a single-picture header carries any
///   field other than its § 5.4.6 inferred default, if `seq_enabled_motion_modes[SIMPLE]`
///   (index 0) is set, or if `order_hint_bits == 0` (it is `minus_1 + 1`, so `>= 1`).
/// - [`WriteError::ValueTooWide`] if `order_hint_bits - 1`, `num_ref_frames - 1`,
///   `long_term_frame_id_bits`, `num_same_ref_compound`, or `enable_opfl_refine` exceeds
///   its field width.
/// - [`WriteError::ValueOutOfRange`] if `seq_max_drl_bits_minus_1` or
///   `seq_max_bvp_drl_bits_minus_1` is outside its `ns(n)` domain.
pub fn write_sequence_inter_config(
    writer: &mut BitWriter,
    config: &SequenceInterConfig,
    single_picture: bool,
) -> WriteResult<()> {
    check_inter_encodable(config, single_picture)?;

    if single_picture {
        writer.write_bit(u8::from(config.enable_refmvbank))?;
        write_drl_reorder(writer, config.drl_reorder)?;
        writer.write_ns(
            config.seq_max_bvp_drl_bits_minus_1,
            MAX_REF_BV_STACK_SIZE - 1,
        )?;
        writer.write_bit(u8::from(config.allow_frame_max_bvp_drl_bits))?;
        writer.write_bit(u8::from(config.enable_bawp))?;
        return Ok(());
    }

    // Full branch. seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]: f(1) each.
    for mode in INTERINTRA..MOTION_MODES {
        writer.write_bit(u8::from(config.seq_enabled_motion_modes[mode]))?;
    }
    // seq_frame_motion_modes_present_flag: f(1), only when any motion mode is enabled.
    if any_motion_mode_enabled(config) {
        writer.write_bit(u8::from(config.seq_frame_motion_modes_present_flag))?;
    }
    // enable_six_param_warp_delta: f(1), only when DELTAWARP is enabled.
    if config.seq_enabled_motion_modes[DELTAWARP] {
        writer.write_bit(u8::from(config.enable_six_param_warp_delta))?;
    }
    writer.write_bit(u8::from(config.enable_masked_compound))?;
    writer.write_bit(u8::from(config.enable_ref_frame_mvs))?;
    // reduced_ref_frame_mvs_mode: f(1), only when enable_ref_frame_mvs.
    if config.enable_ref_frame_mvs {
        writer.write_bit(u8::from(config.reduced_ref_frame_mvs_mode))?;
    }
    // order_hint_bits_minus_1: f(4) of (order_hint_bits - 1).
    writer.write_bits_u8(config.order_hint_bits - 1, 4)?;
    writer.write_bit(u8::from(config.enable_refmvbank))?;
    write_drl_reorder(writer, config.drl_reorder)?;
    writer.write_bit(u8::from(config.explicit_ref_frame_map))?;
    // explicit_num_ref_frames: f(1); when set, num_ref_frames_minus_1: f(4).
    writer.write_bit(u8::from(config.explicit_num_ref_frames()))?;
    if config.explicit_num_ref_frames() {
        writer.write_bits_u8(config.num_ref_frames - 1, 4)?;
    }
    writer.write_bits_u8(config.long_term_frame_id_bits, 3)?;
    writer.write_ns(config.seq_max_drl_bits_minus_1, MAX_REF_MV_STACK_SIZE - 1)?;
    writer.write_bit(u8::from(config.allow_frame_max_drl_bits))?;
    writer.write_ns(
        config.seq_max_bvp_drl_bits_minus_1,
        MAX_REF_BV_STACK_SIZE - 1,
    )?;
    writer.write_bit(u8::from(config.allow_frame_max_bvp_drl_bits))?;
    writer.write_bits_u8(config.num_same_ref_compound, 2)?;
    writer.write_bit(u8::from(config.enable_tip))?;
    if config.enable_tip {
        // disable_tip_output = !EnableTipOutput.
        writer.write_bit(u8::from(!config.enable_tip_output))?;
        writer.write_bit(u8::from(config.enable_tip_hole_fill))?;
    }
    writer.write_bit(u8::from(config.enable_mv_traj))?;
    writer.write_bit(u8::from(config.enable_bawp))?;
    writer.write_bit(u8::from(config.enable_cwp))?;
    writer.write_bit(u8::from(config.enable_imp_msk_bld))?;
    writer.write_bit(u8::from(config.enable_df_sub_pu))?;
    // enable_tip_explicit_qp: f(1), only when EnableTipOutput && enable_df_sub_pu.
    if config.enable_tip_output && config.enable_df_sub_pu {
        writer.write_bit(u8::from(config.enable_tip_explicit_qp))?;
    }
    writer.write_bits_u8(config.enable_opfl_refine, 2)?;
    writer.write_bit(u8::from(config.enable_refinemv))?;
    // enable_tip_refinemv: f(1), only when enable_tip && (opfl_refine != 0 || refinemv).
    if config.enable_tip && (config.enable_opfl_refine != 0 || config.enable_refinemv) {
        writer.write_bit(u8::from(config.enable_tip_refinemv))?;
    }
    writer.write_bit(u8::from(config.enable_bru))?;
    writer.write_bit(u8::from(config.enable_adaptive_mvd))?;
    writer.write_bit(u8::from(config.enable_mvd_sign_derive))?;
    writer.write_bit(u8::from(config.enable_flex_mvres))?;
    // enable_global_motion: f(1) (single_picture is false on this branch).
    writer.write_bit(u8::from(config.enable_global_motion))?;
    writer.write_bit(u8::from(config.enable_short_refresh_frame_flags))?;
    Ok(())
}

/// Returns `true` if any signaled motion mode (`INTERINTRA..MOTION_MODES`) is enabled —
/// the parser's `motionModeEnabled` accumulator that gates
/// `seq_frame_motion_modes_present_flag`.
fn any_motion_mode_enabled(config: &SequenceInterConfig) -> bool {
    config.seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]
        .iter()
        .any(|&enabled| enabled)
}

trait InterConfigExt {
    /// Recovers `explicit_num_ref_frames` (§ 5.4.6): the parser reads `f(4)` of
    /// `num_ref_frames_minus_1` when this is `true`, else infers `NumRefFrames = 8`.
    fn explicit_num_ref_frames(&self) -> bool;
}

impl InterConfigExt for SequenceInterConfig {
    fn explicit_num_ref_frames(&self) -> bool {
        // `explicit_num_ref_frames` is true iff `NumRefFrames` was signaled, i.e. != the
        // inferred 8. A stored `8` is written via the non-explicit (inferred) form — both
        // an explicit-8 and an inferred-8 reparse to `num_ref_frames == 8`, so the
        // canonical (shorter) encoding round-trips.
        self.num_ref_frames != 8
    }
}

/// Validates that `config` is a model the § 5.4.6 parser could have produced. The
/// single-picture branch infers every field except the five it signals; the full branch
/// has field-width bounds and the `SIMPLE` motion-mode invariant.
fn check_inter_encodable(config: &SequenceInterConfig, single_picture: bool) -> WriteResult<()> {
    // seq_enabled_motion_modes[0] (SIMPLE) is never signaled; it must stay 0.
    if config.seq_enabled_motion_modes[0] {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_enabled_motion_modes_simple",
        });
    }

    if single_picture {
        // The single-picture branch infers every field except enable_refmvbank,
        // drl_reorder, seq_max_bvp_drl_bits_minus_1, allow_frame_max_bvp_drl_bits, and
        // enable_bawp. Reject any non-default value for the inferred fields.
        let inferred_ok = !config.seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]
            .iter()
            .any(|&e| e)
            && !config.seq_frame_motion_modes_present_flag
            && !config.enable_six_param_warp_delta
            && !config.enable_masked_compound
            && !config.enable_ref_frame_mvs
            && !config.reduced_ref_frame_mvs_mode
            && config.order_hint_bits == 0
            && !config.explicit_ref_frame_map
            && config.num_ref_frames == 2
            && config.long_term_frame_id_bits == 0
            && config.seq_max_drl_bits_minus_1 == 0
            && !config.allow_frame_max_drl_bits
            && config.num_same_ref_compound == 0
            && !config.enable_tip
            && !config.enable_tip_output
            && !config.enable_tip_hole_fill
            && !config.enable_mv_traj
            && !config.enable_cwp
            && !config.enable_imp_msk_bld
            && !config.enable_df_sub_pu
            && !config.enable_tip_explicit_qp
            && config.enable_opfl_refine == 0
            && !config.enable_refinemv
            && !config.enable_tip_refinemv
            && !config.enable_bru
            && !config.enable_adaptive_mvd
            && !config.enable_mvd_sign_derive
            && !config.enable_flex_mvres
            && !config.enable_global_motion
            && !config.enable_short_refresh_frame_flags;
        if !inferred_ok {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_inter_inferred",
            });
        }
        // The five signaled fields: only seq_max_bvp_drl_bits_minus_1 has a width bound.
        if config.seq_max_bvp_drl_bits_minus_1 >= MAX_REF_BV_STACK_SIZE - 1 {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: i64::from(config.seq_max_bvp_drl_bits_minus_1),
            });
        }
        return Ok(());
    }

    // Full branch field-width / domain bounds.
    // order_hint_bits = order_hint_bits_minus_1 + 1, so it must be >= 1 and <= 16.
    if config.order_hint_bits == 0 {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "order_hint_bits",
        });
    }
    check_field_width(u64::from(config.order_hint_bits - 1), 4)?;
    // num_ref_frames: when explicit, num_ref_frames_minus_1 is f(4), so 1..=16; the
    // inferred (non-explicit) value is exactly 8.
    if config.num_ref_frames == 0 {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "num_ref_frames",
        });
    }
    if config.num_ref_frames != 8 {
        check_field_width(u64::from(config.num_ref_frames - 1), 4)?;
    }
    check_field_width(u64::from(config.long_term_frame_id_bits), 3)?;
    check_field_width(u64::from(config.num_same_ref_compound), 2)?;
    check_field_width(u64::from(config.enable_opfl_refine), 2)?;
    // ns(n) domain bounds: value < n.
    if config.seq_max_drl_bits_minus_1 >= MAX_REF_MV_STACK_SIZE - 1 {
        return Err(WriteError::ValueOutOfRange {
            descriptor: "ns",
            value: i64::from(config.seq_max_drl_bits_minus_1),
        });
    }
    if config.seq_max_bvp_drl_bits_minus_1 >= MAX_REF_BV_STACK_SIZE - 1 {
        return Err(WriteError::ValueOutOfRange {
            descriptor: "ns",
            value: i64::from(config.seq_max_bvp_drl_bits_minus_1),
        });
    }

    // Reject non-canonical values behind DISABLED gates: when a gate is false the parser
    // infers these fields to their defaults and does not read them, so a stored non-default
    // would shift the rest of the stream and break read(write(x)) == x.
    if !any_motion_mode_enabled(config) && config.seq_frame_motion_modes_present_flag {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_frame_motion_modes_present_flag",
        });
    }
    if !config.seq_enabled_motion_modes[DELTAWARP] && config.enable_six_param_warp_delta {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_six_param_warp_delta",
        });
    }
    if !config.enable_ref_frame_mvs && config.reduced_ref_frame_mvs_mode {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "reduced_ref_frame_mvs_mode",
        });
    }
    if !config.enable_tip && (config.enable_tip_output || config.enable_tip_hole_fill) {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_tip_subfields",
        });
    }
    if !(config.enable_tip_output && config.enable_df_sub_pu) && config.enable_tip_explicit_qp {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_tip_explicit_qp",
        });
    }
    if !(config.enable_tip && (config.enable_opfl_refine != 0 || config.enable_refinemv))
        && config.enable_tip_refinemv
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_tip_refinemv",
        });
    }
    Ok(())
}

// =============================================================================
// § 5.4.7 sequence_scc_config()
// =============================================================================

/// Writes `sequence_scc_config()` (AV2 v1.0.0 § 5.4.7,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-7`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_scc_config`].
///
/// `single_picture` (`single_picture_header_flag`) is threaded in from the general header:
/// a single-picture header signals **no** bits (both forces are inferred to the `SELECT_*`
/// sentinel `2`). Otherwise `seq_choose_screen_content_tools` `f(1)` is written (set iff
/// the force is the `2` sentinel), then — when not chosen — the explicit force `f(1)`;
/// and when `seq_force_screen_content_tools > 0`, `seq_choose_integer_mv` `f(1)` (set iff
/// the integer-mv force is `2`) and, when not chosen, the explicit integer-mv force `f(1)`.
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalSequenceValue`] if a single-picture header does not carry the
/// inferred `(2, 2)` forces, if a force value is outside `{0, 1, 2}`, or if the integer-mv
/// force is non-default while the screen-content force is `0` (the parser infers it `2`).
pub fn write_sequence_scc_config(
    writer: &mut BitWriter,
    config: &SequenceSccConfig,
    single_picture: bool,
) -> WriteResult<()> {
    check_scc_encodable(config, single_picture)?;

    if single_picture {
        // Both forces inferred to SELECT_* (no bits signaled).
        return Ok(());
    }

    let sct = config.seq_force_screen_content_tools;
    // seq_choose_screen_content_tools: f(1), set iff sct is the SELECT sentinel.
    let choose_sct = sct == SELECT_SCREEN_CONTENT_TOOLS;
    writer.write_bit(u8::from(choose_sct))?;
    if !choose_sct {
        // seq_force_screen_content_tools: f(1) in {0, 1}.
        writer.write_bits_u8(sct, 1)?;
    }
    if sct > 0 {
        let imv = config.seq_force_integer_mv;
        let choose_imv = imv == SELECT_INTEGER_MV;
        writer.write_bit(u8::from(choose_imv))?;
        if !choose_imv {
            writer.write_bits_u8(imv, 1)?;
        }
    }
    Ok(())
}

/// Validates that `config` is a model the § 5.4.7 parser could have produced.
fn check_scc_encodable(config: &SequenceSccConfig, single_picture: bool) -> WriteResult<()> {
    if single_picture {
        if config.seq_force_screen_content_tools != SELECT_SCREEN_CONTENT_TOOLS
            || config.seq_force_integer_mv != SELECT_INTEGER_MV
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_scc_inferred",
            });
        }
        return Ok(());
    }
    // Non-single-picture: each force is 0, 1, or the SELECT sentinel 2.
    if config.seq_force_screen_content_tools > SELECT_SCREEN_CONTENT_TOOLS {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_force_screen_content_tools",
        });
    }
    if config.seq_force_integer_mv > SELECT_INTEGER_MV {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_force_integer_mv",
        });
    }
    // seq_force_integer_mv is signaled only when seq_force_screen_content_tools > 0;
    // otherwise the parser infers SELECT_INTEGER_MV (2). A different stored value could
    // never have been produced.
    if config.seq_force_screen_content_tools == 0
        && config.seq_force_integer_mv != SELECT_INTEGER_MV
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seq_force_integer_mv",
        });
    }
    Ok(())
}

// =============================================================================
// § 5.4.8 sequence_transform_quant_entropy_config()
// =============================================================================

/// Writes `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_transform_quant_entropy_config`].
///
/// `monochrome` (`Monochrome`) and `single_picture` (`single_picture_header_flag`) are
/// threaded in from the general header and gate the chroma fields, `enable_inter_ddt`,
/// `choose_tcq_per_frame`, and the `enable_avg_cdf`/`avg_cdf_type` inference.
///
/// Field writes (in § 5.4.8 read order): `enable_fsc` `f(1)`; `enable_idtx_intra` `f(1)`
/// (only when `!enable_fsc`); `enable_intra_ist`, `enable_inter_ist` each `f(1)`;
/// `enable_chroma_dctonly` `f(1)` (only when `!monochrome`); `enable_inter_ddt` `f(1)`
/// (only when `!single_picture`); `reduced_tx_part_set` `f(1)`; `enable_cctx` `f(1)`
/// (only when `!monochrome`); `enable_tcq` `f(1)`; `choose_tcq_per_frame` `f(1)` (only when
/// `enable_tcq && !single_picture`); `enable_parity_hiding` `f(1)` (only when
/// `!(enable_tcq && !choose_tcq_per_frame)`); `enable_avg_cdf` `f(1)` + `avg_cdf_type`
/// `f(1)` (only when `!single_picture`, and `avg_cdf_type` only when `enable_avg_cdf`);
/// `separate_uv_delta_q` `f(1)` (only when `!monochrome`); `equal_ac_dc_q` `f(1)`; the
/// `base_y_dc_delta_q` `f(5)` + `y_dc_delta_q_enabled` `f(1)` block (only when
/// `!equal_ac_dc_q`); and the chroma delta-q block (only when `!monochrome`).
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalSequenceValue`] for any inferred field that disagrees with
/// the § 5.4.8 re-derivation (`enable_idtx_intra` must be `true` when `enable_fsc`;
/// chroma fields must be cleared/zeroed when `monochrome`; `enable_inter_ddt` must be
/// `false` when `single_picture`; `choose_tcq_per_frame` must be `false` unless
/// `enable_tcq && !single_picture`; `enable_parity_hiding` must be `false` when
/// `enable_tcq && !choose_tcq_per_frame`; `(enable_avg_cdf, avg_cdf_type)` must be
/// `(true, 1)` when `single_picture`, and `avg_cdf_type` must be `0` when `!enable_avg_cdf`;
/// `base_y_dc_delta_q`/`y_dc_delta_q_enabled` must be zeroed when `equal_ac_dc_q`; and
/// when `equal_ac_dc_q && !monochrome`, `base_uv_dc_delta_q` must mirror
/// `base_uv_ac_delta_q`). [`WriteError::ValueTooWide`] if a `base_*_delta_q` exceeds `f(5)`.
pub fn write_sequence_transform_quant_entropy_config(
    writer: &mut BitWriter,
    config: &SequenceTqEntropyConfig,
    monochrome: bool,
    single_picture: bool,
) -> WriteResult<()> {
    check_tq_entropy_encodable(config, monochrome, single_picture)?;

    writer.write_bit(u8::from(config.enable_fsc))?;
    // enable_idtx_intra: f(1), only when !enable_fsc (inferred 1 when enable_fsc).
    if !config.enable_fsc {
        writer.write_bit(u8::from(config.enable_idtx_intra))?;
    }
    writer.write_bit(u8::from(config.enable_intra_ist))?;
    writer.write_bit(u8::from(config.enable_inter_ist))?;
    if !monochrome {
        writer.write_bit(u8::from(config.enable_chroma_dctonly))?;
    }
    if !single_picture {
        writer.write_bit(u8::from(config.enable_inter_ddt))?;
    }
    writer.write_bit(u8::from(config.reduced_tx_part_set))?;
    if !monochrome {
        writer.write_bit(u8::from(config.enable_cctx))?;
    }
    writer.write_bit(u8::from(config.enable_tcq))?;
    if config.enable_tcq && !single_picture {
        writer.write_bit(u8::from(config.choose_tcq_per_frame))?;
    }
    // enable_parity_hiding is inferred 0 only when (enable_tcq && !choose_tcq_per_frame);
    // it is signaled (f(1)) in every other case.
    let parity_hiding_inferred = config.enable_tcq && !config.choose_tcq_per_frame;
    if !parity_hiding_inferred {
        writer.write_bit(u8::from(config.enable_parity_hiding))?;
    }
    if !single_picture {
        writer.write_bit(u8::from(config.enable_avg_cdf))?;
        if config.enable_avg_cdf {
            writer.write_bits_u8(config.avg_cdf_type, 1)?;
        }
    }
    if !monochrome {
        writer.write_bit(u8::from(config.separate_uv_delta_q))?;
    }
    writer.write_bit(u8::from(config.equal_ac_dc_q))?;
    if !config.equal_ac_dc_q {
        writer.write_bits_u8(config.base_y_dc_delta_q, 5)?;
        writer.write_bit(u8::from(config.y_dc_delta_q_enabled))?;
    }
    if !monochrome {
        if !config.equal_ac_dc_q {
            writer.write_bits_u8(config.base_uv_dc_delta_q, 5)?;
            writer.write_bit(u8::from(config.uv_dc_delta_q_enabled))?;
        }
        writer.write_bits_u8(config.base_uv_ac_delta_q, 5)?;
        writer.write_bit(u8::from(config.uv_ac_delta_q_enabled))?;
        // base_uv_dc_delta_q mirrors base_uv_ac_delta_q when equal_ac_dc_q (no bits).
    }
    Ok(())
}

/// Validates that `config` is a model the § 5.4.8 parser could have produced.
fn check_tq_entropy_encodable(
    config: &SequenceTqEntropyConfig,
    monochrome: bool,
    single_picture: bool,
) -> WriteResult<()> {
    // enable_idtx_intra is inferred 1 when enable_fsc (no bit read).
    if config.enable_fsc && !config.enable_idtx_intra {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_idtx_intra",
        });
    }
    // Chroma fields are inferred 0 for Monochrome.
    if monochrome
        && (config.enable_chroma_dctonly
            || config.enable_cctx
            || config.separate_uv_delta_q
            || config.base_uv_dc_delta_q != 0
            || config.uv_dc_delta_q_enabled
            || config.base_uv_ac_delta_q != 0
            || config.uv_ac_delta_q_enabled)
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "monochrome_chroma_fields",
        });
    }
    // enable_inter_ddt is inferred 0 for single-picture headers.
    if single_picture && config.enable_inter_ddt {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_inter_ddt",
        });
    }
    // choose_tcq_per_frame is inferred 0 unless enable_tcq && !single_picture.
    let choose_tcq_signaled = config.enable_tcq && !single_picture;
    if !choose_tcq_signaled && config.choose_tcq_per_frame {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "choose_tcq_per_frame",
        });
    }
    // enable_parity_hiding is inferred 0 when enable_tcq && !choose_tcq_per_frame.
    if config.enable_tcq && !config.choose_tcq_per_frame && config.enable_parity_hiding {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "enable_parity_hiding",
        });
    }
    // (enable_avg_cdf, avg_cdf_type) is inferred (1, 1) for single-picture headers.
    if single_picture {
        if !config.enable_avg_cdf || config.avg_cdf_type != 1 {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_avg_cdf",
            });
        }
    } else {
        // avg_cdf_type is f(1) only when enable_avg_cdf, else inferred 0.
        if config.enable_avg_cdf {
            check_field_width(u64::from(config.avg_cdf_type), 1)?;
        } else if config.avg_cdf_type != 0 {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "avg_cdf_type",
            });
        }
    }
    // base_y_dc_delta_q / y_dc_delta_q_enabled are inferred 0 when equal_ac_dc_q.
    if config.equal_ac_dc_q && (config.base_y_dc_delta_q != 0 || config.y_dc_delta_q_enabled) {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "base_y_dc_delta_q",
        });
    }
    if !config.equal_ac_dc_q {
        check_field_width(u64::from(config.base_y_dc_delta_q), 5)?;
    }
    // Chroma delta-q block (only when !monochrome).
    if !monochrome {
        if !config.equal_ac_dc_q {
            check_field_width(u64::from(config.base_uv_dc_delta_q), 5)?;
        } else if config.base_uv_dc_delta_q != config.base_uv_ac_delta_q {
            // base_uv_dc_delta_q mirrors base_uv_ac_delta_q when equal_ac_dc_q; a
            // divergent stored value could not have been produced.
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "base_uv_dc_delta_q",
            });
        }
        check_field_width(u64::from(config.base_uv_ac_delta_q), 5)?;
        // uv_dc_delta_q_enabled is only read when !equal_ac_dc_q; when equal_ac_dc_q the
        // parser leaves it false (no bit).
        if config.equal_ac_dc_q && config.uv_dc_delta_q_enabled {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "uv_dc_delta_q_enabled",
            });
        }
    }
    Ok(())
}

// The unit-test and property-test modules live in a sibling file to keep this writer
// source under the advisory source-line limit; `include!` pastes them into this module
// so their `super::*` resolves to the writers and private helpers above.
#[cfg(test)]
include!("seq_config_tests.rs");
