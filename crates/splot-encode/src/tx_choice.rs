// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma transform choices of the key-frame coder: the frame-header transform
//! mode a preset codes, and the reconstruction a transform-type trial measures.
//! The RD search prices each choice with the writer's own syntax.

use splot_core::headers::frame::{KeyFrameTxParams, TxMode};
use splot_decode::encoder_syntax::{IntraTileSyntax, TransformBlock, recon_params, tx_log2};
use splot_recon::{BitDepth, PlaneId, reconstruct_transform_block_residual_with_secondary};

use crate::preset::{PresetContext, PresetParams};
use crate::tile_writer::{Result, coeffs::adjusted_log2};

/// The frame-header transform choices of `preset` in `context`: `TX_MODE_SELECT` when it
/// searches transform partitions, and every intra type (`reduced_tx_set = 0`)
/// when it searches types; otherwise every luma block infers `DCT_DCT`. Key
/// frames code `allow_intrabc` with the preset's IntraBC search.
pub(crate) fn key_frame_tx_params(
    preset: &PresetParams,
    context: PresetContext,
) -> KeyFrameTxParams {
    KeyFrameTxParams {
        tx_mode: if preset.tx_partition_search.at(context) {
            TxMode::Select
        } else {
            TxMode::Largest
        },
        reduced_tx_set: if preset.tx_type_depth == 0 { 2 } else { 0 },
        allow_intrabc: preset.intrabc,
        allow_local_intrabc: preset.intrabc_local,
        ..KeyFrameTxParams::default()
    }
}

/// The § 5.18.6.2 level `qm_y = qm_u = qm_v` of a frame at `qindex`:
/// `min + qindex * (max - min + 1) / (MaxQ + 1)` over the band of
/// [`PresetParams::quant_matrices`]; `None` codes `using_qmatrix = 0`.
pub(crate) fn qm_level(preset: &PresetParams, qindex: u32, bit_depth: BitDepth) -> Option<u8> {
    let (min, max) = match preset.quant_matrices {
        1 => (10, 14),
        2 => (5, 9),
        3 => (0, 14),
        _ => return None,
    };
    let range = u32::from(crate::rate_control::max_qindex(bit_depth)) + 1;
    Some(min + (qindex.min(range - 1) * u32::from(max - min + 1) / range) as u8)
}

/// The number of offered luma types `preset` tries: `2 * tx_type_depth + 1`.
pub(crate) const fn tx_type_candidates(preset: &PresetParams) -> usize {
    2 * preset.tx_type_depth as usize + 1
}

/// The buffers of [`reconstruct`]: the row-major reconstruction, then the
/// dequantized coefficients and the residual it reuses.
#[derive(Default)]
pub(crate) struct ReconScratch {
    pub(crate) recon: Vec<u16>,
    dequant: Vec<i32>,
    residual: Vec<i32>,
}

/// Writes into `scratch.recon` the row-major reconstruction of block `tx`
/// (with its `tx_type` and `ist`) coded as `levels` over `prediction`, with
/// the decoder's dequantization and inverse transforms.
pub(crate) fn reconstruct(
    syntax: &IntraTileSyntax,
    prediction: &[u16],
    tx: &TransformBlock,
    levels: &[i32],
    (qindex, bit_depth): (u32, BitDepth),
    scratch: &mut ReconScratch,
) -> Result<()> {
    let ReconScratch {
        recon,
        dequant,
        residual,
    } = scratch;
    recon.clear();
    recon.extend_from_slice(prediction);
    if levels.iter().all(|&level| level == 0) {
        return Ok(());
    }
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    let (adj_w, adj_h) = adjusted_log2(tx.tx_size)?;
    let tcq = syntax.uses_tcq(tx);
    let (params, transform) = recon_params(
        qindex,
        [PlaneId::Y, PlaneId::U, PlaneId::V][tx.plane.min(2)],
        (log2_w, log2_h),
        (tx.tx_type, tcq, syntax.uses_ddt()),
        (bit_depth, syntax.quant_matrices()),
    )?;
    dequant.clear();
    dequant.resize(1 << (adj_w + adj_h), 0);
    residual.clear();
    residual.resize(prediction.len(), 0);
    reconstruct_transform_block_residual_with_secondary(
        prediction,
        &syntax.decoder_quant(tx, levels)?,
        &params,
        &transform,
        syntax.secondary_transform(tx)?.as_ref(),
        dequant,
        residual,
        recon,
    )?;
    Ok(())
}
