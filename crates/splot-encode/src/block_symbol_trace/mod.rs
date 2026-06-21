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
    DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_EOB_EXTRA_CDF,
    DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_1024_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF,
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
    PartitionCdfRowSelector, PartitionToken, ROOT_64X64_DO_SPLIT_CTX, ROOT_PARTITION_PLANE_START,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const TXB_SKIP_CDF_ROW_LEN: usize = 3;
/// `TileDoSplitCdf` is a binary CDF: `[cdf0, count, 0]` (length 3).
const DO_SPLIT_CDF_ROW_LEN: usize = 3;
const V_TXB_SKIP_CDF_ROW_LEN: usize = 3;
const EOB_PT_16_CDF_ROW_LEN: usize = 6;
/// `TileEobPt1024Cdf` rows hold 8 symbols (`[i32; 9]`).
const EOB_PT_1024_CDF_ROW_LEN: usize = 9;
/// `TileEobExtraCdf` is a binary CDF: `[cdf0, count, 0]` (length 3).
const EOB_EXTRA_CDF_ROW_LEN: usize = 3;
const COEFF_BASE_LF_EOB_CDF_ROW_LEN: usize = 6;
const COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN: usize = 6;
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
// AV2 § 8.3.2 `txb_skip` `txSzCtx` for the general-path single transforms that fill
// a 64x64 superblock leaf: `TX_64X64` luma is `4`, `TX_32X32` chroma is `3` (see
// `coefficient_tokenization`; both empirically confirmed against the decoder).
const TX_SIZE_64X64_CTX: usize = 4;
const TX_SIZE_32X32_CTX: usize = 3;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_U_TXB_SKIP_CTX_NEUTRAL: usize = 6;
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
// AV2 § 8.3.2 (`all_zero`, lines 1257-1262): the V-plane `txb_skip` context adds
// +6 when `EobU != 0` (the U plane is coded). With empty neighbours and
// `bw*bh == w*h` (no +3), a block whose U plane is coded uses V context 6.
const CHROMA_V_TXB_SKIP_CTX_EOBU: usize = 6;
// AV2 § 5.20.7.27 / § 8.3.2 neutral coded-DC luma coefficient contexts.
const EOB_CTX_LUMA_INTRA: usize = 0;
// Intra chroma eob context (`eobCtx = (plane > 0) ? 2 : is_inter`).
const EOB_CTX_CHROMA: usize = 2;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const COEFF_BR_LF_CTX_DC: usize = 0;
// The minimal eob=2 multi-coefficient block (one nonzero AC at scan pos 1, DC=0):
// §5.20.7.27 `eob_pt_16` symbol 1 → eobPt 2 → eob 2; the AC at scan index 1 uses
// `coeff_base_eob_ctx(c=1) = 1` (low-frequency); the DC at scan index 0 uses the
// non-EOB `coeff_base` at the §8.3.2 low-frequency context derived from the AC's
// level (`coeff_base_lf_luma_context` → 1 for an AC level-1 neighbour at pos 1).
// `tcq_ctx = (tcqState >> 1) & 1` is 0 when TCQ is off.
const EOB_PT_16_SYMBOL_EOB2: u8 = 1;
// §5.20.8.2 `transform_type()` reads `intra_tx_type` right after the eob reading
// (§5.20.7.27 line 15474), before the base pass. For a 4x4 `DC_PRED`
// `TX_SET_INTRA_1` block, symbol 0 selects `DCT_DCT` (`Md_Idx_To_Type[0][0][0] = 0`);
// it is inserted after the `eob_pt_16` token (index 4: 3 modes + `all_zero` + `eob_pt`).
const INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4: usize = 0;
const INTRA_TX_TYPE_DCT_DCT_SYMBOL: u8 = 0;
const INTRA_TX_TYPE_SET1_CDF_ROW_LEN: usize = 8;
const EOB_PT_16_TRACE_INDEX: usize = 4;
// §5.20.8.2 `transform_type()` reads `sec_tx_type` (the IST secondary transform) at
// line 16613, right after `intra_tx_type` (line 16529), when the IST condition holds.
// For this 4x4 DCT_DCT DC_PRED eob=2 block with `enable_intra_ist == 1` it holds
// (`eob 2 != 1`, `!Lossless`, `TxType == DCT_DCT`, `YMode != PAETH`, `eob 2 <= eobLim
// = IST_4X4_HEIGHT = 8`), so the symbol is read; symbol 0 is `sec_tx_type = 0` (IST
// off), which reads no `most_probable_stx_set`. It is inserted after `intra_tx_type`.
const SEC_TX_TYPE_INTRA_BANK: usize = 0;
const SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4: usize = 0;
const SEC_TX_TYPE_IST_OFF_SYMBOL: u8 = 0;
const SEC_TX_TYPE_INTRA_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_LF_EOB_CTX_EOB2_AC: usize = 1;
const COEFF_BASE_LF_CTX_EOB2_DC: usize = 1;
// The DC `coeff_base` low-frequency context for a *visible* eob=2/eob=3 block: a level-4 AC
// neighbour raises the DC's § 8.3.2 significant-neighbour sum to low-frequency context 2 (vs 1
// for the minimal level-1 AC). Per-AC-level, via `coeff_base_lf_luma_context`.
const COEFF_BASE_LF_CTX_VISIBLE_AC_DC: usize = 2;
// The non-EOB `coeff_base` low-frequency context for scan index 1 (raster 32) in the eob=3 block
// whose only nonzero coefficient is the EOB at scan index 2 (raster 1): that EOB is not a § 8.3.2
// neighbour of (1,0), so the off-axis band maps to `min(0,6) + 9 = 9` (verified).
const COEFF_BASE_LF_CTX_AC_BAND_BASE: usize = 9;
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;
const COEFF_BASE_LF_CDF_ROW_LEN: usize = 7;
// The minimal eob=2 block's coefficient levels: a single AC of level 1 at scan
// index 1 and a zero DC at scan index 0. The AC's raster position is derived from
// the AV2 2D scan order (`scan[1] = 4` in the 4x4 order `[0, 4, 1, ...]`, i.e.
// row 1 col 0), not assumed equal to the scan index.
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
// A chroma DC `sign_bit` is a § 8.2.5 `L(1)` bypass literal (§ 5.20.7.27 codes
// the luma DC sign as `dc_sign` and the directional luma axis signs as
// `dc_sign_horz_vert`, both CDF; every other sign is `sign_bit`).
const CHROMA_SIGN_BIT_WIDTH: u32 = 1;
// Minimal coded luma block: a single DC coefficient of value +1.
const MINIMAL_CODED_DC_MAGNITUDE: u32 = 1;
const MINIMAL_CODED_DC_NEGATIVE: bool = false;
// Minimal coded chroma U block: a single DC coefficient of value +1.
const MINIMAL_CODED_CHROMA_DC_MAGNITUDE: u32 = 1;
const MINIMAL_CODED_CHROMA_DC_NEGATIVE: bool = false;
// Minimal base-range coded luma block: a single DC coefficient of value +6
// (level 5 base + `coeff_br = 1`).
const MINIMAL_BR_DC_MAGNITUDE: u32 = 6;
const MINIMAL_BR_DC_NEGATIVE: bool = false;
// AV2 § 5.20.7.27 `maxLevel` for the LF luma DC EOB coefficient
// (LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1). The § 5.20.7.28 `read_quant`
// golomb tail encodes `x = magnitude - maxLevel` once the level reaches it.
const GOLOMB_MAXLEVEL: u32 = 8;
// `read_quant` for the first/only DC coefficient has `hrLevelAvg = 0` (§ 5.20.7.27
// init), so `predLevel = 0`, `m = Clip3(1, 6, GetMsb(0)) = 1`, `k = m + 1 = 2`,
// `cMax = Min(m + 4, 6) = 5`. In the finite-q path (`q < cMax`) `length = m = 1`,
// `xBase = q << 1`, so `x = 2q + coeff_rem` → `q = x >> 1`, `coeff_rem = x & 1`.
// The finite-q path covers `x` in `0..=9` (q `0..=4`), i.e. magnitude `8..=17`.
const GOLOMB_DC_M: u32 = 1;
const GOLOMB_FINITE_Q_MAX: u32 = 4;
// Top of the finite-q magnitude range: maxLevel + (2*GOLOMB_FINITE_Q_MAX + 1) =
// 8 + 9 = 17. Above this `q == cMax` and the golomb-prefix path applies.
const GOLOMB_FINITE_Q_MAGNITUDE_MAX: u32 = GOLOMB_MAXLEVEL + (2 * GOLOMB_FINITE_Q_MAX + 1);
// Golomb-prefix path (`q == cMax`, magnitude 18+). `k = m + 1 = 2`; the q_length
// loop emits `cMax = GOLOMB_FINITE_Q_MAX + 1 = 5` zeros (no terminating 1). Then
// `xBase = (cMax << m) + (1 << length) - (1 << k) = bias + 2^length`, where the
// constant bias `(cMax << m) - (1 << k) = 10 - 4 = 6`. Encoding `x = magnitude - 8`
// (x >= 10): `length = GetMsb(x - 6)`, `golomb_zeros = length - k`,
// `coeff_rem = (x - 6) - 2^length` as an `L(length)` literal.
const GOLOMB_DC_K: u32 = GOLOMB_DC_M + 1;
const GOLOMB_PREFIX_Q_ZEROS: u32 = GOLOMB_FINITE_Q_MAX + 1;
const GOLOMB_PREFIX_XBASE_BIAS: u32 = (GOLOMB_PREFIX_Q_ZEROS << GOLOMB_DC_M) - (1 << GOLOMB_DC_K);
// Supported golomb-prefix span for this brick: golomb `length` 2..=8 → magnitude
// 18..=525 (`coeff_rem` <= 255, exact in the decoded u8 view). Larger magnitudes
// are a trivial wider-`coeff_rem` extension, rejected here with a typed error.
const GOLOMB_PREFIX_LENGTH_MAX: u32 = 8;
const GOLOMB_PREFIX_MAGNITUDE_MIN: u32 = GOLOMB_FINITE_Q_MAGNITUDE_MAX + 1;
const GOLOMB_PREFIX_MAGNITUDE_MAX: u32 =
    GOLOMB_MAXLEVEL + GOLOMB_PREFIX_XBASE_BIAS + (1 << (GOLOMB_PREFIX_LENGTH_MAX + 1)) - 1;
