// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder block-symbol trace composition.
//!
//! This module is the home for the growing ordered block-symbol trace — the minimal-tier
//! composers plus the shared `BlockSymbolTraceCdfRows`. The trace advances, in order: the
//! § 5.20.5.3 mode prefix, the per-plane § 5.20.7.27 `txb_skip` all-zero block, the coded
//! single-DC magnitude vocabulary (`eob_pt_16`/`coeff_base_eob`/`coeff_br`/`dc_sign`, the § 8.2.5
//! bypass literals, the U-plane chroma DC, and the § 5.20.7.28 `read_quant` golomb tails up to
//! magnitude 525), and the minimal eob=2 multi-coefficient block (alone, or with the § 5.20.8.2
//! `intra_tx_type` and `sec_tx_type` IST symbols)
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
//!
//! The unified `BlockSymbolToken` spans partition, mode, and coefficient kinds, and
//! `roundtrip_block_symbol_trace` proves the combined sequence through one § 8.2 coder with
//! shared CDF state, routing each token to its scoped `splot-core` default CDF row. The
//! `BlockSymbolTraceCdfRows` table here is shared with the general-path composers in
//! `general_intra_trace` (which add `eob_extra` and the higher eob/scan contexts); these
//! minimal composers themselves do not emit eob > 2, the chroma base-range/golomb tiers, V-plane
//! coded coefficients, partition splits beyond the root `PARTITION_NONE`, tile CDF lifecycle,
//! packets, a public encoder API, or modes beyond the DC minimal tier.
//!
//! The responsibilities are split across submodules to keep each file under the
//! 1000-line source budget: [`coder`] (the `BlockSymbolToken` and the § 8.2 coder
//! driver), [`cdf_rows`] (the shared scoped-CDF-row table), [`compose`] (the
//! minimal-tier composers), and [`golomb`] (the § 5.20.7.28 golomb-tail composers).

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF, DEFAULT_COEFF_BASE_LF_CDF,
    DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF, DEFAULT_COEFF_BR_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_DO_SPLIT_CDF,
    DEFAULT_DO_SQUARE_SPLIT_CDF, DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF,
    DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_1024_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF,
    DEFAULT_SEC_TX_TYPE_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientEntropyToken, CoefficientTokenSyntax,
    chroma_u_all_zero_token, chroma_u_dc_coded_coeff_tokens, chroma_v_all_zero_token,
    coded_luma_all_zero_token, coeff_base_lf_eob_token, coeff_base_lf_luma_context,
    coeff_base_lf_token, eob_pt_16_token, intra_tx_type_set1_token, luma_all_zero_token,
    luma_dc_coded_tokens, luma_dc_golomb_level_tokens, luma_dc_sign_token, sec_tx_type_intra_token,
};
use splot_recon::{TransformClass, coefficient_scan_order};

