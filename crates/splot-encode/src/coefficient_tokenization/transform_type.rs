// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.8.2 `transform_type()` token accessors (`ENC-INTRA-TX-TYPE-TOKEN`,
//! `ENC-SEC-TX-TYPE-TOKEN`): the `intra_tx_type` (primary) and `sec_tx_type` (IST
//! secondary) symbols read between `eob_pt` and the coefficient base pass for
//! `eob > 1` blocks (the `transform_type()` `eob == 1` DCT_DCT shortcut no longer
//! applies). `transform_type()` is the § 5.20.8.2 transform-type syntax, called from
//! § 5.20.7.27 `coeffs()`; `sec_tx_type` (line 16613) is read right after
//! `intra_tx_type` (line 16529) in the same function. Split out of
//! `coefficient_tokenization` to keep the parent file under the 1000-line budget.

use super::{CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax};

/// Returns the AV2 § 5.20.8.2 `intra_tx_type` token for the `TX_SET_INTRA_1`
/// transform set, coded with `TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]` (§ 8.3.2
/// Table 8.2). The `symbol` indexes the resolved transform type via the § 9
/// `Md_Idx_To_Type[Size_Class[txSz]][intraDir]` row (§ 5.20.8.2 line 16569).
///
/// For a 4x4 (`Tx_Size_Sqr = 0`) `DC_PRED` (`intraDir = 0`) block, symbol 0 selects
/// `DCT_DCT` (`Md_Idx_To_Type[0][0][0] = 0`), so `intra_tx_type_set1_token(0, 0)` is
/// the DCT_DCT transform-type symbol for the minimal eob > 1 intra block.
pub(crate) const fn intra_tx_type_set1_token(
    tx_size_sqr: usize,
    symbol: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::IntraTxType,
        selector: CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr },
        symbol,
    }
}

/// Returns the AV2 § 5.20.8.2 intra `sec_tx_type` token (the IST secondary
/// transform), coded with `TileSecTxTypeCdf[0][Tx_Size_Sqr[txSz]]` (`is_inter = 0`;
/// § 8.3.2, `08-parsing-process.md:867`). The `symbol` is one of the four
/// `sec_tx_type` values (`STX_TYPES = 4`); `sec_tx_type` is read at § 5.20.8.2 line
/// 16613, right after `intra_tx_type`, when the IST condition holds (`enable_intra_ist
/// && eob != 1 && !Lossless && (TxType == ADST_ADST || DCT_DCT) && YMode != PAETH_PRED
/// && eob <= eobLim`).
///
/// `symbol 0` is `sec_tx_type = 0` (IST off for the block), which reads no
/// `most_probable_stx_set` follow-up (that S() is read only when `sec_tx_type != 0
/// && !is_inter`), so `sec_tx_type_intra_token(0, 0)` is the minimal IST symbol for a
/// 4x4 (`Tx_Size_Sqr = 0`) intra block.
pub(crate) const fn sec_tx_type_intra_token(
    tx_size_sqr: usize,
    symbol: u8,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::SecTxType,
        selector: CoefficientCdfRowSelector::SecTxTypeIntra { tx_size_sqr },
        symbol,
    }
}
