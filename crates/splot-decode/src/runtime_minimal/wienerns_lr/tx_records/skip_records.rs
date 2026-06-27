// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Skipped residual record helpers for the selectable transform-record handoff.

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

/// An `all_zero` (`txb_skip`) coefficient block: no residual, so reconstruction
/// writes the bare §7.13.2 DC prediction.
fn skipped_coeff_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
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
    // Reconstruct each skipped luma transform as a flat §7.13.2 DC prediction
    // (zero residual) so later blocks' neighbour reads are spec-correct (gated to
    // DC inside the sink). A skipped block carries no coefficients.
    if let Some(sink) = sink {
        let zero = skipped_coeff_block();
        for record in luma_records.iter().copied() {
            sink.reconstruct_luma_transform(
                record.col,
                record.row,
                record.tx_size,
                &zero,
                recon.leaf_y_mode,
                recon.qindex,
                recon.luma_use_tcq,
                recon.fsc_mode,
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
    records.extend(
        luma_records
            .iter()
            .copied()
            .map(|record| WienerNsLrTxSkipTransformRecord {
                row: record.row,
                col: record.col,
                rows: record.rows,
                cols: record.cols,
                skip_flag: true,
                eob: 0,
                intra_ist: None,
            }),
    );
    Ok(())
}

/// Reconstructs the skipped block's §6.4.1 4:2:0 chroma U/V planes as flat DC
/// predictions (zero residual) for the single-chroma-group (non-`large_chunks`)
/// case. A multi-chroma-group skipped block (`n4w >= 32` or `n4h >= 32`) is left
/// unreconstructed for chroma — the verified region excludes it rather than
/// emitting a partial chroma group.
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
    // §6.4.1 4:2:0 chroma origin: half the luma sample origin.
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
    let plane_count = if frontier.has_chroma { 3 } else { 1 };
    for plane in 0..plane_count {
        let (sub_x, sub_y) = if plane == 0 { (0, 0) } else { (1, 1) };
        coeff_ctx
            .reset_block_context_plane(CoeffContextReset {
                plane,
                c: frontier.c,
                r: frontier.r,
                w4: n4w,
                h4: n4h,
                sub_x,
                sub_y,
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
