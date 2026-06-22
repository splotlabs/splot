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

// AV2 § 8.3.2 `coeff_base` HIGH-frequency luma 2D band boundaries and offsets
// (mirroring the decoder `CoeffBaseContext` HF branch, `is_lf == false`,
// `class_idx == 0`): `ctx2 = min(ctx, 4)`; `row + col < 6 -> ctx2`,
// `row + col < 8 -> ctx2 + 5`, else `ctx2 + 10`. The 1-D classes map to
// `ctx2 + 15`. There is NO near-DC `magLimit = 5` carve-out and NO `c == 0` / DC
// special case (both are low-frequency only): every HF neighbour clamps to
// `COEFF_BASE_MAG_LIMIT = 3`.
const COEFF_BASE_HF_CTX2_CLAMP: usize = 4;
const COEFF_BASE_HF_2D_BAND0_DIAGONAL: usize = 6;
const COEFF_BASE_HF_2D_BAND1_DIAGONAL: usize = 8;
const COEFF_BASE_HF_2D_BAND1_OFFSET: usize = 5;
const COEFF_BASE_HF_2D_BAND2_OFFSET: usize = 10;
const COEFF_BASE_HF_1D_OFFSET: usize = 15;

/// AV2 § 3 `MAX_BASE_BR_RANGE` = `COEFF_BASE_RANGE (3) + NUM_BASE_LEVELS (2) + 1`
/// (`03-symbols.md`): the `coeff_br` magnitude-sum clamp is `MAX_BASE_BR_RANGE - 1`.
/// A single integer, not a table (the decoder's private
/// `coeff_context::MAX_BASE_BR_RANGE`); kept local here so the encoder does not
/// reach into `splot-decode`.
const MAX_BASE_BR_RANGE: u32 = 6;
/// The 2D luma `coeff_br` neighbour count (`num`) for the § 8.3.2 magnitude sum
/// (`Min((mag + 1) >> 1, 6)`).
const COEFF_BR_LF_LUMA_NUM: usize = 3;
/// The `coeff_br` context low-frequency / non-DC offset (`mag + 7`) of the § 8.3.2
/// `CoeffBrContext::ctx` luma branch.
const COEFF_BR_LF_NON_DC_OFFSET: usize = 7;

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

/// Derives the AV2 § 8.3.2 `coeff_base` HIGH-frequency LUMA CDF context for a
/// high-frequency luma coefficient at scan position `pos`, given the per-block
/// `Level[]` magnitudes (row-major, `txw`-wide; `level[row * txw + col]`).
///
/// This mirrors the decoder's `CoeffBaseContext` HIGH-frequency luma branch
/// (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs:428-440`, AV2
/// § 8.3.2, the `is_lf == false`, `plane == 0` path). It shares the
/// significant-neighbour magnitude-sum loop with [`coeff_base_lf_luma_context`]
/// (`num = SIG_REF_DIFF_OFFSET_NUM = 5`, the `SIG_REF_DIFF_OFFSET[class][0..5]`
/// offsets, saturating geometry, the `flat < level.len()` guard), but DIVERGES from
/// the low-frequency branch in three ways:
///
/// - `magLimit` is `COEFF_BASE_MAG_LIMIT = 3` for EVERY neighbour — there is NO
///   low-frequency near-DC `magLimit = 5` carve-out (the LF `class_idx == 0 || idx
///   < 2` case),
/// - there is NO `c == 0` / DC special case (low-frequency only), and
/// - the context is `ctx = (mag + 1) >> 1`, `ctx2 = min(ctx, 4)`, then for the 2D
///   class (`class_idx == 0`) banded by raster diagonal: `row + col < 6 -> ctx2`,
///   `row + col < 8 -> ctx2 + 5`, else `ctx2 + 10`; the 1-D classes map to
///   `ctx2 + 15`.
///
/// `tx_class` is `0` = `TX_CLASS_2D`, `1` = `TX_CLASS_HORIZ`, `2` = `TX_CLASS_VERT`
/// (out-of-range treated as 2D). It is total and panic-free: geometry is saturating
/// and out-of-range or short-slice neighbour reads contribute `0` (the spec's
/// `refRow < txh && refCol < txw` guard). It is scoped to HIGH-frequency LUMA
/// coefficients; chroma and the low-frequency bands are out of scope here (use
/// [`coeff_base_lf_luma_context`] for the latter).
pub(crate) fn coeff_base_hf_luma_context(
    pos: usize,
    bwl: u32,
    txw: usize,
    txh: usize,
    tx_class: usize,
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
        // HF: every neighbour clamps to `magLimit = 3` (no near-DC carve-out).
        if ref_row < txh && ref_col < txw {
            let flat = ref_row.saturating_mul(txw).saturating_add(ref_col);
            if flat < level.len() {
                let v = level[flat];
                mag += if v < COEFF_BASE_MAG_LIMIT {
                    v
                } else {
                    COEFF_BASE_MAG_LIMIT
                };
            }
        }
        idx += 1;
    }
    let ctx2 = (((mag + 1) >> 1) as usize).min(COEFF_BASE_HF_CTX2_CLAMP);
    if class_idx == 0 {
        if row + col < COEFF_BASE_HF_2D_BAND0_DIAGONAL {
            ctx2
        } else if row + col < COEFF_BASE_HF_2D_BAND1_DIAGONAL {
            ctx2 + COEFF_BASE_HF_2D_BAND1_OFFSET
        } else {
            ctx2 + COEFF_BASE_HF_2D_BAND2_OFFSET
        }
    } else {
        ctx2 + COEFF_BASE_HF_1D_OFFSET
    }
}

