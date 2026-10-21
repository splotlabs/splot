// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Placeholder luma transform choice of the key-frame coder, until the RD search
//! prices the real syntax.
//!
//! A candidate costs `SSE * 2^16 + lambda_q16 * bits`: the SSE of its
//! reconstruction against the source and a rate proxy in bits. The type search
//! tries the first [`tx_type_candidates`] offered types (most probable first);
//! the partition search codes each partition's luma units in a trial and rolls
//! the trial back.

use splot_core::headers::frame::{KeyFrameTxParams, TxMode};
use splot_decode::encoder_syntax::{
    IntraModes, IntraTileSyntax, LeafBlock, TransformBlock, TxPartition, TxType, recon_params,
    tx_log2,
};
use splot_recon::{BitDepth, PlaneId, reconstruct_transform_block_residual};

use crate::kernels::{BlockDims, kernels};
use crate::preset::PresetParams;
use crate::tile_writer::{Result, TileWriterError, TransformDecision, coeffs::adjusted_log2};

/// Proxy bits of a transform block: its `all_zero` flag and its end of block.
const BLOCK_BITS: u64 = 2;
/// Proxy bits of one nonzero level, before its magnitude (tuned for BD-rate on CIF content).
const LEVEL_BITS: u64 = 1;
/// Proxy bits of a luma type other than `DCT_DCT`, the most probable one.
const TYPE_BITS: u64 = 2;

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
    }
}

/// The number of offered luma types `preset` tries: `2 * tx_type_depth + 1`.
pub(crate) const fn tx_type_candidates(preset: &PresetParams) -> usize {
    2 * preset.tx_type_depth as usize + 1
}

/// The cheapest `(cost, decision)` of `code` over `tx_types`.
pub(crate) fn cheapest<'a>(
    tx_types: impl IntoIterator<Item = &'a TxType>,
    mut code: impl FnMut(TxType) -> Result<(u64, TransformDecision)>,
) -> Result<(u64, TransformDecision)> {
    let mut best: Option<(u64, TransformDecision)> = None;
    for &tx_type in tx_types {
        let candidate = code(tx_type)?;
        if best.as_ref().is_none_or(|(cost, _)| candidate.0 < *cost) {
            best = Some(candidate);
        }
    }
    best.ok_or(TileWriterError::Shape("no luma transform type"))
}

/// The cost of coding `tx` (with its `tx_type`) as `levels` over `prediction`;
/// `source` holds the block's samples at `stride`.
pub(crate) fn decision_cost(
    (source, stride): (&[u16], usize),
    prediction: &[u16],
    tx: &TransformBlock,
    levels: &[i32],
    (qindex, bit_depth, lambda_q16): (u32, BitDepth, u64),
) -> Result<u64> {
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    let (adj_w, adj_h) = adjusted_log2(tx.tx_size)?;
    let samples = prediction.len();
    let mut recon = prediction.to_vec();
    let nonzero = levels.iter().filter(|&&level| level != 0).count();
    if nonzero != 0 {
        let (params, transform) =
            recon_params(qindex, PlaneId::Y, (log2_w, log2_h), tx.tx_type, bit_depth)?;
        let mut dequant = vec![0; 1 << (adj_w + adj_h)];
        let mut residual = vec![0; samples];
        reconstruct_transform_block_residual(
            prediction,
            levels,
            &params,
            &transform,
            &mut dequant,
            &mut residual,
            &mut recon,
        )?;
    }
    let width = 1 << log2_w;
    let dims = BlockDims::new(width, 1 << log2_h)?;
    let sse = kernels().sse(source, stride, &recon, width, dims);
    let magnitude: u64 = levels
        .iter()
        .filter(|&&level| level != 0)
        .map(|level| u64::from(2 * level.unsigned_abs().ilog2()))
        .sum();
    let type_bits = if nonzero > 1 && tx.tx_type != TxType::DctDct {
        TYPE_BITS
    } else {
        0
    };
    let bits = BLOCK_BITS + LEVEL_BITS * nonzero as u64 + magnitude + type_bits;
    Ok((sse << 16).saturating_add(lambda_q16.saturating_mul(bits)))
}

/// The luma transform partition of `leaf` under `modes` whose units cost least
/// when each is coded by `code_luma`, which returns the cheapest
/// `(cost, decision)` of a luma block among the offered types.
pub(crate) fn cheapest_partition(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    modes: IntraModes,
    mut code_luma: impl FnMut(&TransformBlock, &[u16], &[TxType]) -> Result<(u64, TransformDecision)>,
) -> Result<TxPartition> {
    let options = syntax.luma_tx_partitions(leaf)?;
    if options.len() < 2 {
        return Ok(TxPartition::None);
    }
    let (width, height) = leaf.dimensions()?;
    let snapshot = syntax.snapshot((leaf.col4() * 4, leaf.row4() * 4, width, height))?;
    let mut best = (u64::MAX, TxPartition::None);
    for partition in options {
        let mut cost = 0u64;
        for (index, tx) in syntax
            .plan_leaf_partitioned(leaf, modes, partition)?
            .iter()
            .enumerate()
        {
            if tx.plane != 0 {
                continue;
            }
            let prediction = syntax.predict(index)?;
            let tx_types = syntax.allowed_luma_tx_types(tx)?;
            let (unit_cost, decision) = code_luma(tx, &prediction, &tx_types)?;
            syntax.reconstruct_as(index, decision.tx_type, &decision.levels)?;
            cost = cost.saturating_add(unit_cost);
        }
        syntax.restore(&snapshot)?;
        if cost < best.0 {
            best = (cost, partition);
        }
    }
    Ok(best.1)
}
