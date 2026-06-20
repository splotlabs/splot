// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra-path coded DC coefficient token constructors.
//!
//! These are the general-path (`TX_64X64` luma / `TX_32X32` chroma, `eob_pt_1024`)
//! counterparts of the minimal-tier coded-DC token builders in the parent module. They model
//! the symbol sequence the AVM-validated general intra decode path reads for one coded DC
//! coefficient.

use splot_core::coefficient::LF_NUM_BASE_LEVELS;
use splot_recon::PlaneId;

use super::{
    CHROMA_U_TXB_SKIP_CTX_NEUTRAL, COEFF_BASE_LF_EOB_CTX_DC, COEFF_BR_LF_CTX_DC,
    CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax, EOB_CTX_CHROMA,
    EOB_CTX_LUMA_INTRA, INTRA_NON_FSC_TXB_SKIP_BANK, LUMA_PLANE_TYPE, MAX_BASE_EOB_MAGNITUDE,
    TX_SIZE_32X32_CTX, TX_SIZE_64X64_CTX, TXB_SKIP_CTX_NEUTRAL, luma_dc_sign_token,
};
use crate::error::{Error, Result};

/// Returns the ordered AV2 § 5.20.7.27 coded luma DC-only coefficient tokens for the
/// **general** intra decode path's single `TX_64X64` transform: a single nonzero DC of
/// unsigned `magnitude` (`1..=MAX_BASE_BR_MAGNITUDE`) and the given sign.
///
/// The general-path counterpart of `super::luma_dc_coded_tokens`: identical token sequence
/// (`txb_skip == 0`, `eob_pt == 0`, `coeff_base_eob`, optional `coeff_br`, `dc_sign`) except
/// the `txb_skip` and `coeff_base_eob` use the `TX_64X64` `txSzCtx` (`4`) and the EOB symbol
/// is `eob_pt_1024` (the 1024-position size class) rather than `eob_pt_16`. The `coeff_br`
/// and `dc_sign` rows are shared with the minimal tier (their selectors carry no `tx_size`).
pub(crate) fn general_intra_64x64_luma_dc_coded_tokens(
    coeff_cdf_q_ctx: usize,
    magnitude: u32,
    negative: bool,
) -> Result<Vec<CoefficientEntropyToken>> {
    debug_assert!(magnitude >= 1, "coded DC magnitude must be nonzero");
    let needs_br = magnitude > LF_NUM_BASE_LEVELS;
    let len = if needs_br { 5 } else { 4 };
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(len)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded DC coefficient tokens",
        })?;
    // luma `txb_skip == 0` (coded) at the TX_64X64 context.
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size: TX_SIZE_64X64_CTX,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: false as u8,
    });
    // `eob_pt == 0` -> a single EOB coefficient at scan position 0 (no `eob_extra`).
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt1024,
        selector: CoefficientCdfRowSelector::EobPt1024 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_64X64_CTX,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: magnitude.min(LF_NUM_BASE_LEVELS + 1).saturating_sub(1) as u8,
    });
    if needs_br {
        tokens.push(CoefficientEntropyToken {
            syntax: CoefficientTokenSyntax::CoeffBr,
            selector: CoefficientCdfRowSelector::CoeffBrLf {
                coeff_cdf_q_ctx,
                ctx: COEFF_BR_LF_CTX_DC,
            },
            symbol: magnitude.saturating_sub(LF_NUM_BASE_LEVELS + 1) as u8,
        });
    }
    tokens.push(luma_dc_sign_token(coeff_cdf_q_ctx, negative));
    Ok(tokens)
}

/// Returns the three AV2 § 5.20.7.27 coded chroma U DC CDF tokens for the **general** intra
/// decode path's `TX_32X32` chroma transform: a single nonzero DC of unsigned `magnitude`
/// (`1..=MAX_BASE_EOB_MAGNITUDE`, the base tier — no `coeff_br`/golomb).
///
/// The general-path counterpart of `super::chroma_u_dc_coded_coeff_tokens`: identical except
/// the `txb_skip` uses the `TX_32X32` `txSzCtx` (`3`) and the EOB symbol is `eob_pt_1024`. The
/// caller appends the U DC `sign_bit` § 8.2.5 bypass literal (a chroma sign is not CDF-coded).
pub(crate) fn general_intra_32x32_chroma_u_dc_coded_tokens(
    coeff_cdf_q_ctx: usize,
    magnitude: u32,
) -> Result<Vec<CoefficientEntropyToken>> {
    if !(1..=MAX_BASE_EOB_MAGNITUDE).contains(&magnitude) {
        return Err(Error::CoefficientTokenizationUnsupportedChromaMagnitude {
            plane: PlaneId::U,
            magnitude,
            max_magnitude: MAX_BASE_EOB_MAGNITUDE,
        });
    }
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded chroma U DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
            tx_size: TX_SIZE_32X32_CTX,
            ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: false as u8,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt1024,
        selector: CoefficientCdfRowSelector::EobPt1024 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_CHROMA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEobUv {
            coeff_cdf_q_ctx,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: (magnitude - 1) as u8,
    });
    Ok(tokens)
}