/// Derives the AV2 § 8.3.2 `coeff_br` LUMA CDF context for a luma coefficient at
/// scan position `pos`, given the per-block `Level[]` magnitudes (row-major,
/// `txw`-wide; `level[row * txw + col]`) and whether the coefficient is in the
/// low-frequency region (`is_lf`).
///
/// This mirrors the decoder's `CoeffBrContext::ctx` luma branch
/// (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`, AV2 § 8.3.2): it
/// sums the up-to-three neighbour `Level[]` magnitudes (each clamped to
/// `MAX_BASE_BR_RANGE - 1 = 5`) at the first `COEFF_BR_LF_LUMA_NUM (3)`
/// `SIG_REF_DIFF_OFFSET` offsets for the transform class — those first three entries
/// equal the decoder's `Mag_Ref_Offset_With_Tx_Class[class]` — forms
/// `mag = Min((sum + 1) >> 1, 6)`, then offsets it per the decoder's final branch:
///
/// - `pos == 0` (the DC): `class_idx != 0 ? mag + 7 : mag` (the `self.pos == 0`
///   branch — INDEPENDENT of `is_lf`),
/// - a non-DC LOW-frequency luma coefficient (`is_lf == true`): `mag + 7` (the
///   decoder `self.is_lf` branch), and
/// - a non-DC HIGH-frequency luma coefficient (`is_lf == false`): plain `mag` (the
///   decoder final `else { mag }` branch — NO `+7`; this is the single easiest bug to
///   leave in when reusing the LF code).
///
/// `tx_class` is `0` = `TX_CLASS_2D`, `1` = `TX_CLASS_HORIZ`, `2` = `TX_CLASS_VERT`
/// (out-of-range treated as 2D). It is total and panic-free: geometry is saturating
/// and out-of-range or short-slice neighbour reads contribute `0` (the spec's
/// `refRow < txh && refCol < txw` guard). It is scoped to LUMA coefficients (chroma
/// is out of scope for this brick).
pub(crate) fn coeff_br_lf_luma_context(
    pos: usize,
    bwl: u32,
    txw: usize,
    txh: usize,
    tx_class: usize,
    is_lf: bool,
    level: &[u32],
) -> usize {
    let row = pos.checked_shr(bwl).unwrap_or(0);
    let col = pos - row.checked_shl(bwl).unwrap_or(0);
    let class_idx = if tx_class < 3 { tx_class } else { 0 };
    let clamp = MAX_BASE_BR_RANGE - 1;
    let mut mag: u32 = 0;
    let mut idx = 0;
    while idx < COEFF_BR_LF_LUMA_NUM {
        let off = SIG_REF_DIFF_OFFSET[class_idx][idx];
        let ref_row = row.saturating_add(off[0] as usize);
        let ref_col = col.saturating_add(off[1] as usize);
        if ref_row < txh && ref_col < txw {
            let flat = ref_row.saturating_mul(txw).saturating_add(ref_col);
            if flat < level.len() {
                let v = level[flat];
                mag += if v < clamp { v } else { clamp };
            }
        }
        idx += 1;
    }
    let halved = (mag + 1) >> 1;
    let mag = (if halved < 6 { halved } else { 6 }) as usize;
    if pos == 0 {
        // The DC takes the `self.pos == 0` branch of `CoeffBrContext::ctx`: under a
        // 2D class (`class_idx == 0`) the context is `mag`, under a 1-D class it is
        // `mag + 7`. This branch is INDEPENDENT of `is_lf`. Only the 2D DCT_DCT path
        // is exercised by the general walk today; the 1-D branch mirrors the decoder
        // faithfully for later reuse.
        if class_idx != 0 {
            mag + COEFF_BR_LF_NON_DC_OFFSET
        } else {
            mag
        }
    } else if is_lf {
        // A non-DC LOW-frequency luma coefficient takes the decoder `self.is_lf`
        // branch: `mag + 7`.
        mag + COEFF_BR_LF_NON_DC_OFFSET
    } else {
        // A non-DC HIGH-frequency luma coefficient takes the decoder final
        // `else { mag }` branch: plain `mag`, with NO `+7` offset.
        mag
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
