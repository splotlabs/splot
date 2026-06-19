// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder block-symbol trace composition.
//!
//! This module is the home for the growing ordered block-symbol trace. It
//! advances `ENC-INTRA-BLOCK-MODE-TRACE` (the AV2 § 5.20.5.3 mode-info prefix
//! `y_mode_set`, `y_mode_index`, `uv_mode`), `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`
//! (the unified trace extended with the first `residual()` symbol, the luma
//! `txb_skip` / § 5.20.7.27 `all_zero`), `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`
//! (the complete all-zero block: the per-plane luma/U/V `txb_skip` symbols),
//! `ENC-INTRA-BLOCK-TRACE-CODED-DC` (the minimal *coded* block: a single luma DC
//! coefficient's `txb_skip=0` + `eob_pt_16` + `coeff_base_eob` + `dc_sign`), and
//! `ENC-INTRA-BLOCK-TRACE-CODED-BR` (the base-range tier: a larger luma DC
//! coefficient adding `coeff_br` after `coeff_base_eob`), and
//! `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` (the §8.2.5 bypass-literal token kind,
//! the foundation for non-luma-DC `sign_bit` and the golomb tail),
//! reusing the merged mode emitters and the coefficient tokenization's per-plane
//! all-zero and coded-DC tokens
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
//!
//! AV2 § 5.20.5.3 `intra_frame_mode_info()` calls `read_intra_y_mode()` then
//! `read_intra_uv_mode()` before `residual()`, so the ordered trace is the mode
//! prefix followed by the residual symbols; the unified `BlockSymbolToken` spans
//! both kinds, and `roundtrip_block_symbol_trace` proves the combined sequence
//! through one § 8.2 coder with shared CDF state, routing each token to its scoped
//! CDF row from `splot-core` defaults.
//!
//! It does not emit multi-coefficient blocks, the coefficient golomb
//! (exp-golomb) tail beyond the base-range tier, higher-frequency coefficients,
//! chroma coefficients, partition syntax, tile CDF lifecycle, packets, a public
//! encoder API, or modes beyond the DC minimal tier.

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF,
    DEFAULT_EOB_PT_16_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientEntropyToken, chroma_u_all_zero_token,
    chroma_v_all_zero_token, luma_all_zero_token, luma_dc_coded_tokens,
};
use crate::error::{Error, Result};
use crate::intra_mode_emission::{
    IntraModeCdfRowSelector, IntraModeToken, emit_minimal_dc_chroma_uv_mode,
    emit_minimal_dc_luma_intra_mode,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const TXB_SKIP_CDF_ROW_LEN: usize = 3;
const V_TXB_SKIP_CDF_ROW_LEN: usize = 3;
const EOB_PT_16_CDF_ROW_LEN: usize = 6;
const COEFF_BASE_LF_EOB_CDF_ROW_LEN: usize = 6;
const COEFF_BR_LF_CDF_ROW_LEN: usize = 5;
const DC_SIGN_CDF_ROW_LEN: usize = 3;
const TILE_ORIGIN_Y_MODE_INDEX_CTX: usize = 0;
const NON_DIRECTIONAL_UV_MODE_CTX: usize = 0;
const MINIMAL_COEFF_CDF_Q_CTX: usize = 0;
// AV2 § 8.3.2 `TileTxbSkipCdf`'s first index is `is_inter || fsc_mode` (the
// `plane_type` field name is a pre-existing misnomer), 0 for an intra non-FSC
// block — the bank luma and U share; the plane is distinguished only by `ctx`.
const LUMA_PLANE_TYPE: usize = 0;
const TX_SIZE_4X4_CTX: usize = 0;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_U_TXB_SKIP_CTX_NEUTRAL: usize = 6;
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
// AV2 § 5.20.7.27 / § 8.3.2 neutral coded-DC luma coefficient contexts.
const EOB_CTX_LUMA_INTRA: usize = 0;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const COEFF_BR_LF_CTX_DC: usize = 0;
const DC_SIGN_PLANE_TYPE_LUMA: usize = 0;
const DC_SIGN_GROUP_VISIBLE: usize = 0;
const DC_SIGN_CTX_NEUTRAL: usize = 0;
// Minimal coded luma block: a single DC coefficient of value +1.
const MINIMAL_CODED_DC_MAGNITUDE: u32 = 1;
const MINIMAL_CODED_DC_NEGATIVE: bool = false;
// Minimal base-range coded luma block: a single DC coefficient of value +6
// (level 5 base + `coeff_br = 1`).
const MINIMAL_BR_DC_MAGNITUDE: u32 = 6;
const MINIMAL_BR_DC_NEGATIVE: bool = false;
// Headroom (operations + output bytes) added on top of the per-trace cost. The
// roundtrip's encoder budget scales with the trace: one operation per CDF symbol
// and one per bypass-literal bit (`write_literal` charges per bit), so a wide
// `L(n)` literal — e.g. the golomb tail — is not rejected by a fixed cap.
const BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM: usize = 32;

/// Composes the ordered AV2 § 5.20.5.3 intra-block mode-info prefix
/// (`y_mode_set`, `y_mode_index`, `uv_mode`) for the current minimal DC subset.
pub(crate) fn compose_minimal_intra_dc_block_mode_trace() -> Result<Vec<IntraModeToken>> {
    let luma = emit_minimal_dc_luma_intra_mode()?;
    let uv = emit_minimal_dc_chroma_uv_mode()?;

    let total = luma.tokens().len().checked_add(uv.tokens().len()).ok_or(
        Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace length",
        },
    )?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace",
        })?;
    trace.extend_from_slice(luma.tokens());
    trace.extend_from_slice(uv.tokens());
    Ok(trace)
}

