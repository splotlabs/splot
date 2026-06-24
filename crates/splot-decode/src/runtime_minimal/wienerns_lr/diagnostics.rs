// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Diagnostic constructors for the Wiener NS loop-restoration runtime frontier.

use splot_core::span::ByteOffset;

use crate::error::DecodeError;
use crate::tile_payload::{MinimalRuntimePartitionFrontierError, TilePartitionTraversalError};

use super::super::{
    AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_FEATURE_ID, AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_MATRIX_ROW,
    AC0EJ3_INTRA_IST_ZERO_FRONTIER_FEATURE_ID, AC0EJ3_INTRA_IST_ZERO_FRONTIER_MATRIX_ROW,
    AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_FEATURE_ID, AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_MATRIX_ROW,
    AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_FEATURE_ID, AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_MATRIX_ROW,
    AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
    AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
    AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_FEATURE_ID, AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_MATRIX_ROW,
    AC0EJ3_LR_SOURCE_READ_FEATURE_ID, AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
    AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID, AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW,
    AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_FEATURE_ID, AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_MATRIX_ROW,
    AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID, AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW,
    unsupported_feature_at,
};

pub(in crate::runtime_minimal) fn transform_tool_residual_frontier(
    reason: &'static str,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match reason {
        "unsupported_dctonly_residual_intra_sec_tx_type"
        | "unsupported_dctonly_residual_intra_ist_context" => (
            "minimal runtime reached active Wiener NS LR, consumed DCT-only residual prelude syntax, read intra IST secondary-transform syntax, and found an active secondary transform; secondary inverse transforms, decoded samples, filtering, output, and reference refresh remain unsupported",
            AC0EJ3_INTRA_IST_ZERO_FRONTIER_MATRIX_ROW,
            AC0EJ3_INTRA_IST_ZERO_FRONTIER_FEATURE_ID,
            "5.20.7.29",
        ),
        _ => (
            "minimal runtime reached active Wiener NS LR, consumed the all_zero decision, staged the nonzero EOB syntax, consumed supported active luma transform_type syntax, and proved this residual is outside the DCT_DCT-only transform-tool subset; non-DCT transforms, CCTX, IST, decoded samples, filtering, output, and reference refresh remain unsupported",
            AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_MATRIX_ROW,
            AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_FEATURE_ID,
            "5.20.7.27",
        ),
    }
}

pub(in crate::runtime_minimal) fn map_wienerns_lr_unit_frontier_error(
    err: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match err {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        _ => wienerns_lr_unit_runtime_error(offset),
    }
}

pub(in crate::runtime_minimal) fn wienerns_lr_unit_runtime_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_active_wienerns_lr_units",
        offset,
        "minimal runtime consumed the supported AV2 §5.20.10.4/§5.20.10.5 frame-level Wiener NS LR unit syntax, retained per-unit selection state, and found at least one unit selecting RESTORE_WIENER_NONSEP, but does not yet apply active loop-restoration reconstruction before output",
        AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW,
        AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID,
        "5.20.10.5",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_source_read_runtime_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_source_read",
        offset,
        "minimal runtime consumed active AV2 §5.20.10.4/§5.20.10.5 frame-level Wiener NS LR unit syntax, retained per-unit selection state, derived active §7.20.1 loop-restoration source-bound facts, and resolved §7.20.2 source-read state for output, Wiener tap, and chroma luma-source coordinates, but does not yet read source sample values or apply §7.20.3 filtering before output",
        AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
        AC0EJ3_LR_SOURCE_READ_FEATURE_ID,
        "7.20.2",
    )
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "old storage-helper diagnostic is retained for the helper-row regression test after the live path advanced"
    )
)]
pub(in crate::runtime_minimal) fn wienerns_lr_classified_wiener_storage_runtime_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_classified_wiener_runtime_storage",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, retained §7.20.1 source-bound and tile-bound facts, resolved §7.20.4 skip-filter classified-luma source-read and LrTxSkip lookup coordinates, resolved the later §7.20.3 source-read state, and has storage-backed FilterClass derivation for decoded CurrFrame/CdefFrame views plus a bounded LrTxSkip grid, but the live ac0ej3 path reaches loop restoration before decoded 10-bit frame buffers and an LrTxSkip grid are retained for filtering; loop-restoration filtering/output/reference refresh is not applied",
        AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_MATRIX_ROW,
        AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_FEATURE_ID,
        "7.20.4",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_runtime_storage_retention_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_runtime_storage_unpopulated",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, retained §7.20.1 source-bound and tile-bound facts, resolved §7.20.4 classified-luma source-read and LrTxSkip lookup coordinates, resolved later §7.20.3 source-read state, has storage-backed FilterClass derivation for decoded CurrFrame/CdefFrame views plus a bounded LrTxSkip grid, and now derives/limit-checks the live active-bit-depth CurrFrame/CdefFrame storage footprint plus the frame-wide LrTxSkip grid shape, but tile reconstruction has not populated decoded frame samples or LrTxSkip values for filtering; loop-restoration filtering/output/reference refresh is not applied",
        AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_MATRIX_ROW,
        AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_FEATURE_ID,
        "7.20.4",
    )
}

