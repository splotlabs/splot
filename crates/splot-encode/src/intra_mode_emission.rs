// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra-mode tokens used by the supported DC general-intra encoder path.

/// Exact default-CDF row used by one supported intra-mode token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraModeCdfRowSelector {
    /// `TileYModeSetCdf`.
    YModeSet,
    /// `TileYModeIndexCdf[0]`.
    YModeIndexTileOrigin,
    /// `TileUVModeCflNotAllowedCdf[0]`.
    UvModeNonDirectional,
}

/// One CDF-coded intra-mode token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraModeToken {
    selector: IntraModeCdfRowSelector,
    symbol: u8,
}

impl IntraModeToken {
    /// Returns the scoped CDF row selector.
    pub(crate) const fn selector(self) -> IntraModeCdfRowSelector {
        self.selector
    }

    /// Returns the raw AV2 § 8.2 symbol value.
    pub(crate) const fn symbol(self) -> u8 {
        self.symbol
    }
}

/// Emits `y_mode_set == 0` and `y_mode_index == 0` for DC_PRED luma.
pub(crate) fn emit_minimal_dc_luma_intra_mode() -> [IntraModeToken; 2] {
    [
        IntraModeToken {
            selector: IntraModeCdfRowSelector::YModeSet,
            symbol: 0,
        },
        IntraModeToken {
            selector: IntraModeCdfRowSelector::YModeIndexTileOrigin,
            symbol: 0,
        },
    ]
}

/// Emits `uv_mode == 0` for DC_PRED chroma.
pub(crate) fn emit_minimal_dc_chroma_uv_mode() -> [IntraModeToken; 1] {
    [IntraModeToken {
        selector: IntraModeCdfRowSelector::UvModeNonDirectional,
        symbol: 0,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_dc_pred_luma_tokens() {
        assert_eq!(
            emit_minimal_dc_luma_intra_mode(),
            [
                IntraModeToken {
                    selector: IntraModeCdfRowSelector::YModeSet,
                    symbol: 0,
                },
                IntraModeToken {
                    selector: IntraModeCdfRowSelector::YModeIndexTileOrigin,
                    symbol: 0,
                },
            ]
        );
    }

    #[test]
    fn emits_dc_pred_chroma_token() {
        assert_eq!(
            emit_minimal_dc_chroma_uv_mode(),
            [IntraModeToken {
                selector: IntraModeCdfRowSelector::UvModeNonDirectional,
                symbol: 0,
            }]
        );
    }
}
