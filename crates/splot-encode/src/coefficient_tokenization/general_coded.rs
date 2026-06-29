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
    TX_SIZE_16X16_CTX, TX_SIZE_32X32_CTX, TX_SIZE_64X64_CTX, TXB_SKIP_CTX_NEUTRAL,
    luma_dc_sign_token,
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

/// Returns the ordered AV2 § 5.20.7.27 coded luma DC-only coefficient tokens for the
/// **general** intra decode path's single `TX_16X16` transform: a single nonzero DC of
/// unsigned `magnitude` (`1..=MAX_BASE_BR_MAGNITUDE`) and the given sign.
///
/// The `TX_16X16` counterpart of [`general_intra_64x64_luma_dc_coded_tokens`]: identical token
/// sequence (`txb_skip == 0`, `eob_pt == 0`, `coeff_base_eob`, optional `coeff_br`, `dc_sign`)
/// except the `txb_skip` and `coeff_base_eob` use the `TX_16X16` `txSzCtx` (`2`,
/// [`TX_SIZE_16X16_CTX`]) and the EOB symbol is `eob_pt_256` (the 256-position size class:
/// `eobMultisize = Min(4, 5) + Min(4, 5) - 4 = 4`) rather than `eob_pt_16`/`eob_pt_1024`. For
/// eob == 1 the `eob_pt_256` symbol is `0` (eobPt 1 → eob 1: no `eob_pt_extra`/`eob_extra` simple
/// path). The DC is at scan position 0 (`row + col == 0 < 4`, the size-independent low-frequency
/// region), so its `coeff_base_eob` reads the low-frequency EOB bank
/// (`TileCoeffBaseLfEobCdf[q][TX_16X16][ctx 0]`) — mirroring the decoder's `coeff_base_eob_ctx(c =
/// 0) == 0` (`SIG_COEF_CONTEXTS_EOB - 4`). The `coeff_br` and `dc_sign` rows are shared with the
/// minimal/64x64 tiers (their selectors carry no `tx_size`). The arithmetic is saturating so the
/// function is total for any input; `tokenize_coefficients` validates the magnitude bound.
pub(crate) fn general_intra_16x16_luma_dc_coded_tokens(
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
            context: "general coded 16x16 DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size: TX_SIZE_16X16_CTX,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: false as u8,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt256,
        selector: CoefficientCdfRowSelector::EobPt256 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_16X16_CTX,
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

/// The § 8.3.2 neutral V `txb_skip` context: `0`. For a V-only-coded block the chroma block
/// equals its transform and the U plane is skipped (`EobU == 0`), so neither the
/// chroma-larger-than-tx (`+3`) nor the `EobU != 0` (`+6`) term applies.
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;

/// Returns the three AV2 § 5.20.7.27 coded chroma V DC CDF tokens for the **general** intra
/// decode path's `TX_32X32` chroma transform: a single nonzero DC of unsigned `magnitude`
/// (`1..=MAX_BASE_EOB_MAGNITUDE`, the base tier — no `coeff_br`/golomb).
///
/// Like [`general_intra_32x32_chroma_u_dc_coded_tokens`] but the `txb_skip` uses the dedicated
/// `TileVTxbSkipCdf` (`VTxbSkip`) at the neutral context `0` (the U plane is skipped, so
/// `EobU == 0`); the `eob_pt_1024` and `coeff_base_lf_eob_uv` rows are shared with U. The caller
/// appends the V DC `sign_bit` § 8.2.5 bypass literal.
pub(crate) fn general_intra_32x32_chroma_v_dc_coded_tokens(
    coeff_cdf_q_ctx: usize,
    magnitude: u32,
    v_txb_skip_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    if !(1..=MAX_BASE_EOB_MAGNITUDE).contains(&magnitude) {
        return Err(Error::CoefficientTokenizationUnsupportedChromaMagnitude {
            plane: PlaneId::V,
            magnitude,
            max_magnitude: MAX_BASE_EOB_MAGNITUDE,
        });
    }
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded chroma V DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::VTxbSkip {
            coeff_cdf_q_ctx,
            ctx: v_txb_skip_ctx,
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

/// AV2 § 8.3.2 eob=2 non-EOB `coeff_base` contexts (mirroring the minimal-tier eob=2 trace):
/// the EOB-position (AC) `coeff_base_eob` low-frequency context `1`, and the DC `coeff_base`
/// low-frequency context `1` (the AC level-1 coefficient at scan index 1 / raster row 1 col 0
/// is the DC's significant neighbour; derived via `coeff_base_lf_luma_context`). TCQ off -> tcq
/// context `0`. The 32x32 (`TX_64X64` coded) scan maps scan index 1 to raster position 32, the
/// same vertical-neighbour relationship as the 4x4 scan, so the contexts match the minimal tier.
const COEFF_BASE_LF_EOB_CTX_EOB2_AC: usize = 1;
const COEFF_BASE_LF_CTX_EOB2_DC: usize = 1;
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Luma64BasePassToken {
    CoeffBaseEob { ctx: usize, symbol: u8 },
    CoeffBase { ctx: usize, symbol: u8 },
}

fn general_intra_64x64_luma_tokens(
    coeff_cdf_q_ctx: usize,
    context: &'static str,
    eob_pt_symbol: u8,
    eob_extra_symbol: Option<u8>,
    base_pass: &[Luma64BasePassToken],
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    let token_count = 2 + usize::from(eob_extra_symbol.is_some()) + base_pass.len();
    tokens
        .try_reserve_exact(token_count)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed { context })?;
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
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt1024,
        selector: CoefficientCdfRowSelector::EobPt1024 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: eob_pt_symbol,
    });
    if let Some(symbol) = eob_extra_symbol {
        tokens.push(CoefficientEntropyToken {
            syntax: CoefficientTokenSyntax::EobExtra,
            selector: CoefficientCdfRowSelector::EobExtra { coeff_cdf_q_ctx },
            symbol,
        });
    }
    for token in base_pass {
        match *token {
            Luma64BasePassToken::CoeffBaseEob { ctx, symbol } => {
                tokens.push(CoefficientEntropyToken {
                    syntax: CoefficientTokenSyntax::CoeffBaseEob,
                    selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
                        coeff_cdf_q_ctx,
                        tx_size: TX_SIZE_64X64_CTX,
                        ctx,
                    },
                    symbol,
                });
            }
            Luma64BasePassToken::CoeffBase { ctx, symbol } => {
                tokens.push(CoefficientEntropyToken {
                    syntax: CoefficientTokenSyntax::CoeffBase,
                    selector: CoefficientCdfRowSelector::CoeffBaseLf {
                        coeff_cdf_q_ctx,
                        tx_size: TX_SIZE_64X64_CTX,
                        ctx,
                        tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                    },
                    symbol,
                });
            }
        }
    }
    Ok(tokens)
}

