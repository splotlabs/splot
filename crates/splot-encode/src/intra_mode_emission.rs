// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder intra-mode symbol-emission foundation.
//!
//! This module advances `ENC-INTRA-MODE-SYMBOL-EMISSION`. It produces the ordered
//! AV2 § 5.20.5.5 `y_mode_set` / `y_mode_index` entropy-token records for the
//! current minimal DC_PRED luma block at the tile-origin neutral context, and
//! proves those records can roundtrip through `splot-core`'s AV2 § 8.2 symbol
//! encoder/decoder
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-5`).
//!
//! `y_mode_set` and `y_mode_index` both equal to 0 select the DC_PRED luma mode
//! (the mode the closed-loop reconstruction uses). This module emits only the
//! mode *selector* syntax; it does not perform or verify the AV2 § 7.13.2.10 DC
//! prediction process (that proof lives in the reconstruction features). The
//! `y_mode_index` § 8.3.2 context is the joint-neighbour context, which is 0 at
//! the tile origin where both neighbours are out of frame.
//!
//! This emits the AV2 § 5.20.5.5 ordered sequence for a **non-lossless** block.
//! Per `read_intra_y_mode()` (`if ( Lossless ) { use_dpcm_y } else
//! { use_dpcm_y = 0 }`), a non-lossless block infers `use_dpcm_y = 0` and emits no
//! `use_dpcm_y` symbol, so the ordered syntax reduces to `y_mode_set` then
//! `y_mode_index`. Lossless blocks — which read `use_dpcm_y` (and possibly
//! `dpcm_mode_y`) before `y_mode_set` — are out of scope and MUST NOT use this
//! helper until lossless DPCM-mode emission is implemented.
//!
//! It also emits the chroma `uv_mode` selector (AV2 § 5.20.5.6) for the DC chroma
//! mode (`Default_Mode_List_Uv` index 0 = DC_PRED) at the non-directional context
//! (`is_directional(YMode)` is 0 for DC_PRED), tracked by
//! `ENC-UV-MODE-SYMBOL-EMISSION`. In the AV2 § 5.20.5.3 mode-info order
//! `read_intra_uv_mode()` is called right after `read_intra_y_mode()` and before
//! `residual()`, so `uv_mode` precedes all coefficient symbols; it is emitted as a
//! standalone token for ordered composition (`y_mode_set`, `y_mode_index`,
//! `uv_mode`, then residual/coefficient syntax).
//!
//! `uv_mode` emission here is valid only for the minimal tier where the § 5.20.5.6
//! predecessors are not read: a non-lossless block (`use_dpcm_uv` is inferred 0
//! per `if ( Lossless ) { use_dpcm_uv } else { use_dpcm_uv = 0 }`) with CfL
//! disabled (`enable_cfl_intra == 0` makes `cflAllowed == 0`) and MHCCP
//! unavailable, so `is_cfl` is not read either. Lossless blocks (which read
//! `use_dpcm_uv` / `dpcm_mode_uv`) and CfL/MHCCP-enabled blocks (which read
//! `is_cfl`) read those symbols before `uv_mode` and are out of scope.
//!
//! It does not emit coefficient or all-zero symbols, lossless
//! `use_dpcm_y`/`dpcm_mode_y` or `use_dpcm_uv`/`dpcm_mode_uv` symbols, `is_cfl` /
//! CfL / CCTX / MHCCP syntax, tile payloads, tile CDF lifecycle, packets, a public
//! encoder API, or chroma/intra modes beyond DC.

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use crate::error::{Error, Result};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CDF_ROW_LEN: usize = 9;
const UV_MODE_CDF_ROW_LEN: usize = 9;
const TILE_ORIGIN_Y_MODE_INDEX_CTX: usize = 0;
const NON_DIRECTIONAL_UV_MODE_CTX: usize = 0;
const DC_PRED_Y_MODE_SET_SYMBOL: u8 = 0;
const DC_PRED_Y_MODE_INDEX_SYMBOL: u8 = 0;
const DC_CHROMA_UV_MODE_SYMBOL: u8 = 0;
const INTRA_MODE_SYMBOL_BUDGET: usize = 16;

