// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient tokens for the supported general-intra packet paths.

use super::{CoefficientCdfRowSelector, CoefficientEntropyToken, luma_negative_dc_sign_token};
use crate::error::{Error, Result};

/// Returns the luma tokens for the supported negative level-6 DC block.
pub(crate) fn general_intra_64x64_luma_dc_coded_tokens() -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(5)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaTxbSkip64x64,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaEobPt1024,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobDc,
        symbol: 4,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaCoeffBrLfDc,
        symbol: 1,
    });
    tokens.push(luma_negative_dc_sign_token());
    Ok(tokens)
}

/// Returns the chroma-U tokens for the supported negative level-4 DC block.
pub(crate) fn general_intra_32x32_chroma_u_dc_coded_tokens() -> Result<Vec<CoefficientEntropyToken>>
{
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded chroma U DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaUTxbSkip32x32,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaEobPt1024,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaCoeffBaseLfEob,
        symbol: 3,
    });
    Ok(tokens)
}

/// Returns the neutral-row chroma-V tokens for the supported negative level-4 DC block.
pub(crate) fn general_intra_32x32_chroma_v_dc_coded_tokens() -> Result<Vec<CoefficientEntropyToken>>
{
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded chroma V DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaVTxbSkipNeutral,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaEobPt1024,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaCoeffBaseLfEob,
        symbol: 3,
    });
    Ok(tokens)
}

/// Returns the after-coded-U chroma-V tokens for the supported negative level-4 DC block.
pub(crate) fn general_intra_32x32_chroma_v_after_coded_u_dc_coded_tokens()
-> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general coded chroma V DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaVTxbSkipAfterCodedU,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaEobPt1024,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaCoeffBaseLfEob,
        symbol: 3,
    });
    Ok(tokens)
}

fn general_intra_64x64_luma_tokens(
    context: &'static str,
    eob_pt_symbol: u8,
    eob_extra_symbol: Option<u8>,
    base_pass: &[CoefficientEntropyToken],
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    let token_count = 2 + usize::from(eob_extra_symbol.is_some()) + base_pass.len();
    tokens
        .try_reserve_exact(token_count)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed { context })?;
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaTxbSkip64x64,
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaEobPt1024,
        symbol: eob_pt_symbol,
    });
    if let Some(symbol) = eob_extra_symbol {
        tokens.push(CoefficientEntropyToken {
            selector: CoefficientCdfRowSelector::EobExtra,
            symbol,
        });
    }
    tokens.extend_from_slice(base_pass);
    Ok(tokens)
}

/// Returns the ordered general `TX_64X64` luma eob=2 multi-coefficient CDF tokens: a single
/// nonzero AC coefficient of level 1 at scan index 1 and a zero DC. The sequence is
/// `txb_skip == 0`, `eob_pt_1024 == 1` (eob 2), then the base pass over `c = eob-1..0`: the AC
/// `coeff_base_eob` (level 1 -> symbol 0) at the EOB context `1`, then the DC `coeff_base`
/// (level 0 -> symbol 0) at the low-frequency context `1`. The caller appends the AC `sign_bit`
/// § 8.2.5 bypass literal (the zero DC carries no sign). `TX_64X64` is DCT-only, so no
/// `intra_tx_type`/`sec_tx_type` symbol is read.
pub(crate) fn general_intra_64x64_luma_two_coeff_tokens() -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        "general two-coefficient luma tokens",
        1,
        None,
        &[
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc,
                symbol: 0,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx1,
                symbol: 0,
            },
        ],
    )
}

/// The visible eob=2 AC level. Level 4 is the largest base level with no `coeff_br` tail and dequantizes to a residual large
/// enough to reconstruct a visibly non-flat (low-frequency cosine) luma plane.
const VISIBLE_AC_LEVEL: u8 = 4;