/// One symbol of the ordered block-symbol trace, spanning the intra-mode and
/// coefficient token kinds that a coded tile body interleaves through one coder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockSymbolToken {
    /// An AV2 § 5.20.5 mode-info token (`y_mode_set` / `y_mode_index` / `uv_mode`).
    Mode(IntraModeToken),
    /// An AV2 § 5.20.7 coefficient token (here, the luma `txb_skip` all-zero).
    Coeff(CoefficientEntropyToken),
    /// An AV2 § 8.2.5 bypass literal of `width` bits carrying `value` (MSB-first).
    ///
    /// Unlike `Mode`/`Coeff` (CDF-coded `S()` symbols) a bypass literal is an
    /// `L(n)` read with no CDF — e.g. the `sign_bit` of a chroma or ordinary
    /// non-axis luma coefficient (§ 5.20.7.27 codes the luma DC sign as `dc_sign`
    /// and the directional luma axis signs as `dc_sign_horz_vert`, both CDF; every
    /// other sign is `sign_bit L(1)`) or the `read_quant` golomb tail (§ 5.20.7.28).
    Bypass {
        /// Number of literal bits (`n` in `L(n)`).
        width: u32,
        /// The literal value, written/read most-significant-bit first.
        value: u32,
    },
}

impl BlockSymbolToken {
    /// Constructs a bypass literal of `width` bits carrying `value`.
    pub(crate) const fn bypass(width: u32, value: u32) -> Self {
        Self::Bypass { width, value }
    }

    /// Returns the raw symbol/value view of the token (the CDF symbol for
    /// `Mode`/`Coeff`, or the literal value for `Bypass`).
    pub(crate) const fn symbol(self) -> u8 {
        match self {
            Self::Mode(token) => token.symbol(),
            Self::Coeff(token) => token.symbol(),
            Self::Bypass { value, .. } => value as u8,
        }
    }
}

/// Composes the ordered minimal intra DC all-zero block trace: the AV2 § 5.20.5.3
/// mode-info prefix (`y_mode_set`, `y_mode_index`, `uv_mode`) followed by the
/// first `residual()` symbol, the luma `txb_skip` (§ 5.20.7.27 `all_zero`).
pub(crate) fn compose_minimal_intra_dc_all_zero_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let total = modes
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "all-zero block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "all-zero block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(luma_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    Ok(trace)
}

