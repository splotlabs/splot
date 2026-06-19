// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 `transform_type()` token accessors (`ENC-INTRA-TX-TYPE-TOKEN`):
//! the `intra_tx_type` symbol read between `eob_pt` and the coefficient base pass
//! for `eob > 1` blocks (the `transform_type()` `eob == 1` DCT_DCT shortcut no
//! longer applies). Split out of `coefficient_tokenization` to keep the parent file
//! under the 1000-line source budget.

use super::{CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax};

/// Returns the AV2 § 5.20.7.27 `intra_tx_type` token for the `TX_SET_INTRA_1`
/// transform set, coded with `TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]` (§ 8.3.2
/// Table 8.2). The `symbol` indexes the resolved transform type via the § 9
/// `Md_Idx_To_Type[Size_Class[txSz]][intraDir]` row (§ 5.20.7.27 line 16569).
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