/// Returns the ordered general `TX_64X64` luma eob=2 multi-coefficient CDF tokens: a single
/// nonzero AC coefficient of level 1 at scan index 1 and a zero DC. The sequence is
/// `txb_skip == 0`, `eob_pt_1024 == 1` (eob 2), then the base pass over `c = eob-1..0`: the AC
/// `coeff_base_eob` (level 1 -> symbol 0) at the EOB context `1`, then the DC `coeff_base`
/// (level 0 -> symbol 0) at the low-frequency context `1`. The caller appends the AC `sign_bit`
/// § 8.2.5 bypass literal (the zero DC carries no sign). `TX_64X64` is DCT-only, so no
/// `intra_tx_type`/`sec_tx_type` symbol is read.
pub(crate) fn general_intra_64x64_luma_two_coeff_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        coeff_cdf_q_ctx,
        "general two-coefficient luma tokens",
        1,
        None,
        &[
            Luma64BasePassToken::CoeffBaseEob {
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                symbol: 0,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: COEFF_BASE_LF_CTX_EOB2_DC,
                symbol: 0,
            },
        ],
    )
}

/// The *visible* eob=2 AC level. Unlike the minimal level-1 AC (whose dequantized residual
/// rounds to ~0, reconstructing flat 128), level 4 — the largest base level with no `coeff_br`
/// tail (`coeff_base_eob` symbol `LF_NUM_BASE_LEVELS - 1` = 3; `needs_br` is `level > 4`) — dequantizes to a residual large
/// enough to reconstruct a visibly non-flat (low-frequency cosine) luma plane.
const VISIBLE_AC_LEVEL: u8 = 4;
/// The DC `coeff_base` low-frequency context for the level-4 AC: `coeff_base_lf_luma_context`
/// maps the larger significant-neighbour magnitude to context `2` (vs `1` for level 1, and `3`
/// for level 5). Pinned here and asserted against the derivation in this module's tests.
const VISIBLE_AC_DC_CTX: usize = 2;

