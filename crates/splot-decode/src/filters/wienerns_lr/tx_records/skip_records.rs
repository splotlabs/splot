// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;

use crate::bitstream::tile_payload::{
    CoeffContextReset, DecodeBlockFrontier, TileCoeffContextState,
};
use crate::error::Result;

use super::super::wienerns_lr_selectable_transform_record_error_reason;
use super::{SelectableLumaTxRecord, WienerNsLrTxSkipTransformRecord};

const COEFF_CONTEXT_PLANES: [(usize, u32); 3] = [(0, 0), (1, 1), (2, 1)];

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_skipped_selectable_residuals(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    tile_offset: ByteOffset,
) -> Result<()> {
    if luma_records.is_empty() {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_empty_skipped_block",
        ));
    }
    reset_skipped_block_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
    records.try_reserve(luma_records.len()).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_skipped_record_allocation",
        )
    })?;
    records.extend(luma_records.iter().copied().map(skipped_tx_record));
    Ok(())
}

fn skipped_tx_record(record: SelectableLumaTxRecord) -> WienerNsLrTxSkipTransformRecord {
    WienerNsLrTxSkipTransformRecord {
        row: record.row,
        col: record.col,
        rows: record.rows,
        cols: record.cols,
        skip_flag: true,
        eob: 0,
        intra_ist: None,
    }
}

fn reset_skipped_block_coeff_contexts(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let plane_count = 1 + usize::from(frontier.has_chroma) * (COEFF_CONTEXT_PLANES.len() - 1);
    for &(plane, sub) in COEFF_CONTEXT_PLANES.iter().take(plane_count) {
        coeff_ctx
            .reset_block_context_plane(CoeffContextReset {
                plane,
                c: frontier.c,
                r: frontier.r,
                w4: n4w,
                h4: n4h,
                sub_x: sub,
                sub_y: sub,
            })
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_skipped_context_reset",
                )
            })?;
    }
    Ok(())
}