/// Emits the current minimal DC_PRED luma intra-mode block symbols.
pub(crate) fn emit_minimal_dc_luma_intra_mode() -> Result<IntraModeEmissionPlan> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(2)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "dc intra-mode tokens",
        })?;
    tokens.push(IntraModeToken {
        syntax: IntraModeSyntax::YModeSet,
        selector: IntraModeCdfRowSelector::YModeSet,
        symbol: DC_PRED_Y_MODE_SET_SYMBOL,
    });
    tokens.push(IntraModeToken {
        syntax: IntraModeSyntax::YModeIndex,
        selector: IntraModeCdfRowSelector::YModeIndex {
            ctx: TILE_ORIGIN_Y_MODE_INDEX_CTX,
        },
        symbol: DC_PRED_Y_MODE_INDEX_SYMBOL,
    });
    Ok(IntraModeEmissionPlan { tokens })
}

/// Emits the current minimal DC chroma `uv_mode` block symbol.
///
/// `uv_mode == 0` selects `Default_Mode_List_Uv[0] == DC_PRED` chroma (AV2
/// § 5.20.5.6) for a non-directional DC_PRED luma block, whose § 8.3.2 context
/// `is_directional(YMode)` is 0.
pub(crate) fn emit_minimal_dc_chroma_uv_mode() -> Result<IntraModeEmissionPlan> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(1)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "dc chroma uv-mode token",
        })?;
    tokens.push(IntraModeToken {
        syntax: IntraModeSyntax::UvMode,
        selector: IntraModeCdfRowSelector::UvModeCflNotAllowed {
            ctx: NON_DIRECTIONAL_UV_MODE_CTX,
        },
        symbol: DC_CHROMA_UV_MODE_SYMBOL,
    });
    Ok(IntraModeEmissionPlan { tokens })
}

/// AV2 intra-mode syntax covered by the current private subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraModeSyntax {
    /// `y_mode_set` in AV2 § 5.20.5.5.
    YModeSet,
    /// `y_mode_index` in AV2 § 5.20.5.5.
    YModeIndex,
    /// `uv_mode` in AV2 § 5.20.5.6.
    UvMode,
}

impl IntraModeSyntax {
    const fn as_str(self) -> &'static str {
        match self {
            Self::YModeSet => "y_mode_set",
            Self::YModeIndex => "y_mode_index",
            Self::UvMode => "uv_mode",
        }
    }
}

/// Scoped default-CDF selector for one intra-mode token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraModeCdfRowSelector {
    /// `TileYModeSetCdf` (§ 8.3.2, no context).
    YModeSet,
    /// `TileYModeIndexCdf[ctx]` (§ 8.3.2).
    YModeIndex {
        /// Joint-neighbour context (0 at the tile origin).
        ctx: usize,
    },
    /// `TileUVModeCflNotAllowedCdf[ctx]` (§ 8.3.2).
    UvModeCflNotAllowed {
        /// `is_directional(YMode)` context (0 for the DC_PRED luma block).
        ctx: usize,
    },
}

impl IntraModeCdfRowSelector {
    const fn syntax_name(self) -> &'static str {
        match self {
            Self::YModeSet => IntraModeSyntax::YModeSet.as_str(),
            Self::YModeIndex { .. } => IntraModeSyntax::YModeIndex.as_str(),
            Self::UvModeCflNotAllowed { .. } => IntraModeSyntax::UvMode.as_str(),
        }
    }
}

/// Ordered intra-mode entropy-token record for the current private subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraModeToken {
    syntax: IntraModeSyntax,
    selector: IntraModeCdfRowSelector,
    symbol: u8,
}

impl IntraModeToken {
    /// Returns the token syntax.
    pub(crate) const fn syntax(self) -> IntraModeSyntax {
        self.syntax
    }