/// Returns the ordered general `TX_64X64` luma eob=2 tokens for a **visibly non-flat** block:
/// a single nonzero AC coefficient of level 4 at scan index 1 and a zero DC. The sequence is
/// `txb_skip == 0`, `eob_pt_1024 == 1` (eob 2), then the base pass over `c = eob-1..0`: the AC
/// `coeff_base_eob` (level 4 -> symbol 3, the largest no-`coeff_br` base level) at the EOB
/// context `1`, then the DC `coeff_base` (level 0 -> symbol 0) at its `Level[]`-derived
/// low-frequency context `2`. The caller appends the AC `sign_bit` § 8.2.5
/// bypass literal. Like the minimal tier, `TX_64X64` is DCT-only (no transform-type symbol),
/// and the AC sits at scan index 1; only the level (and hence the DC context) differs.
pub(crate) fn general_intra_64x64_luma_visible_ac_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        coeff_cdf_q_ctx,
        "general visible-AC luma tokens",
        1,
        None,
        &[
            Luma64BasePassToken::CoeffBaseEob {
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: VISIBLE_AC_DC_CTX,
                symbol: 0,
            },
        ],
    )
}

/// The DC coefficient level for the two-nonzero-coefficient block: `1` (the smallest nonzero
/// level, a single non-EOB `coeff_base` symbol with no `coeff_br` tail). Its sign is CDF-coded
/// (`dc_sign`).
const TWO_NONZERO_DC_LEVEL: u8 = 1;

/// Returns the ordered general `TX_64X64` luma eob=2 **base-pass** tokens for a block with two
/// nonzero coefficients — a level-4 AC at scan index 1 and a level-1 DC at scan index 0 — up to
/// but excluding the signs: `txb_skip == 0`, `eob_pt_1024 == 1`, the AC `coeff_base_eob` (symbol
/// 3 at ctx 1), and the DC `coeff_base` (symbol 1 at the AC-level-derived ctx 2). The DC context
/// is the same ctx 2 as the visible-AC frame (it depends on the AC neighbour level, not the DC's
/// own value). `TX_64X64` is DCT-only, so no transform-type symbol is read.
///
/// The caller emits the two signs **after** these tokens, in the AV2 § 5.20.7.27 sign-pass order
/// `c = eob-1 .. 0` (reverse scan): the AC `sign_bit` § 8.2.5 bypass (c=1) FIRST, then the DC
/// `dc_sign` CDF symbol (c=0) — see [`luma_dc_sign_token`].
pub(crate) fn general_intra_64x64_luma_two_nonzero_base_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        coeff_cdf_q_ctx,
        "general two-nonzero-coefficient luma base tokens",
        1,
        None,
        &[
            Luma64BasePassToken::CoeffBaseEob {
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: VISIBLE_AC_DC_CTX,
                symbol: TWO_NONZERO_DC_LEVEL,
            },
        ],
    )
}

/// AV2 §5.20.7.27 eob=3 EOB signaling + the scan-index-1 `coeff_base` low-frequency context.
/// `eob_pt_1024` symbol 2 -> eobPt 3 -> eob = ((1<<(eobPt-2))+1) + eob_extra = 3 (with the
/// `eob_extra` CDF bit 0; the `eob_extra_bit` § 8.2.5 bypass width is eobPt-3 = 0, so no bypass
/// literal). Scan index 1 (raster 32) is off the EOB coefficient's significant-neighbour set, so
/// its non-EOB `coeff_base` low-frequency context is `9` (`coeff_base_lf_luma_context`, verified).
const EOB3_EOB_PT_SYMBOL: u8 = 2;
const EOB3_EOB_EXTRA_SYMBOL: u8 = 0;
const EOB3_SCAN1_COEFF_BASE_CTX: usize = 9;