#[allow(
    dead_code,
    reason = "live storage-allocation diagnostic is retained for the helper-row regression test after the live path advanced"
)]
pub(in crate::runtime_minimal) fn wienerns_lr_live_storage_allocation_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_storage_unpopulated",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, retained §7.20.1 source-bound and tile-bound facts, resolved §7.20.4 classified-luma source-read and LrTxSkip lookup coordinates, resolved later §7.20.3 source-read state, derived/limit-checked the live active-bit-depth CurrFrame/CdefFrame storage footprint plus the frame-wide LrTxSkip grid shape, and allocated private unpopulated CurrFrame, CdefFrame, and LrTxSkip storage shells, but tile reconstruction has not populated decoded frame samples or LrTxSkip values for storage-backed classification; FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
        AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_MATRIX_ROW,
        AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_FEATURE_ID,
        "7.20.4",
    )
}

#[allow(
    dead_code,
    reason = "historical TX_MODE_SELECT diagnostic is retained for the helper-row regression test after selectable records advanced"
)]
pub(in crate::runtime_minimal) fn wienerns_lr_tx_mode_select_transform_record_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_tx_mode_select_transform_records",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived live storage footprints, and reached the live LrTxSkip transform-record handoff, but the key frame uses TX_MODE_SELECT; deriving LrTxSkip from this stream requires §5.20.6.1 read_tx_size/read_tx_partition records before live samples, FilterClass retention, loop-restoration filtering/output, and reference refresh can run",
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
        "5.20.6.1",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_selectable_transform_record_error_reason(
    offset: ByteOffset,
    reason: &'static str,
) -> DecodeError {
    let (message, matrix_row, feature_id, spec_section) = match reason {
        "unsupported_wienerns_lr_selectable_transform_records_chroma_offset_leaf" => (
            "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived selectable transform records, retained the active non-DCT luma transform type for syntax-only LR tx-skip coefficient derivation, and advanced to a chroma-offset selectable transform-record leaf; chroma residual coordinate handoff, decoded samples, FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
            AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_MATRIX_ROW,
            AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_FEATURE_ID,
            "5.20.3.1",
        ),
        _ => (
            "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived live storage footprints, and reached the TX_MODE_SELECT LrTxSkip transform-record handoff, but a bounded selectable transform-record subcase is still outside the non-FSC intra subset currently wired into live LR storage; decoded samples, FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
            AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW,
            AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID,
            "5.20.6.1",
        ),
    };
    unsupported_feature_at(
        reason,
        offset,
        message,
        matrix_row,
        feature_id,
        spec_section,
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_live_transform_record_handoff_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_transform_records",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived live storage footprints, and reached the live LrTxSkip transform-record handoff, but the tile transform records are outside the fixed-largest subset currently wired into live LR storage; selectable transform records, live samples, FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
        "5.20.7.27",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_live_transform_record_tool_gate_error(
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
        "minimal runtime reached active Wiener NS LR transform-record derivation, but an enabled mode, coefficient, or filtering tool can add unmodelled tile syntax before fixed-largest LR record handoff",
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
        "5.20.5.3",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_mode_symbol_reason(
    reason: &'static str,
) -> &'static str {
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

pub(in crate::runtime_minimal) fn wienerns_lr_mode_literal_reason(
    reason: &'static str,
) -> &'static str {
    match reason {
        "intra_y_second_mode" => {
            "unsupported_wienerns_lr_live_transform_record_y_second_mode_literal"
        }
        "intra_uv_mode_idx" => "unsupported_wienerns_lr_live_transform_record_uv_mode_idx_literal",
        _ => "unsupported_wienerns_lr_live_transform_record_mode_literal",
    }
}

pub(in crate::runtime_minimal) fn wienerns_lr_selectable_live_frame_samples_unpopulated_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_selectable_live_frame_samples_unpopulated",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived live storage footprints, parsed supported TX_MODE_SELECT §5.20.6.1/§5.20.6.3 transform records, derived live LrTxSkip values from tile luma coefficient facts, and populated the live LrTxSkip shell, but decoded CurrFrame and CdefFrame samples are still unpopulated for storage-backed classification; FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
        AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW,
        AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID,
        "7.20.4",
    )
}

pub(in crate::runtime_minimal) fn wienerns_lr_live_frame_samples_unpopulated_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_live_frame_samples_unpopulated",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, derived live storage footprints, derived live LrTxSkip values from fixed-largest tile transform records, and populated the live LrTxSkip shell, but decoded CurrFrame and CdefFrame samples are still unpopulated for storage-backed classification; FilterClass retention, loop-restoration filtering/output, and reference refresh are not applied",
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
        AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
        "7.20.4",
    )
}
