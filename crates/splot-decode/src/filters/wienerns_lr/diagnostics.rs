// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;

use crate::error::{DecodeError, Result};
use crate::pipeline::unsupported_feature_at;

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
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_symbol_range"
        | "unsupported_wienerns_lr_selectable_transform_records_ccso_reference_reuse" => (
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
