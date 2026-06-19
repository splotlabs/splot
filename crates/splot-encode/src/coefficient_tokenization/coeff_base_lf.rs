// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 / § 5.20.7.27 non-EOB `coeff_base` low-frequency luma building
//! blocks for multi-coefficient (eob > 1) blocks: the low-frequency context
//! derivation (`ENC-COEFF-BASE-LF-CONTEXT`) and the non-EOB `coeff_base` token
//! (`ENC-COEFF-BASE-LF-TOKEN`). Split out of `coefficient_tokenization` to keep the
//! parent file under the 1000-line source budget.

use splot_core::coefficient::{LF_SIG_COEF_CONTEXTS_2D, SIG_REF_DIFF_OFFSET_NUM};
use splot_core::tables::conversion::SIG_REF_DIFF_OFFSET;

use super::{
    CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax, TX_SIZE_4X4_CTX,
};

// AV2 § 8.3.2 `coeff_base` low-frequency `magLimit` clamps (mirroring the decoder
// `CoeffBaseContext`): 5 for the near-DC low-frequency samples, 3 otherwise. The
// luma significant-neighbour count (`SIG_REF_DIFF_OFFSET_NUM`) and the 2D
// context-band base (`LF_SIG_COEF_CONTEXTS_2D`) are shared from `splot-core`.
const COEFF_BASE_LF_NEAR_DC_MAG_LIMIT: u32 = 5;
const COEFF_BASE_MAG_LIMIT: u32 = 3;

/// Derives the AV2 § 8.3.2 `coeff_base` low-frequency LUMA CDF context for a
/// low-frequency luma coefficient at scan position `pos`, given the per-block
/// `Level[]` magnitudes (row-major, `txw`-wide; `level[row * txw + col]`).
///
/// This mirrors the decoder's `CoeffBaseContext` low-frequency luma branch
/// (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`, AV2 § 8.3.2): it
/// sums the significant-neighbour magnitudes (each clamped by `magLimit`) at the
/// `SIG_REF_DIFF_OFFSET` offsets for the transform class, forms
/// `ctx = (mag + 1) >> 1`, and maps it into the low-frequency luma context bands.
/// `tx_class` is `0` = `TX_CLASS_2D`, `1` = `TX_CLASS_HORIZ`, `2` = `TX_CLASS_VERT`
/// (out-of-range treated as 2D); `c` is the scan index.
///
/// It is total and panic-free: geometry is saturating and out-of-range or
/// short-slice neighbour reads contribute `0` (the spec's
/// `refRow < height && refCol < width` guard). It is scoped to low-frequency LUMA
/// coefficients; chroma, the parity-hidden DC override, and the high-frequency
/// (non-LF) bands are out of scope for this brick.
pub(crate) fn coeff_base_lf_luma_context(
    pos: usize,
    bwl: u32,
    txw: usize,
    txh: usize,
    tx_class: usize,
    c: usize,
    level: &[u32],
) -> usize {
    let row = pos.checked_shr(bwl).unwrap_or(0);
    let col = pos - row.checked_shl(bwl).unwrap_or(0);
    let class_idx = if tx_class < 3 { tx_class } else { 0 };
    let mut mag: u32 = 0;
    let mut idx = 0;
    while idx < SIG_REF_DIFF_OFFSET_NUM {
        let off = SIG_REF_DIFF_OFFSET[class_idx][idx];
        let ref_row = row.saturating_add(off[0] as usize);
        let ref_col = col.saturating_add(off[1] as usize);
        // `magLimit` is 5 for the low-frequency near-DC samples, else 3.
        let mag_limit = if class_idx == 0 || idx < 2 {
            COEFF_BASE_LF_NEAR_DC_MAG_LIMIT
        } else {
            COEFF_BASE_MAG_LIMIT
        };
        if ref_row < txh && ref_col < txw {
            let flat = ref_row.saturating_mul(txw).saturating_add(ref_col);
            if flat < level.len() {
                let v = level[flat];
                mag += if v < mag_limit { v } else { mag_limit };
            }
        }
        idx += 1;
    }
    let ctx = ((mag + 1) >> 1) as usize;
    if class_idx == 0 {
        if c == 0 {
            ctx.min(8)
        } else if row + col < 2 {
            ctx.min(6) + 9
        } else {
            ctx.min(4) + 16
        }
    } else {
        // TX_CLASS_HORIZ (1) keys on col; TX_CLASS_VERT (2) keys on row.
        let lidx = if class_idx == 1 { col } else { row };
        if lidx == 0 {
            LF_SIG_COEF_CONTEXTS_2D + ctx.min(6)
        } else {
            LF_SIG_COEF_CONTEXTS_2D + 7 + ctx.min(4)
        }
    }
}

/// Returns the AV2 § 5.20.7.27 non-EOB `coeff_base` token for a low-frequency luma
/// coefficient: the base `level` (a non-EOB base level equals the decoded symbol)
/// coded with the `TileCoeffBaseLfCdf[coeff_cdf_q_ctx][4x4][ctx][tcq_ctx]` row. The
/// `ctx` is the § 8.3.2 low-frequency context (see `coeff_base_lf_luma_context`);
/// `tcq_ctx` is `(tcqState >> 1) & 1`, which is 0 when TCQ is off.
pub(crate) const fn coeff_base_lf_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
    tcq_ctx: usize,
    level: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBase,
        selector: CoefficientCdfRowSelector::CoeffBaseLf {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx,
            tcq_ctx,
        },
        symbol: level,
    }
}
