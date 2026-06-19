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
//! `y_mode_set` and `y_mode_index` both equal to 0 select DC_PRED (AV2 § 7.13.2.10
//! DC intra prediction), the same luma mode the closed-loop reconstruction uses.
//! The `y_mode_index` § 8.3.2 context is the joint-neighbour context, which is 0
//! at the tile origin where both neighbours are out of frame.
//!
//! It does not emit chroma `uv_mode` syntax, coefficient or all-zero symbols,
//! tile payloads, tile CDF lifecycle, packets, a public encoder API, or broad
//! intra-mode coverage beyond DC_PRED.

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF};

use crate::error::{Error, Result};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CDF_ROW_LEN: usize = 9;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const TILE_ORIGIN_Y_MODE_INDEX_CTX: usize = 0;
const DC_PRED_Y_MODE_SET_SYMBOL: u8 = 0;
const DC_PRED_Y_MODE_INDEX_SYMBOL: u8 = 0;
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

/// AV2 intra-mode syntax covered by the current private subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraModeSyntax {
    /// `y_mode_set` in AV2 § 5.20.5.5.
    YModeSet,
    /// `y_mode_index` in AV2 § 5.20.5.5.
    YModeIndex,
}

impl IntraModeSyntax {
    const fn as_str(self) -> &'static str {
        match self {
            Self::YModeSet => "y_mode_set",
            Self::YModeIndex => "y_mode_index",
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
}

impl IntraModeCdfRowSelector {
    const fn syntax_name(self) -> &'static str {
        match self {
            Self::YModeSet => IntraModeSyntax::YModeSet.as_str(),
            Self::YModeIndex { .. } => IntraModeSyntax::YModeIndex.as_str(),
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
    y_mode_index: [[i32; Y_MODE_INDEX_CDF_ROW_LEN]; Y_MODE_INDEX_CONTEXTS],
}

impl IntraModeCdfRows {
    fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index: DEFAULT_Y_MODE_INDEX_CDF,
        }
    }

    fn row_mut(&mut self, selector: IntraModeCdfRowSelector) -> Result<&mut [i32]> {
        match selector {
            IntraModeCdfRowSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
            IntraModeCdfRowSelector::YModeIndex { ctx } if ctx < Y_MODE_INDEX_CONTEXTS => {
                Ok(self.y_mode_index[ctx].as_mut_slice())
            }
            selector @ IntraModeCdfRowSelector::YModeIndex { .. } => {
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
    fn rejects_out_of_range_y_mode_index_context() {
        let mut cdfs = IntraModeCdfRows::from_defaults();
        let err = cdfs
            .row_mut(IntraModeCdfRowSelector::YModeIndex { ctx: 3 })
            .unwrap_err();

        assert!(matches!(
            err,
            Error::IntraModeEmissionUnsupportedCdfSelector {
                syntax: "y_mode_index",
            }
        ));
    }

    #[test]
    fn supported_y_mode_index_contexts_resolve_to_distinct_rows() {
        let mut cdfs = IntraModeCdfRows::from_defaults();
        for ctx in 0..Y_MODE_INDEX_CONTEXTS {
            assert!(
                cdfs.row_mut(IntraModeCdfRowSelector::YModeIndex { ctx })
                    .is_ok()
            );
        }
    }
}