/// Composes the complete ordered minimal intra DC all-zero block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the per-plane § 5.20.7.27 `all_zero`
/// (`txb_skip`) symbols for luma, U, and V (each `1` for an all-zero block),
/// read in `residual()` plane order Y, U, V.
pub(crate) fn compose_minimal_intra_dc_complete_all_zero_block_trace()
-> Result<Vec<BlockSymbolToken>> {
    let mut trace = compose_minimal_intra_dc_all_zero_block_trace()?;
    trace
        .try_reserve_exact(2)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "complete all-zero block trace",
        })?;
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes a coded intra DC block trace for a single nonzero luma DC coefficient
/// of unsigned `magnitude` and the given sign: the AV2 § 5.20.5.3 mode-info
/// prefix, then the luma `residual()` coded coefficient tokens (§ 5.20.7.27,
/// including `coeff_br` for `magnitude > LF_NUM_BASE_LEVELS`), then the all-zero U
/// and V `txb_skip` symbols, in `residual()` plane order Y, U, V.
fn compose_coded_dc_block_trace(magnitude: u32, negative: bool) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = luma_dc_coded_tokens(MINIMAL_COEFF_CDF_Q_CTX, magnitude, negative)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "coded block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes the minimal ordered intra DC *coded* block trace: the mode-info
/// prefix, then the luma `residual()` for a single coded DC coefficient
/// (`txb_skip == 0`, `eob_pt_16`, `coeff_base_eob`, `dc_sign` per § 5.20.7.27),
/// then the all-zero U and V `txb_skip` symbols.
///
/// The luma block carries one nonzero DC coefficient of value `+1`; the chroma
/// planes are all-zero. This is the minimal *non-degenerate* (actually coded)
/// intra block symbol sequence.
pub(crate) fn compose_minimal_intra_dc_coded_block_trace() -> Result<Vec<BlockSymbolToken>> {
    compose_coded_dc_block_trace(MINIMAL_CODED_DC_MAGNITUDE, MINIMAL_CODED_DC_NEGATIVE)
}

/// Composes the minimal ordered intra DC coded *base-range* block trace: like
/// [`compose_minimal_intra_dc_coded_block_trace`] but with a luma DC magnitude in
/// the § 5.20.7.27 base-range tier, so the luma `residual()` additionally emits a
/// `coeff_br` symbol after `coeff_base_eob`.
///
/// The luma block carries one nonzero DC coefficient of value `+6` (level 5 base
/// plus `coeff_br = 1`); the chroma planes are all-zero.
pub(crate) fn compose_minimal_intra_dc_br_block_trace() -> Result<Vec<BlockSymbolToken>> {
    compose_coded_dc_block_trace(MINIMAL_BR_DC_MAGNITUDE, MINIMAL_BR_DC_NEGATIVE)
}

/// Result of proving a block-symbol trace through AV2 § 8.2 symbol bytes.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BlockSymbolTraceRoundtrip {
    bytes: Vec<u8>,
    decoded_symbols: Vec<u8>,
    symbol_count: u64,
}

impl BlockSymbolTraceRoundtrip {
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

/// Writes a block-symbol trace through one § 8.2 symbol encoder and decodes it
/// back through one symbol decoder, sharing CDF state across the whole sequence.
pub(crate) fn roundtrip_block_symbol_trace(
    trace: &[BlockSymbolToken],
) -> Result<BlockSymbolTraceRoundtrip> {
    let mut encode_cdfs = BlockSymbolTraceCdfRows::from_defaults();
    // One operation per CDF symbol and per bypass-literal bit, plus headroom.
    let trace_cost = trace
        .iter()
        .map(|token| match token {
            BlockSymbolToken::Bypass { width, .. } => *width as usize,
            _ => 1,
        })
        .sum::<usize>();
    let budget = trace_cost.saturating_add(BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM);
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new()
            .with_max_output_bytes(budget)
            .with_max_operations(budget),
    );
    for (index, token) in trace.iter().enumerate() {
        match token {
            BlockSymbolToken::Bypass { width, value } => {
                encoder
                    .write_literal(*value, *width)
                    .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?;
            }
            _ => {
                encoder
                    .write_symbol(
                        encode_cdfs.row_mut(*token, index)?,
                        Symbol::new(token.symbol()),
                    )
                    .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?;
            }
        }
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::BlockSymbolTraceSymbolEncodeFinish { source })?;
    let bytes = output.into_bytes();

