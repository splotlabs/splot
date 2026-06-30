// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_recon::PlaneId;

use crate::error::Result;
use crate::tile_payload::{
    CoeffContextReset, DecodeBlockFrontier, LumaCoeffBlock, TileCoeffContextState,
};

use super::super::fixed_largest_420_chroma_tx_size_from_luma_4x4;
use super::super::recon::{SelectableReconContext, WienerNsLrReconSink};
use super::super::wienerns_lr_selectable_transform_record_error_reason;
use super::{SelectableLumaTxRecord, WienerNsLrTxSkipTransformRecord, mi_to_sample};

const COEFF_CONTEXT_PLANES: [(usize, u32); 3] = [(0, 0), (1, 1), (2, 1)];

fn skipped_coeff_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        plane_tx_type: 0,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_skipped_selectable_residuals(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    luma_records: &[SelectableLumaTxRecord],
    records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
    sink: Option<&mut WienerNsLrReconSink<u16>>,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    if luma_records.is_empty() {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_empty_skipped_block",
        ));
    }
    reset_skipped_block_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
    if let Some(sink) = sink
        && !recon.is_intrabc
    {
        let zero = skipped_coeff_block();
        for record in luma_records.iter().copied() {
            sink.reconstruct_luma_transform(
                record.col,
                record.row,
                record.tx_size,
                &zero,
                recon.leaf_y_mode,
                recon.directional_luma,
                recon.mrl_index,
                recon.angle_delta_y,
                recon.qindex,
                recon.luma_use_tcq,
                recon.fsc_mode,
                false,
                tile_offset,
            )?;
        }
        reconstruct_skipped_chroma(sink, frontier, n4w, n4h, recon, tile_offset)?;
    }
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

fn reconstruct_skipped_chroma(
    sink: &mut WienerNsLrReconSink<u16>,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    recon: SelectableReconContext,
    tile_offset: ByteOffset,
) -> Result<()> {
    if !frontier.has_chroma || n4w >= 32 || n4h >= 32 {
        return Ok(());
    }
    let Some(chroma_tx) = fixed_largest_420_chroma_tx_size_from_luma_4x4(n4w, n4h) else {
        return Ok(());
    };
    let chroma_x = mi_to_sample(frontier.c, tile_offset)? / 2;
    let chroma_y = mi_to_sample(frontier.r, tile_offset)? / 2;
    let zero = skipped_coeff_block();
    for plane in [PlaneId::U, PlaneId::V] {
        sink.reconstruct_chroma_transform(
            plane,
            chroma_tx,
            chroma_x,
            chroma_y,
            &zero,
            recon.chroma_mode,
            recon.qindex,
            tile_offset,
        )?;
    }
    Ok(())
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
