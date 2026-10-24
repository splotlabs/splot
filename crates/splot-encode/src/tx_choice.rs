// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma transform choices of the key-frame coder: the frame-header transform
//! mode a preset codes, and the reconstruction a transform-type trial measures.
//! The RD search prices each choice with the writer's own syntax.

use splot_core::headers::frame::{KeyFrameTxParams, TxMode};
use splot_decode::encoder_syntax::{IntraTileSyntax, TransformBlock, recon_params, tx_log2};
use splot_recon::{BitDepth, PlaneId, reconstruct_transform_block_residual_with_secondary};

use crate::preset::PresetParams;
use crate::tile_writer::{Result, coeffs::adjusted_log2};

/// The frame-header transform choices of `preset`: `TX_MODE_SELECT` when it
/// searches transform partitions, and every intra type (`reduced_tx_set = 0`)
/// when it searches types; otherwise every luma block infers `DCT_DCT`.
pub(crate) fn key_frame_tx_params(preset: &PresetParams) -> KeyFrameTxParams {
    KeyFrameTxParams {
        tx_mode: if preset.tx_partition_search {
            TxMode::Select
        } else {
            TxMode::Largest
        },
        reduced_tx_set: if preset.tx_type_depth == 0 { 2 } else { 0 },
        allow_tcq: false,
    }
}

/// The number of offered luma types `preset` tries: `2 * tx_type_depth + 1`.
pub(crate) const fn tx_type_candidates(preset: &PresetParams) -> usize {
    2 * preset.tx_type_depth as usize + 1
}

/// The row-major reconstruction of block `tx` (with its `tx_type` and
/// `ist`) coded as `levels` over `prediction`, with the decoder's
/// dequantization and inverse transforms.
pub(crate) fn reconstruct(
    syntax: &IntraTileSyntax,
    prediction: &[u16],
    tx: &TransformBlock,
    levels: &[i32],
    (qindex, bit_depth): (u32, BitDepth),
) -> Result<Vec<u16>> {
    let mut recon = prediction.to_vec();
    if levels.iter().all(|&level| level == 0) {
        return Ok(recon);
    }
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    let (adj_w, adj_h) = adjusted_log2(tx.tx_size)?;
    let tcq = syntax.uses_tcq(tx);
    let (params, transform) = recon_params(
        qindex,
        [PlaneId::Y, PlaneId::U, PlaneId::V][tx.plane.min(2)],
        (log2_w, log2_h),
        (tx.tx_type, tcq),
        bit_depth,
    )?;
    let mut dequant = vec![0; 1 << (adj_w + adj_h)];
    let mut residual = vec![0; prediction.len()];
    reconstruct_transform_block_residual_with_secondary(
        prediction,
        &syntax.decoder_quant(tx, levels)?,
        &params,
        &transform,
        syntax.secondary_transform(tx)?.as_ref(),
        &mut dequant,
        &mut residual,
        &mut recon,
    )?;
    Ok(recon)
}
