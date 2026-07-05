// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;

use crate::bitstream::tile_payload::{TilePartitionFrontierError, TilePartitionTraversalError};
use crate::error::{DecodeError, Result};

use crate::pipeline::unsupported_feature_at;

pub(crate) fn transform_tool_residual_frontier(
    reason: &'static str,
) -> (&'static str, &'static str) {
    match reason {
        "unsupported_dctonly_residual_intra_sec_tx_type"
        | "unsupported_dctonly_residual_intra_ist_context" => (
            "Active Wiener NS LR parsed DCT-only residual prelude and intra IST syntax, but active secondary transforms are unsupported before decoded samples, filtering, output, and reference refresh",
            "5.20.7.29",
        ),
        _ => (
            "Active Wiener NS LR parsed nonzero residual syntax and a supported luma transform_type, but the residual is outside the DCT_DCT-only subset; non-DCT transforms, CCTX, IST, decoded samples, filtering, output, and reference refresh are unsupported",
            "5.20.7.27",
        ),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn map_wienerns_lr_unit_frontier_error(
    err: TilePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match err {
        TilePartitionFrontierError::Limit(source)
        | TilePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(source)) => {
            DecodeError::Limit { source }
        }
        _ => wienerns_lr_unit_runtime_error(offset),
    }
}

pub(crate) fn wienerns_lr_unit_runtime_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_active_wienerns_lr_units",
        offset,
        "AV2 §5.20.10.4/§5.20.10.5 Wiener NS LR unit syntax selected RESTORE_WIENER_NONSEP; loop-restoration reconstruction before output is unsupported",
        "5.20.10.5",
    )
}

pub(crate) fn wienerns_lr_source_read_runtime_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_source_read",
        offset,
        "Active Wiener NS LR retained per-unit selection state and source-bound facts, and resolved §7.20.2 source-read state for output, Wiener tap, and chroma luma-source coordinates; source sample values and §7.20.3 filtering are unsupported",
        "7.20.2",
    )
}

pub(crate) fn wienerns_lr_runtime_storage_retention_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_runtime_storage_unpopulated",
        offset,
        "Active Wiener NS LR derives the active-bit-depth CurrFrame/CdefFrame storage footprint and LrTxSkip grid shape, but tile reconstruction has not populated decoded frame samples or LrTxSkip values; loop-restoration filtering/output is not applied",
        "7.20.4",
    )
}

#[allow(
    dead_code,
    reason = "regression test keeps the historical live-storage row"
)]
pub(crate) fn wienerns_lr_live_storage_allocation_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_storage_unpopulated",
        offset,
        "Active Wiener NS LR allocated private unpopulated CurrFrame, CdefFrame, and LrTxSkip storage shells, but tile reconstruction has not populated decoded frame samples or LrTxSkip values; FilterClass retention and loop-restoration filtering/output are unsupported",
        "7.20.4",
    )
}

#[allow(
    dead_code,
    reason = "regression test keeps the historical TX_MODE_SELECT row"
)]
pub(crate) fn wienerns_lr_tx_mode_select_transform_record_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_tx_mode_select_transform_records",
        offset,
        "Active Wiener NS LR reached the LrTxSkip handoff with TX_MODE_SELECT; deriving LrTxSkip needs §5.20.6.1 read_tx_size/read_tx_partition records before live samples and filtering",
        "5.20.6.1",
    )
}

pub(crate) fn wienerns_lr_selectable_transform_record_error_reason(
    offset: ByteOffset,
    reason: &'static str,
) -> DecodeError {
    let (message, spec_section) = match reason {
        "unsupported_wienerns_lr_selectable_transform_records_chroma_offset_leaf" => (
            "Selectable LR transform records reached a chroma-offset leaf; chroma residual coordinate handoff and decoded samples are unsupported before FilterClass retention, loop-restoration filtering/output, and reference refresh",
            "5.20.3.1",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc" => (
            "Selectable LR transform records reached §5.20.5.3 use_intrabc mode info; IntrABC mode info, block-vector prediction, decoded samples, FilterClass retention, loop-restoration filtering/output, and reference refresh are unsupported",
            "5.20.5.3",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv" => (
            "IntrABC transform records parsed the bounded mode-info prelude but stopped before §5.20.5.4 NEWMV block-vector syntax; IntrABC prediction, decoded samples, filtering, output, and reference refresh are unsupported",
            "5.20.5.4",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_prediction" => (
            "IntrABC transform records parsed block-vector syntax but current-frame IntrABC prediction is unsupported before decoded samples, loop-restoration filtering/output, and reference refresh",
            "5.20.7.13",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack" => (
            "IntrABC transform records need a §7.12.2 MV stack candidate beyond the bounded subset; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "7.12.2",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_currframe_samples" => (
            "IntrABC transform records derived §7.13.3.18 luma prediction geometry, but decoded CurrFrame samples are unpopulated; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "7.13.3.18",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_nonskip_residual" => (
            "IntrABC transform records decoded a skip block, then reached a NON-skip block using §5.20.7.23 residual syntax on the inter/IntrABC path; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "5.20.7.23",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry" => (
            "IntrABC block-vector syntax produced luma current-frame prediction geometry outside the bounded frontier subset; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "6.19.7.12",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_ccso_grid_overflow"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range" => (
            "Per-block §5.20.10.2 CCSO ccso_blk parsing hit an internal CCSO-grid inconsistency; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "5.20.10.2",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_bitstream_desync" => (
            "Selectable transform-record parsing consumed past the tile payload end (§8.2.4 SymbolMaxBits < -14); the decoder fails closed before phantom zero-padded reads, decoded samples, filtering, output, and reference refresh",
            "8.2.4",
        ),
        _ => (
            "TX_MODE_SELECT LrTxSkip handoff reached a selectable transform-record subcase outside the supported non-FSC intra subset; decoded samples, FilterClass retention, filtering, output, and reference refresh are unsupported",
            "5.20.6.1",
        ),
    };
    unsupported_feature_at(reason, offset, message, spec_section)
}

/// `get_seq_sb_size()` (AV2 § 5.18.2) for the selectable transform-record frontier:
/// the § 5.18.2 intra-capped superblock used by the per-block delta-Q, IntrABC, and
/// CCSO grid derivations (a single source for the shared `partition` lookup).
pub(crate) fn intra_capped_seq_sb_size(
    sequence: &SequenceHeader,
    tile_offset: ByteOffset,
) -> Result<SuperblockSize> {
    let partition = sequence.partition.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config",
        )
    })?;
    Ok(partition.seq_sb_size())
}

pub(crate) fn wienerns_lr_live_transform_record_handoff_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_transform_records",
        offset,
        "Active Wiener NS LR reached the live LrTxSkip handoff, but tile transform records are outside the fixed-largest subset; selectable transform records, live samples, FilterClass retention, filtering, output, and reference refresh are unsupported",
        "5.20.7.27",
    )
}