// Minimal golomb-prefix coded luma block: magnitude +18 (x=10, length=2,
// golomb_zeros=0, coeff_rem=0).
const MINIMAL_GOLOMB_PREFIX_DC_MAGNITUDE: u32 = GOLOMB_PREFIX_MAGNITUDE_MIN;
const MINIMAL_GOLOMB_PREFIX_DC_NEGATIVE: bool = false;
// Minimal golomb-tail coded luma block: a single DC coefficient of value +10
// (level reaches maxLevel 8, then `x = 2` → q=1, coeff_rem=0).
const MINIMAL_GOLOMB_DC_MAGNITUDE: u32 = 10;
const MINIMAL_GOLOMB_DC_NEGATIVE: bool = false;
// Headroom (operations + output bytes) added on top of the per-trace cost. The
// roundtrip's encoder budget scales with the trace: one operation per CDF symbol
// and one per bypass-literal bit (`write_literal` charges per bit), so a wide
// `L(n)` literal — e.g. the golomb tail — is not rejected by a fixed cap.
const BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM: usize = 32;

mod cdf_rows;
mod coder;
mod compose;
mod golomb;

use cdf_rows::BlockSymbolTraceCdfRows;

// Re-export every crate-reachable item at its original `crate::block_symbol_trace::…`
// path so the `general_intra_trace` consumers, `coefficient_tokenization`, and the
// sibling tests keep resolving unchanged after the split.
#[allow(unused_imports)]
pub(crate) use coder::{
    BlockSymbolToken, BlockSymbolTraceRoundtrip, encode_block_symbol_trace,
    roundtrip_block_symbol_trace,
};
#[allow(unused_imports)]
pub(crate) use compose::{
    compose_minimal_intra_dc_all_zero_block_trace, compose_minimal_intra_dc_block_mode_trace,
    compose_minimal_intra_dc_br_block_trace, compose_minimal_intra_dc_coded_block_trace,
    compose_minimal_intra_dc_coded_chroma_block_trace,
    compose_minimal_intra_dc_complete_all_zero_block_trace,
    compose_minimal_intra_two_coeff_block_trace,
    compose_minimal_intra_two_coeff_block_trace_with_ist,
    compose_minimal_intra_two_coeff_block_trace_with_tx_type,
};
#[allow(unused_imports)]
pub(crate) use golomb::{
    compose_intra_dc_golomb_block_trace, compose_intra_dc_golomb_prefix_block_trace,
    compose_minimal_intra_dc_golomb_block_trace,
    compose_minimal_intra_dc_golomb_prefix_block_trace,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "tests.rs"]
mod tests;