    /// Returns the scoped CDF row selector.
    pub(crate) const fn selector(self) -> IntraModeCdfRowSelector {
        self.selector
    }

    /// Returns the raw AV2 § 8.2 symbol value.
    pub(crate) const fn symbol(self) -> u8 {
        self.symbol
    }

    const fn syntax_name(self) -> &'static str {
        self.syntax.as_str()
    }
}

/// Emission result for the current private intra-mode subset.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct IntraModeEmissionPlan {
    tokens: Vec<IntraModeToken>,
}

impl IntraModeEmissionPlan {
    /// Returns ordered entropy-token records.
    pub(crate) fn tokens(&self) -> &[IntraModeToken] {
        &self.tokens
    }
}

/// Result of proving intra-mode token values through AV2 § 8.2 symbol bytes.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct IntraModeRoundtrip {
    bytes: Vec<u8>,
    decoded_symbols: Vec<u8>,
    symbol_count: u64,
}

impl IntraModeRoundtrip {
    /// Returns finalized symbol payload bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns decoded token symbols in order.
    pub(crate) fn decoded_symbols(&self) -> &[u8] {
        &self.decoded_symbols
    }

    /// Returns the decoder's final symbol count.
    pub(crate) const fn symbol_count(&self) -> u64 {
        self.symbol_count
    }
}

