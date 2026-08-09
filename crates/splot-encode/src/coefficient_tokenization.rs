// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient tokens used by the supported general-intra packet encoder.

mod general_coded;

pub(crate) use general_coded::{
    general_intra_32x32_chroma_u_dc_coded_tokens,
    general_intra_32x32_chroma_v_after_coded_u_dc_coded_tokens,
    general_intra_32x32_chroma_v_dc_coded_tokens, general_intra_64x64_luma_2d_base_tokens,
    general_intra_64x64_luma_dc_coded_tokens, general_intra_64x64_luma_eob3_base_tokens,
    general_intra_64x64_luma_two_coeff_tokens, general_intra_64x64_luma_two_nonzero_base_tokens,
    general_intra_64x64_luma_visible_ac_tokens,
};

/// Exact default-CDF row used by one supported coefficient token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoefficientCdfRowSelector {
    LumaTxbSkip64x64,
    ChromaUTxbSkip32x32,
    ChromaVTxbSkipNeutral,
    ChromaVTxbSkipAfterCodedU,
    LumaEobPt1024,
    ChromaEobPt1024,
    EobExtra,
    LumaCoeffBaseLfEobDc,
    LumaCoeffBaseLfEobAc,
    ChromaCoeffBaseLfEob,
    LumaCoeffBaseLfCtx1,
    LumaCoeffBaseLfCtx2,
    LumaCoeffBaseLfCtx9,
    LumaCoeffBaseLfCtx4,
    LumaCoeffBrLfDc,
    LumaDcSign,
}

/// One CDF-coded coefficient token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoefficientEntropyToken {
    selector: CoefficientCdfRowSelector,
    symbol: u8,
}

impl CoefficientEntropyToken {
    /// Returns the scoped CDF row selector.
    pub(crate) const fn selector(self) -> CoefficientCdfRowSelector {
        self.selector
    }

    /// Returns the raw AV2 § 8.2 symbol value.
    pub(crate) const fn symbol(self) -> u8 {
        self.symbol
    }
}

/// Returns the neutral V-plane `txb_skip` token.
pub(crate) const fn chroma_v_all_zero_token() -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaVTxbSkipNeutral,
        symbol: 1,
    }
}

/// Returns the V-plane `txb_skip` token after a coded U plane.
pub(crate) const fn chroma_v_all_zero_after_coded_u_token() -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaVTxbSkipAfterCodedU,
        symbol: 1,
    }
}

/// Returns the `TX_64X64` luma `txb_skip` token used by the general intra path.
pub(crate) const fn general_intra_64x64_luma_all_zero_token() -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaTxbSkip64x64,
        symbol: 1,
    }
}

/// Returns the `TX_32X32` chroma-U `txb_skip` token used by the general intra path.
pub(crate) const fn general_intra_32x32_chroma_u_all_zero_token() -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::ChromaUTxbSkip32x32,
        symbol: 1,
    }
}

/// Returns the negative neutral luma DC-sign token.
pub(crate) const fn luma_negative_dc_sign_token() -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        selector: CoefficientCdfRowSelector::LumaDcSign,
        symbol: 1,
    }
}