    let mut decode_cdfs = BlockSymbolTraceCdfRows::from_defaults();
    let mut decoder = SymbolDecoder::with_config(&bytes, SymbolDecoderConfig::new())
        .map_err(|source| Error::BlockSymbolTraceSymbolDecodeInit { source })?;
    let mut decoded_symbols = Vec::new();
    decoded_symbols
        .try_reserve_exact(trace.len())
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "roundtrip decoded symbols",
        })?;
    for (index, token) in trace.iter().enumerate() {
        let decoded = match token {
            BlockSymbolToken::Bypass { width, value } => {
                // A bypass literal carries no CDF. Verify the FULL-WIDTH value
                // roundtrips (the `decoded_symbols` view below truncates to u8, so
                // this u32 check is what proves a wide literal — e.g. the golomb
                // tail — was reproduced exactly, not just its low byte).
                let actual = decoder
                    .read_literal(*width)
                    .map_err(|source| Error::BlockSymbolTraceSymbolRead { index, source })?;
                if actual != *value {
                    return Err(Error::BlockSymbolTraceLiteralMismatch {
                        index,
                        width: *width,
                        expected: *value,
                        actual,
                    });
                }
                actual as u8
            }
            _ => {
                let decoded = decoder
                    .read_symbol(decode_cdfs.row_mut(*token, index)?)
                    .map_err(|source| Error::BlockSymbolTraceSymbolRead { index, source })?
                    .get();
                if decoded != token.symbol() {
                    return Err(Error::BlockSymbolTraceSymbolMismatch {
                        index,
                        expected: token.symbol(),
                        actual: decoded,
                    });
                }
                decoded
            }
        };
        decoded_symbols.push(decoded);
    }
    let summary = decoder
        .finish()
        .map_err(|source| Error::BlockSymbolTraceSymbolDecodeFinish { source })?;

    Ok(BlockSymbolTraceRoundtrip {
        bytes,
        decoded_symbols,
        symbol_count: summary.symbol_count,
    })
}

/// Unified scoped default-CDF rows for the minimal block-symbol trace, built
/// directly from `splot-core` defaults so the trace module does not reach into
/// the emitter modules' private CDF-row internals.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BlockSymbolTraceCdfRows {
    y_mode_set: [i32; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [i32; INTRA_MODE_CDF_ROW_LEN],
    uv_mode_non_directional: [i32; INTRA_MODE_CDF_ROW_LEN],
    luma_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip: [i32; V_TXB_SKIP_CDF_ROW_LEN],
    eob_pt_16: [i32; EOB_PT_16_CDF_ROW_LEN],
    coeff_base_lf_eob: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_br_lf: [i32; COEFF_BR_LF_CDF_ROW_LEN],
    dc_sign: [i32; DC_SIGN_CDF_ROW_LEN],
}