pub(crate) fn wienerns_lr_live_transform_record_tool_gate_error(
    offset: ByteOffset,
    tool: &'static str,
) -> DecodeError {
    let reason = match tool {
        "tile_grid" => "unsupported_wienerns_lr_live_transform_record_tile_grid",
        "screen_content_tools" => {
            "unsupported_wienerns_lr_live_transform_record_screen_content_tools"
        }
        "intra_tool" => "unsupported_wienerns_lr_live_transform_record_intra_tool",
        "transform_tool" => "unsupported_wienerns_lr_live_transform_record_transform_tool",
        "frame_tool" => "unsupported_wienerns_lr_live_transform_record_frame_tool",
        _ => "unsupported_wienerns_lr_live_transform_record_unsupported_tool",
    };
    unsupported_feature_at(
        reason,
        offset,
        "Active Wiener NS LR transform-record derivation found an enabled tool that may add unmodelled tile syntax before fixed-largest LR record handoff",
        "5.20.5.3",
    )
}

pub(crate) fn wienerns_lr_mode_symbol_reason(reason: &'static str) -> &'static str {
    match reason {
        "intra_y_mode_set" => "unsupported_wienerns_lr_live_transform_record_y_mode_set_symbol",
        "intra_y_mode_index" => "unsupported_wienerns_lr_live_transform_record_y_mode_index_symbol",
        "intra_y_mode_offset" => {
            "unsupported_wienerns_lr_live_transform_record_y_mode_offset_symbol"
        }
        "intra_is_cfl" => "unsupported_wienerns_lr_live_transform_record_is_cfl_symbol",
        "intra_cfl_index" => "unsupported_wienerns_lr_live_transform_record_cfl_index_symbol",
        "intra_cfl_alpha_signs" => {
            "unsupported_wienerns_lr_live_transform_record_cfl_alpha_signs_symbol"
        }
        "intra_cfl_alpha_u" => "unsupported_wienerns_lr_live_transform_record_cfl_alpha_u_symbol",
        "intra_cfl_alpha_v" => "unsupported_wienerns_lr_live_transform_record_cfl_alpha_v_symbol",
        "intra_cfl_mhccp" => "unsupported_wienerns_lr_live_transform_record_cfl_mhccp_symbol",
        "intra_cfl_mh_dir" => "unsupported_wienerns_lr_live_transform_record_cfl_mh_dir_symbol",
        "intra_fsc_mode" => "unsupported_wienerns_lr_live_transform_record_fsc_mode_symbol",
        "intra_mrl_index" => "unsupported_wienerns_lr_live_transform_record_mrl_index_symbol",
        "intra_mrl_sec_index" => {
            "unsupported_wienerns_lr_live_transform_record_mrl_sec_index_symbol"
        }
        "intra_uv_mode" => "unsupported_wienerns_lr_live_transform_record_uv_mode_symbol",
        _ => "unsupported_wienerns_lr_live_transform_record_mode_symbol",
    }
}

pub(crate) fn wienerns_lr_mode_literal_reason(reason: &'static str) -> &'static str {
    match reason {
        "intra_y_second_mode" => {
            "unsupported_wienerns_lr_live_transform_record_y_second_mode_literal"
        }
        "intra_uv_mode_idx" => "unsupported_wienerns_lr_live_transform_record_uv_mode_idx_literal",
        _ => "unsupported_wienerns_lr_live_transform_record_mode_literal",
    }
}

pub(crate) fn wienerns_lr_selectable_live_frame_samples_unpopulated_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_selectable_live_frame_samples_unpopulated",
        offset,
        "Active Wiener NS LR parsed supported TX_MODE_SELECT §5.20.6.1/§5.20.6.3 transform records, derived LrTxSkip from tile luma coefficient facts, and populated the live LrTxSkip shell, but CurrFrame and CdefFrame samples are still unpopulated for storage-backed classification; FilterClass retention, loop-restoration filtering/output, and reference refresh are unsupported",
        "7.20.4",
    )
}

pub(crate) fn wienerns_lr_live_frame_samples_unpopulated_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_frame_samples_unpopulated",
        offset,
        "Active Wiener NS LR derived LrTxSkip from fixed-largest tile transform records and populated the live LrTxSkip shell, but CurrFrame and CdefFrame samples are still unpopulated for storage-backed classification; FilterClass retention, loop-restoration filtering/output, and reference refresh are unsupported",
        "7.20.4",
    )
}
