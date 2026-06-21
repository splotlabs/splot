// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 token accessors for multi-coefficient (eob > 1) blocks
//! (`ENC-COEFF-MULTI-TOKENS`): the coded `all_zero`, a parameterized `eob_pt_16`,
//! and a parameterized low-frequency `coeff_base_eob`. Split out of
//! `coefficient_tokenization` to keep the parent file under the 1000-line source
//! budget. The eob > 1 trace brick composes these with `coeff_base_lf_token` and
//! the per-plane all-zero tokens.

use super::{
    CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax, TX_SIZE_4X4_CTX,
    all_zero_token,
};

/// Returns the AV2 § 5.20.7.27 luma `all_zero` (`txb_skip`) token for a *coded*
/// block (`all_zero == 0`, i.e. the block has nonzero coefficients), at the given
/// coefficient CDF q-context. This is the first `residual()` symbol of a coded
/// luma block; the coefficient symbols follow.
pub(crate) const fn coded_luma_all_zero_token(coeff_cdf_q_ctx: usize) -> CoefficientEntropyToken {
    all_zero_token(coeff_cdf_q_ctx, false)
}

/// Returns the AV2 § 5.20.7.27 `eob_pt_16` token carrying the given EOB-point
/// symbol at the given EOB context (`TileEobPt16Cdf[coeff_cdf_q_ctx][eob_ctx]`).
/// The EOB point selects the end-of-block count: for `eob_pt_16` symbol `s` with
/// no extra bits, `eobPt = s + 1` and (for `eobPt < 3`) `eob = eobPt`, so symbol 0
/// is `eob = 1` (single coefficient) and symbol 1 is `eob = 2`.
pub(crate) const fn eob_pt_16_token(
    coeff_cdf_q_ctx: usize,
    eob_ctx: usize,
    symbol: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt16,
        selector: CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx,
            eob_ctx,
        },
        symbol,
    }
}

/// Returns the AV2 § 5.20.7.27 `eob_extra` token carrying the given binary flag
/// (`TileEobExtraCdf[coeff_cdf_q_ctx]`; the `eob_extra` CDF is indexed only by the
/// coefficient CDF q-context, with no per-symbol/eobPt context — see
/// `DEFAULT_EOB_EXTRA_CDF`). This flag is read only for `eobPt >= 3`; for an
/// `eob_pt_16` block (`eob_pt_16` symbol 2 → eobPt 3) it refines the EOB:
/// `eob = ((1 << (eobPt - 2)) + 1) + (eob_extra << (eobPt - 3)) + eob_extra_bits`,
/// i.e. for eobPt 3 (no `eob_extra_bit` literals, width `eobPt - 3 == 0`)
/// `eob = 3 + eob_extra` (flag 0 → eob 3, flag 1 → eob 4). Mirrors the decoder
/// `read_nonzero_coeff_eob` / `nonzero_coeff_eob` arithmetic.
pub(crate) const fn eob_extra_token(coeff_cdf_q_ctx: usize, flag: bool) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobExtra,
        selector: CoefficientCdfRowSelector::EobExtra { coeff_cdf_q_ctx },
        symbol: flag as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 low-frequency `coeff_base_eob` token for the
/// EOB-position coefficient of base `level`
/// (`TileCoeffBaseLfEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`). The EOB base level is
/// `coeff_base_eob + 1`, so the symbol is `level - 1` (the caller guarantees
/// `level >= 1`; `level == 0` saturates to symbol 0).
pub(crate) const fn coeff_base_lf_eob_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
    level: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx,
        },
        symbol: level.saturating_sub(1),
    }
}

/// Returns the AV2 § 5.20.7.27 low-frequency `coeff_br` (base-range) token at the
/// given § 8.3.2 `coeff_br` context (`TileCoeffBrLfCdf[coeff_cdf_q_ctx][ctx]`). The
/// `coeff_br` symbol refines a coefficient whose `coeff_base` / `coeff_base_eob`
/// level reached its maximum (`LF_NUM_BASE_LEVELS + 1`), adding
/// `symbol` (`0..COEFF_BASE_RANGE`, i.e. `0..=2`) to the level. The caller resolves
/// `ctx` (a constant for the reverse-scan EOB coefficient, whose running `Level[]`
/// is empty; see the general-walk EOB `coeff_br` context constants).
pub(crate) const fn coeff_br_lf_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
    symbol: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBr,
        selector: CoefficientCdfRowSelector::CoeffBrLf {
            coeff_cdf_q_ctx,
            ctx,
        },
        symbol,
    }
}

/// Returns the AV2 § 5.20.7.27 HIGH-frequency `coeff_base_eob` token for the
/// EOB-position coefficient of base `level`
/// (`TileCoeffBaseEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`, the 4-symbol HF table
/// `DEFAULT_COEFF_BASE_EOB_CDF`). The EOB base level is `coeff_base_eob + 1`, so the
/// symbol is `level - 1` — the SAME level mapping as the LF
/// [`coeff_base_lf_eob_token`]; only the CDF table/selector differs (the HF table is
/// 4-symbol vs the LF 6-symbol `DEFAULT_COEFF_BASE_LF_EOB_CDF`). The `coeff_base_eob`
/// `ctx` is LF/HF-independent (`coeff_base_eob_ctx`). The caller guarantees
/// `level >= 1`; `level == 0` saturates to symbol 0.
pub(crate) const fn coeff_base_hf_eob_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
    level: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx,
        },
        symbol: level.saturating_sub(1),
    }
}

/// Returns the AV2 § 5.20.7.27 HIGH-frequency `coeff_br` (base-range) token at the
/// given § 8.3.2 `coeff_br` context (`TileCoeffBrCdf[coeff_cdf_q_ctx][ctx]`, the HF
/// table `DEFAULT_COEFF_BR_CDF`, which has NO transform-size dimension). The
/// `coeff_br` symbol refines a coefficient whose `coeff_base_eob` / `coeff_base`
/// level reached its maximum (`LF_NUM_BASE_LEVELS + 1`), adding `symbol`
/// (`0..COEFF_BASE_RANGE`, i.e. `0..=2`) to the level — the SAME refinement as the LF
/// [`coeff_br_lf_token`]; only the CDF table/selector and the context derivation
/// differ (the HF non-DC luma `coeff_br` context is plain `mag`, with NO `+7`
/// offset). The caller resolves `ctx` (a constant `0` for the reverse-scan EOB
/// coefficient, whose running `Level[]` is empty → `mag == 0`).
pub(crate) const fn coeff_br_hf_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
    symbol: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBr,
        selector: CoefficientCdfRowSelector::CoeffBr {
            coeff_cdf_q_ctx,
            ctx,
        },
        symbol,
    }
}