/// Returns the ordered general `TX_64X64` luma **eob=3 base-pass** tokens for a block whose only
/// nonzero coefficient is a level-4 AC at scan index 2 (raster 1, the horizontal frequency-1
/// position), with scan indices 1 and 0 zero. This is the first eob>2 block — it exercises the
/// `eob_extra` CDF symbol. The sequence is `txb_skip == 0`, `eob_pt_1024 == 2`, `eob_extra == 0`,
/// then the base pass over the reverse scan `c = 2,1,0`: the AC `coeff_base_eob` (level 4 ->
/// symbol 3) at ctx 1, the scan-index-1 `coeff_base` (level 0) at ctx 9, and the DC `coeff_base`
/// (level 0) at ctx 2 (the level-4 AC raises the DC's significant-neighbour sum to the same band
/// as the visible-AC frame). The caller appends the single AC `sign_bit` § 8.2.5 bypass (scan
/// index 2, the only nonzero coefficient). `TX_64X64` is DCT-only, so no transform-type symbol.
pub(crate) fn general_intra_64x64_luma_eob3_base_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        coeff_cdf_q_ctx,
        "general eob=3 luma base tokens",
        EOB3_EOB_PT_SYMBOL,
        Some(EOB3_EOB_EXTRA_SYMBOL),
        &[
            Luma64BasePassToken::CoeffBaseEob {
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: EOB3_SCAN1_COEFF_BASE_CTX,
                symbol: 0,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: VISIBLE_AC_DC_CTX,
                symbol: 0,
            },
        ],
    )
}

/// The DC `coeff_base` low-frequency context for the eob=3 2-D block: two level-4 AC neighbours
/// (scan 1 + scan 2) sum to § 8.3.2 magnitude 8, mapping the DC to context `(8+1)>>1 = 4`.
const COEFF_2D_DC_CTX: usize = 4;

/// Returns the ordered general `TX_64X64` luma **eob=3 2-D base-pass** tokens for a block with
/// two nonzero level-4 ACs — scan index 1 (raster 32, vertical frequency 1) and scan index 2
/// (raster 1, horizontal frequency 1, the EOB) — with a zero DC. This is the first block whose
/// reconstruction varies in both dimensions (the vertical + horizontal cosines superimposed). The
/// sequence is `txb_skip == 0`, `eob_pt_1024 == 2`, `eob_extra == 0`, then the base pass over the
/// reverse scan `c = 2,1,0`: the EOB AC `coeff_base_eob` (level 4 -> symbol 3) at ctx 1, the
/// scan-1 AC `coeff_base` (level 4, no `coeff_br` since 4 == LF_NUM_BASE_LEVELS) at ctx 9, and the
/// DC `coeff_base` (level 0) at ctx 4. The caller appends the two AC `sign_bit` § 8.2.5 bypasses
/// in reverse-scan order (scan 2 then scan 1). `TX_64X64` is DCT-only — no transform-type symbol.
pub(crate) fn general_intra_64x64_luma_2d_base_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        coeff_cdf_q_ctx,
        "general 2-D luma base tokens",
        EOB3_EOB_PT_SYMBOL,
        Some(EOB3_EOB_EXTRA_SYMBOL),
        &[
            Luma64BasePassToken::CoeffBaseEob {
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: EOB3_SCAN1_COEFF_BASE_CTX,
                symbol: VISIBLE_AC_LEVEL,
            },
            Luma64BasePassToken::CoeffBase {
                ctx: COEFF_2D_DC_CTX,
                symbol: 0,
            },
        ],
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{VISIBLE_AC_DC_CTX, VISIBLE_AC_LEVEL};
    use crate::coefficient_tokenization::coeff_base_lf_luma_context;
    use splot_recon::{TransformClass, coefficient_scan_order};

    fn dc_ctx_for_ac_level(level: u32) -> usize {
        const W: usize = 32;
        const H: usize = 32;
        const BWL: u32 = 5; // log2(32)
        let mut scan = vec![0u16; W * H];
        coefficient_scan_order(W, H, TransformClass::TwoD, &mut scan).unwrap();
        let ac_pos = scan[1] as usize;
        let mut levels = vec![0u32; W * H];
        levels[ac_pos] = level;
        coeff_base_lf_luma_context(0, BWL, W, H, 0, 0, &levels)
    }

    #[test]
    fn visible_ac_dc_context_matches_the_level_4_derivation() {
        assert_eq!(VISIBLE_AC_LEVEL, 4);
        assert_eq!(
            dc_ctx_for_ac_level(VISIBLE_AC_LEVEL as u32),
            VISIBLE_AC_DC_CTX
        );
        assert_eq!(VISIBLE_AC_DC_CTX, 2);
    }

    #[test]
    fn dc_context_is_ac_level_dependent() {
        assert_eq!(dc_ctx_for_ac_level(1), 1);
        assert_eq!(dc_ctx_for_ac_level(2), 1);
        assert_eq!(dc_ctx_for_ac_level(3), 2);
        assert_eq!(dc_ctx_for_ac_level(4), 2);
        assert_eq!(dc_ctx_for_ac_level(5), 3);
    }
}