/// Returns the ordered general `TX_64X64` luma eob=2 tokens for a **visibly non-flat** block:
/// a single nonzero AC coefficient of level 4 at scan index 1 and a zero DC. The sequence is
/// `txb_skip == 0`, `eob_pt_1024 == 1` (eob 2), then the base pass over `c = eob-1..0`: the AC
/// `coeff_base_eob` (level 4 -> symbol 3, the largest no-`coeff_br` base level) at the EOB
/// context `1`, then the DC `coeff_base` (level 0 -> symbol 0) at its `Level[]`-derived
/// low-frequency context `2`. The caller appends the AC `sign_bit` § 8.2.5
/// bypass literal. `TX_64X64` is DCT-only (no transform-type symbol).
pub(crate) fn general_intra_64x64_luma_visible_ac_tokens() -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        "general visible-AC luma tokens",
        1,
        None,
        &[
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx2,
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
/// `dc_sign` CDF symbol (c=0).
pub(crate) fn general_intra_64x64_luma_two_nonzero_base_tokens()
-> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        "general two-nonzero-coefficient luma base tokens",
        1,
        None,
        &[
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx2,
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

/// Returns the ordered general `TX_64X64` luma **eob=3 base-pass** tokens for a block whose only
/// nonzero coefficient is a level-4 AC at scan index 2 (raster 1, the horizontal frequency-1
/// position), with scan indices 1 and 0 zero. This is the first eob>2 block — it exercises the
/// `eob_extra` CDF symbol. The sequence is `txb_skip == 0`, `eob_pt_1024 == 2`, `eob_extra == 0`,
/// then the base pass over the reverse scan `c = 2,1,0`: the AC `coeff_base_eob` (level 4 ->
/// symbol 3) at ctx 1, the scan-index-1 `coeff_base` (level 0) at ctx 9, and the DC `coeff_base`
/// (level 0) at ctx 2 (the level-4 AC raises the DC's significant-neighbour sum to the same band
/// as the visible-AC frame). The caller appends the single AC `sign_bit` § 8.2.5 bypass (scan
/// index 2, the only nonzero coefficient). `TX_64X64` is DCT-only, so no transform-type symbol.
pub(crate) fn general_intra_64x64_luma_eob3_base_tokens() -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        "general eob=3 luma base tokens",
        EOB3_EOB_PT_SYMBOL,
        Some(EOB3_EOB_EXTRA_SYMBOL),
        &[
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx9,
                symbol: 0,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx2,
                symbol: 0,
            },
        ],
    )
}

/// The DC `coeff_base` low-frequency context for the eob=3 2-D block: two level-4 AC neighbours
/// (scan 1 + scan 2) sum to § 8.3.2 magnitude 8, mapping the DC to context `(8+1)>>1 = 4`.
/// Returns the ordered general `TX_64X64` luma **eob=3 2-D base-pass** tokens for a block with
/// two nonzero level-4 ACs — scan index 1 (raster 32, vertical frequency 1) and scan index 2
/// (raster 1, horizontal frequency 1, the EOB) — with a zero DC. This is the first block whose
/// reconstruction varies in both dimensions (the vertical + horizontal cosines superimposed). The
/// sequence is `txb_skip == 0`, `eob_pt_1024 == 2`, `eob_extra == 0`, then the base pass over the
/// reverse scan `c = 2,1,0`: the EOB AC `coeff_base_eob` (level 4 -> symbol 3) at ctx 1, the
/// scan-1 AC `coeff_base` (level 4, no `coeff_br` since 4 == LF_NUM_BASE_LEVELS) at ctx 9, and the
/// DC `coeff_base` (level 0) at ctx 4. The caller appends the two AC `sign_bit` § 8.2.5 bypasses
/// in reverse-scan order (scan 2 then scan 1). `TX_64X64` is DCT-only — no transform-type symbol.
pub(crate) fn general_intra_64x64_luma_2d_base_tokens() -> Result<Vec<CoefficientEntropyToken>> {
    general_intra_64x64_luma_tokens(
        "general 2-D luma base tokens",
        EOB3_EOB_PT_SYMBOL,
        Some(EOB3_EOB_EXTRA_SYMBOL),
        &[
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc,
                symbol: VISIBLE_AC_LEVEL - 1,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx9,
                symbol: VISIBLE_AC_LEVEL,
            },
            CoefficientEntropyToken {
                selector: CoefficientCdfRowSelector::LumaCoeffBaseLfCtx4,
                symbol: 0,
            },
        ],
    )
}