impl BlockSymbolTraceCdfRows {
    fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index_tile_origin: DEFAULT_Y_MODE_INDEX_CDF[TILE_ORIGIN_Y_MODE_INDEX_CTX],
            uv_mode_non_directional: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
                [NON_DIRECTIONAL_UV_MODE_CTX],
            luma_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][V_TXB_SKIP_CTX_NEUTRAL],
            eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            coeff_base_lf_eob: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX][COEFF_BR_LF_CTX_DC],
            dc_sign: DEFAULT_DC_SIGN_CDF[MINIMAL_COEFF_CDF_Q_CTX][DC_SIGN_PLANE_TYPE_LUMA]
                [DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
        }
    }

    fn row_mut(&mut self, token: BlockSymbolToken, index: usize) -> Result<&mut [i32]> {
        match token {
            // Bypass literals carry no CDF row; `roundtrip_block_symbol_trace`
            // dispatches them before ever calling `row_mut`, so this arm is
            // unreachable in practice.
            BlockSymbolToken::Bypass { .. } => {
                Err(Error::BlockSymbolTraceUnsupportedSelector { index })
            }
            BlockSymbolToken::Mode(mode) => match mode.selector() {
                IntraModeCdfRowSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
                IntraModeCdfRowSelector::YModeIndex {
                    ctx: TILE_ORIGIN_Y_MODE_INDEX_CTX,
                } => Ok(self.y_mode_index_tile_origin.as_mut_slice()),
                IntraModeCdfRowSelector::UvModeCflNotAllowed {
                    ctx: NON_DIRECTIONAL_UV_MODE_CTX,
                } => Ok(self.uv_mode_non_directional.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
            BlockSymbolToken::Coeff(coeff) => match coeff.selector() {
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.luma_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.u_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::VTxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: V_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.v_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt16 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_16.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBrLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: COEFF_BR_LF_CTX_DC,
                } => Ok(self.coeff_br_lf.as_mut_slice()),
                CoefficientCdfRowSelector::DcSign {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: DC_SIGN_PLANE_TYPE_LUMA,
                    group: DC_SIGN_GROUP_VISIBLE,
                    ctx: DC_SIGN_CTX_NEUTRAL,
                } => Ok(self.dc_sign.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::intra_mode_emission::{
        IntraModeCdfRowSelector, IntraModeSyntax, roundtrip_intra_mode_tokens,
    };

    #[test]
    fn composes_ordered_mode_info_prefix() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();

        assert_eq!(trace.len(), 3);
        assert_eq!(trace[0].syntax(), IntraModeSyntax::YModeSet);
        assert_eq!(trace[1].syntax(), IntraModeSyntax::YModeIndex);
        assert_eq!(trace[2].syntax(), IntraModeSyntax::UvMode);
        assert!(matches!(
            trace[0].selector(),
            IntraModeCdfRowSelector::YModeSet
        ));
        assert!(matches!(
            trace[1].selector(),
            IntraModeCdfRowSelector::YModeIndex { ctx: 0 }
        ));
        assert!(matches!(
            trace[2].selector(),
            IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx: 0 }
        ));
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
    }

    #[test]
    fn composed_trace_matches_concatenated_emitters() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let luma = emit_minimal_dc_luma_intra_mode().unwrap();
        let uv = emit_minimal_dc_chroma_uv_mode().unwrap();

        let mut expected = luma.tokens().to_vec();
        expected.extend_from_slice(uv.tokens());
        assert_eq!(trace, expected);
    }

    #[test]
    fn composed_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let proof = roundtrip_intra_mode_tokens(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0]);
        assert_eq!(proof.symbol_count(), 3);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let first = roundtrip_intra_mode_tokens(&trace).unwrap();
        let second = roundtrip_intra_mode_tokens(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn composes_all_zero_block_trace_in_order() {
        let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();

        assert_eq!(trace.len(), 4);
        assert!(matches!(trace[0], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[3], BlockSymbolToken::Coeff(_)));
        if let BlockSymbolToken::Mode(token) = trace[0] {
            assert_eq!(token.syntax(), IntraModeSyntax::YModeSet);
        }
        if let BlockSymbolToken::Mode(token) = trace[2] {
            assert_eq!(token.syntax(), IntraModeSyntax::UvMode);
        }
        // y_mode_set=0, y_mode_index=0, uv_mode=0, luma all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 1]
        );
    }

    #[test]
    fn unified_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1]);
        assert_eq!(proof.symbol_count(), 4);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn unified_roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn rejects_unsupported_unified_selector() {
        // A luma txb_skip token at a non-minimal coefficient CDF q-context is
        // outside the unified router's supported rows.
        let unsupported = BlockSymbolToken::Coeff(luma_all_zero_token(1));
        let err = roundtrip_block_symbol_trace(&[unsupported]).unwrap_err();

        assert!(matches!(
            err,
            Error::BlockSymbolTraceUnsupportedSelector { index: 0 }
        ));
    }

    #[test]
    fn composes_complete_all_zero_block_trace_in_order() {
        let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();

        assert_eq!(trace.len(), 6);
        // Mode prefix, then per-plane all_zero (Y, U, V) in residual() order.
        assert!(matches!(trace[0], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[3], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
        // y_mode_set=0, y_mode_index=0, uv_mode=0, then luma/U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn complete_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1, 1, 1]);
        assert_eq!(proof.symbol_count(), 6);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn complete_roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn composes_coded_block_trace_in_order() {
        let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();

        assert_eq!(trace.len(), 9);
        // Mode prefix (3), then coded luma residual (txb_skip, eob_pt_16,
        // coeff_base_eob, dc_sign), then all-zero U and V txb_skip.
        for token in &trace[0..3] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        for token in &trace[3..9] {
            assert!(matches!(token, BlockSymbolToken::Coeff(_)));
        }
        // y_mode_set=0, y_mode_index=0, uv_mode=0, luma txb_skip=0 (coded),
        // eob_pt_16=0, coeff_base_eob=0 (mag 1), dc_sign=0 (positive),
        // then U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 0, 0, 1, 1]
        );
    }

    #[test]
    fn coded_block_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 0, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 9);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn coded_block_roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn composes_br_block_trace_in_order() {
        let trace = compose_minimal_intra_dc_br_block_trace().unwrap();

        assert_eq!(trace.len(), 10);
        // Mode prefix (3), coded luma residual with coeff_br (5), U/V txb_skip (2).
        for token in &trace[0..3] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        for token in &trace[3..10] {
            assert!(matches!(token, BlockSymbolToken::Coeff(_)));
        }
        // y_mode_set=0, y_mode_index=0, uv_mode=0, txb_skip=0, eob_pt_16=0,
        // coeff_base_eob=4 (level 5), coeff_br=1 (magnitude 6), dc_sign=0,
        // then U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 4, 1, 0, 1, 1]
        );
    }

    #[test]
    fn br_block_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_br_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 4, 1, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 10);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn br_block_roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_br_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn bypass_literals_interleave_with_cdf_symbols() {
        // §8.2.5 bypass literals (the foundation for non-luma-DC `sign_bit` and
        // the golomb tail) must roundtrip bit-exactly through the same coder that
        // carries the CDF symbols.
        let mut trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
        trace.push(BlockSymbolToken::bypass(1, 1)); // a 1-bit sign-like literal
        trace.push(BlockSymbolToken::bypass(4, 13)); // a wider literal
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        // The all-zero block symbols, then the two bypass values (value-as-u8).
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1, 1, 1, 1, 13]);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn bypass_literal_roundtrip_is_deterministic() {
        let mut trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
        trace.push(BlockSymbolToken::bypass(3, 5));
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn wide_bypass_literals_roundtrip_full_width() {
        // Literals wider than 8 bits (the golomb tail uses up to L(32)) must
        // roundtrip their FULL value: the budget scales with the bit width, and
        // the full-width check rejects truncation — so `roundtrip_block_symbol_trace`
        // returning Ok proves the exact value was reproduced, not just its low byte.
        let mut trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
        trace.push(BlockSymbolToken::bypass(16, 0x1234));
        trace.push(BlockSymbolToken::bypass(32, 0xDEAD_BEEF));
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert!(!proof.bytes().is_empty());
        // The u8 view truncates the wide values to their low byte; the Ok above is
        // the full-width proof.
        assert_eq!(proof.decoded_symbols().last(), Some(&0xEFu8));
    }
}