/// Writes intra-mode token records with the § 8.2 symbol encoder and decodes them back.
pub(crate) fn roundtrip_intra_mode_tokens(tokens: &[IntraModeToken]) -> Result<IntraModeRoundtrip> {
    let mut encode_cdfs = IntraModeCdfRows::from_defaults();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new()
            .with_max_output_bytes(INTRA_MODE_SYMBOL_BUDGET)
            .with_max_operations(INTRA_MODE_SYMBOL_BUDGET),
    );
    for token in tokens.iter().copied() {
        encoder
            .write_symbol(
                encode_cdfs.row_mut(token.selector())?,
                Symbol::new(token.symbol()),
            )
            .map_err(|source| Error::IntraModeEmissionSymbolWrite {
                syntax: token.syntax_name(),
                source,
            })?;
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::IntraModeEmissionSymbolEncodeFinish { source })?;
    let bytes = output.into_bytes();

    let mut decode_cdfs = IntraModeCdfRows::from_defaults();
    let mut decoder = SymbolDecoder::with_config(&bytes, SymbolDecoderConfig::new())
        .map_err(|source| Error::IntraModeEmissionSymbolDecodeInit { source })?;
    let mut decoded_symbols = Vec::new();
    decoded_symbols
        .try_reserve_exact(tokens.len())
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "roundtrip decoded symbols",
        })?;
    for token in tokens.iter().copied() {
        let decoded = decoder
            .read_symbol(decode_cdfs.row_mut(token.selector())?)
            .map_err(|source| Error::IntraModeEmissionSymbolRead {
                syntax: token.syntax_name(),
                source,
            })?
            .get();
        if decoded != token.symbol() {
            return Err(Error::IntraModeEmissionSymbolMismatch {
                syntax: token.syntax_name(),
                expected: token.symbol(),
                actual: decoded,
            });
        }
        decoded_symbols.push(decoded);
    }
    let summary = decoder
        .finish()
        .map_err(|source| Error::IntraModeEmissionSymbolDecodeFinish { source })?;

    Ok(IntraModeRoundtrip {
        bytes,
        decoded_symbols,
        symbol_count: summary.symbol_count,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IntraModeCdfRows {
    y_mode_set: [i32; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [i32; Y_MODE_INDEX_CDF_ROW_LEN],
    uv_mode_cfl_not_allowed_non_directional: [i32; UV_MODE_CDF_ROW_LEN],
}

impl IntraModeCdfRows {
    fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index_tile_origin: DEFAULT_Y_MODE_INDEX_CDF[TILE_ORIGIN_Y_MODE_INDEX_CTX],
            uv_mode_cfl_not_allowed_non_directional: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
                [NON_DIRECTIONAL_UV_MODE_CTX],
        }
    }

    fn row_mut(&mut self, selector: IntraModeCdfRowSelector) -> Result<&mut [i32]> {
        match selector {
            IntraModeCdfRowSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
            IntraModeCdfRowSelector::YModeIndex {
                ctx: TILE_ORIGIN_Y_MODE_INDEX_CTX,
            } => Ok(self.y_mode_index_tile_origin.as_mut_slice()),
            IntraModeCdfRowSelector::UvModeCflNotAllowed {
                ctx: NON_DIRECTIONAL_UV_MODE_CTX,
            } => Ok(self.uv_mode_cfl_not_allowed_non_directional.as_mut_slice()),
            selector @ (IntraModeCdfRowSelector::YModeIndex { .. }
            | IntraModeCdfRowSelector::UvModeCflNotAllowed { .. }) => {
                Err(Error::IntraModeEmissionUnsupportedCdfSelector {
                    syntax: selector.syntax_name(),
                })
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn emits_ordered_dc_pred_intra_mode_tokens() {
        let plan = emit_minimal_dc_luma_intra_mode().unwrap();

        assert_eq!(
            plan.tokens(),
            &[
                IntraModeToken {
                    syntax: IntraModeSyntax::YModeSet,
                    selector: IntraModeCdfRowSelector::YModeSet,
                    symbol: 0,
                },
                IntraModeToken {
                    syntax: IntraModeSyntax::YModeIndex,
                    selector: IntraModeCdfRowSelector::YModeIndex { ctx: 0 },
                    symbol: 0,
                },
            ]
        );
    }

    #[test]
    fn intra_mode_tokens_roundtrip_through_symbol_coder() {
        let plan = emit_minimal_dc_luma_intra_mode().unwrap();
        let proof = roundtrip_intra_mode_tokens(plan.tokens()).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0]);
        assert_eq!(proof.symbol_count(), 2);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn roundtrip_is_deterministic() {
        let plan = emit_minimal_dc_luma_intra_mode().unwrap();
        let first = roundtrip_intra_mode_tokens(plan.tokens()).unwrap();
        let second = roundtrip_intra_mode_tokens(plan.tokens()).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn accepts_only_the_tile_origin_y_mode_index_context() {
        let mut cdfs = IntraModeCdfRows::from_defaults();

        assert!(
            cdfs.row_mut(IntraModeCdfRowSelector::YModeIndex { ctx: 0 })
                .is_ok()
        );
        for ctx in [1usize, 2, 3] {
            let err = cdfs
                .row_mut(IntraModeCdfRowSelector::YModeIndex { ctx })
                .unwrap_err();
            assert!(matches!(
                err,
                Error::IntraModeEmissionUnsupportedCdfSelector {
                    syntax: "y_mode_index",
                }
            ));
        }
    }

    #[test]
    fn emits_dc_chroma_uv_mode_token() {
        let plan = emit_minimal_dc_chroma_uv_mode().unwrap();

        assert_eq!(
            plan.tokens(),
            &[IntraModeToken {
                syntax: IntraModeSyntax::UvMode,
                selector: IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx: 0 },
                symbol: 0,
            }]
        );
    }

    #[test]
    fn uv_mode_token_roundtrips_through_symbol_coder() {
        let plan = emit_minimal_dc_chroma_uv_mode().unwrap();
        let proof = roundtrip_intra_mode_tokens(plan.tokens()).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0]);
        assert_eq!(proof.symbol_count(), 1);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn accepts_only_the_non_directional_uv_mode_context() {
        let mut cdfs = IntraModeCdfRows::from_defaults();

        assert!(
            cdfs.row_mut(IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx: 0 })
                .is_ok()
        );
        for ctx in [1usize, 2] {
            let err = cdfs
                .row_mut(IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx })
                .unwrap_err();
            assert!(matches!(
                err,
                Error::IntraModeEmissionUnsupportedCdfSelector { syntax: "uv_mode" }
            ));
        }
    }
}