use crate::error::{Error, Result};
use crate::intra_mode_emission::{
    IntraModeCdfRowSelector, IntraModeToken, emit_minimal_dc_chroma_uv_mode,
    emit_minimal_dc_luma_intra_mode,
};
use crate::partition_emission::{
    PartitionCdfRowSelector, PartitionToken, ROOT_64X64_DO_SPLIT_CTX,
    ROOT_64X64_DO_SQUARE_SPLIT_CTX, ROOT_PARTITION_PLANE_START,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const TXB_SKIP_CDF_ROW_LEN: usize = 3;
/// `TileDoSplitCdf` is a binary CDF: `[cdf0, count, 0]` (length 3).
const DO_SPLIT_CDF_ROW_LEN: usize = 3;
/// `TileDoSquareSplitCdf` is a binary CDF: `[cdf0, count, 0]` (length 3).
const DO_SQUARE_SPLIT_CDF_ROW_LEN: usize = 3;
const V_TXB_SKIP_CDF_ROW_LEN: usize = 3;
const EOB_PT_16_CDF_ROW_LEN: usize = 6;
/// `TileEobPt1024Cdf` rows hold 8 symbols (`[i32; 9]`).
const EOB_PT_1024_CDF_ROW_LEN: usize = 9;
/// `TileEobPt256Cdf` rows hold 8 symbols (`[i32; 9]`).
const EOB_PT_256_CDF_ROW_LEN: usize = 9;
/// `TileEobExtraCdf` is a binary CDF: `[cdf0, count, 0]` (length 3).
const EOB_EXTRA_CDF_ROW_LEN: usize = 3;
const COEFF_BASE_LF_EOB_CDF_ROW_LEN: usize = 6;
const COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN: usize = 6;
const COEFF_BR_LF_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_LF_CTX_COUNT: usize = 33;
const COEFF_BASE_LF_EOB_CTX_COUNT: usize = 4;
const COEFF_BR_LF_CTX_COUNT: usize = 14;
const COEFF_BASE_EOB_CTX_COUNT: usize = 4;
const COEFF_BR_CTX_COUNT: usize = 7;
const COEFF_BASE_EOB_CDF_ROW_LEN: usize = 4;
const COEFF_BR_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_CTX_COUNT: usize = 20;
const COEFF_BASE_CDF_ROW_LEN: usize = 5;
const DC_SIGN_CDF_ROW_LEN: usize = 3;
const TILE_ORIGIN_Y_MODE_INDEX_CTX: usize = 0;
const NON_DIRECTIONAL_UV_MODE_CTX: usize = 0;
const MINIMAL_COEFF_CDF_Q_CTX: usize = 0;
const LUMA_PLANE_TYPE: usize = 0;
const TX_SIZE_4X4_CTX: usize = 0;
const TX_SIZE_64X64_CTX: usize = 4;
const TX_SIZE_32X32_CTX: usize = 3;
const TX_SIZE_16X16_CTX: usize = 2;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_U_TXB_SKIP_CTX_NEUTRAL: usize = 6;
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_V_TXB_SKIP_CTX_EOBU: usize = 6;
const EOB_CTX_LUMA_INTRA: usize = 0;
const EOB_CTX_CHROMA: usize = 2;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const EOB_PT_16_SYMBOL_EOB2: u8 = 1;
const INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4: usize = 0;
const INTRA_TX_TYPE_DCT_DCT_SYMBOL: u8 = 0;
const INTRA_TX_TYPE_SET1_CDF_ROW_LEN: usize = 8;
const EOB_PT_16_TRACE_INDEX: usize = 4;
const SEC_TX_TYPE_INTRA_BANK: usize = 0;
const SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4: usize = 0;
const SEC_TX_TYPE_IST_OFF_SYMBOL: u8 = 0;
const SEC_TX_TYPE_INTRA_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_LF_EOB_CTX_EOB2_AC: usize = 1;
const COEFF_BASE_LF_CTX_EOB2_DC: usize = 1;
const COEFF_BASE_LF_CTX_VISIBLE_AC_DC: usize = 2;
const COEFF_BASE_LF_CTX_AC_BAND_BASE: usize = 9;
const COEFF_BASE_LF_CTX_2D_DC: usize = 4;
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;
const COEFF_BASE_LF_CDF_ROW_LEN: usize = 7;
const EOB2_AC_LEVEL: u8 = 1;
const EOB2_AC_SCAN_INDEX: usize = 1;
const EOB2_AC_NEGATIVE: bool = false;
const EOB2_DC_LEVEL: u8 = 0;
const TX_4X4_BWL: u32 = 2;
const TX_4X4_WIDTH: usize = 4;
const TX_4X4_HEIGHT: usize = 4;
const TX_CLASS_2D: usize = 0;
const DC_SIGN_PLANE_TYPE_LUMA: usize = 0;
const DC_SIGN_GROUP_VISIBLE: usize = 0;
const DC_SIGN_CTX_NEUTRAL: usize = 0;
const CHROMA_SIGN_BIT_WIDTH: u32 = 1;
const MINIMAL_CODED_DC_MAGNITUDE: u32 = 1;
const MINIMAL_CODED_DC_NEGATIVE: bool = false;
const MINIMAL_CODED_CHROMA_DC_MAGNITUDE: u32 = 1;
const MINIMAL_CODED_CHROMA_DC_NEGATIVE: bool = false;
const MINIMAL_BR_DC_MAGNITUDE: u32 = 6;
const MINIMAL_BR_DC_NEGATIVE: bool = false;
const GOLOMB_MAXLEVEL: u32 = 8;
const GOLOMB_DC_M: u32 = 1;
const GOLOMB_FINITE_Q_MAX: u32 = 4;
const GOLOMB_FINITE_Q_MAGNITUDE_MAX: u32 = GOLOMB_MAXLEVEL + (2 * GOLOMB_FINITE_Q_MAX + 1);
const GOLOMB_DC_K: u32 = GOLOMB_DC_M + 1;
const GOLOMB_PREFIX_Q_ZEROS: u32 = GOLOMB_FINITE_Q_MAX + 1;
const GOLOMB_PREFIX_XBASE_BIAS: u32 = (GOLOMB_PREFIX_Q_ZEROS << GOLOMB_DC_M) - (1 << GOLOMB_DC_K);
const GOLOMB_PREFIX_LENGTH_MAX: u32 = 8;
const GOLOMB_PREFIX_MAGNITUDE_MIN: u32 = GOLOMB_FINITE_Q_MAGNITUDE_MAX + 1;
const GOLOMB_PREFIX_MAGNITUDE_MAX: u32 =
    GOLOMB_MAXLEVEL + GOLOMB_PREFIX_XBASE_BIAS + (1 << (GOLOMB_PREFIX_LENGTH_MAX + 1)) - 1;
const MINIMAL_GOLOMB_PREFIX_DC_MAGNITUDE: u32 = GOLOMB_PREFIX_MAGNITUDE_MIN;
const MINIMAL_GOLOMB_PREFIX_DC_NEGATIVE: bool = false;
const MINIMAL_GOLOMB_DC_MAGNITUDE: u32 = 10;
const MINIMAL_GOLOMB_DC_NEGATIVE: bool = false;
const BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM: usize = 32;

mod cdf_rows;
mod coder;
mod compose;
mod golomb;

use cdf_rows::BlockSymbolTraceCdfRows;

#[cfg(test)]
pub(crate) use coder::roundtrip_block_symbol_trace;
pub(crate) use coder::{BlockSymbolToken, encode_block_symbol_trace};
pub(crate) use compose::compose_minimal_intra_dc_block_mode_trace;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "tests.rs"]
mod tests;
