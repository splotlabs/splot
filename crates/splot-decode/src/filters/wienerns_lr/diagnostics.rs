// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;

use crate::error::{DecodeError, Result};
use crate::pipeline::unsupported_feature_at;

pub(crate) fn selectable_missing_quantization_error(_offset: ByteOffset) -> DecodeError {
    crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into()
}

pub(crate) fn selectable_symbol_read_error(_offset: ByteOffset) -> DecodeError {
    crate::error::DecodeHeaderStateError::SelectableTransformRecordReadFailed.into()
}

pub(crate) fn wienerns_lr_selectable_transform_record_error_reason(
    offset: ByteOffset,
    reason: &'static str,
) -> DecodeError {
    let (message, spec_section) = match reason {
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv" => (
            "IntrABC transform records failed to read the §5.20.5.4 NEWMV block-vector MVD; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "5.20.5.4",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack" => (
            "IntrABC transform records selected a ref_mv_idx beyond the derived §7.12.2 MV stack; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "7.12.2",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size"
        | "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry" => (
            "IntrABC block-vector syntax produced luma current-frame prediction geometry outside the bounded frontier subset; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "6.19.7.12",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_ccso_grid_overflow"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_bounds"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_reference_reuse" => (
            "Per-block §5.20.10.2 CCSO ccso_blk parsing hit an internal CCSO-grid inconsistency; decoded samples, loop-restoration filtering/output, and reference refresh are unsupported",
            "5.20.10.2",
        ),
        "unsupported_wienerns_lr_selectable_transform_records_gdf_grid"
        | "unsupported_wienerns_lr_selectable_transform_records_gdf_symbol" => (
            "Per-block §5.20.10.3 GDF use_gdf parsing hit an invalid symbol or GDF-grid state",
            "5.20.10.3",
        ),
        _ => (
            "TX_MODE_SELECT LrTxSkip handoff reached a selectable transform-record subcase outside the supported non-FSC intra subset; decoded samples, FilterClass retention, filtering, output, and reference refresh are unsupported",
            "5.20.6.1",
        ),
    };
    unsupported_feature_at(reason, offset, message, spec_section)
}

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
