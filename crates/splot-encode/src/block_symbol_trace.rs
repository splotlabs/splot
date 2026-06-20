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
//! the foundation for non-luma-DC `sign_bit` and the golomb tail), and
//! `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` (a coded U-plane DC coefficient whose
//! sign is a `sign_bit` bypass literal, with the §8.3.2 chroma contexts and the V
//! `txb_skip` EobU context), and
//! `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` (the §5.20.7.28 `read_quant` finite-q
//! golomb tail: a larger luma DC coefficient's `coeff_rem` bypass bits), and
//! `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` (the §5.20.7.28 golomb-*prefix* path,
//! `q == cMax`: the q_length / `golomb_length` unary codes + a sized `coeff_rem`
//! for luma DC magnitude 18..=525), and
//! `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` (the first MULTI-coefficient block: an eob=2
//! luma block with one nonzero AC at scan pos 1 and a zero DC, exercising
//! `eob_pt_16=1`, the non-EOB `coeff_base` with a `Level[]`-derived §8.3.2
//! low-frequency context, and the AC `sign_bit` bypass), and
//! `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` (the same eob=2 block for the
//! default-`reduced_tx_set` `TX_SET_INTRA_1` config, inserting the §5.20.8.2
//! `intra_tx_type` DCT_DCT symbol after `eob_pt_16`), and
//! `ENC-INTRA-BLOCK-TRACE-IST` (that eob=2 block for `enable_intra_ist == 1`, adding
//! the §5.20.8.2 `sec_tx_type` IST symbol right after `intra_tx_type`),
//! reusing the merged mode emitters and the coefficient tokenization's per-plane
//! all-zero and coded-DC tokens
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
//!
//! On the general intra tile path the decoder reads the § 5.20.3.2 `do_split` partition
//! flag first, then AV2 § 5.20.5.3 `intra_frame_mode_info()` (`read_intra_y_mode()` then
//! `read_intra_uv_mode()`) before `residual()`, so the ordered trace is the partition flag,
//! then the mode prefix, then the residual symbols; the unified `BlockSymbolToken` spans
//! partition, mode, and coefficient kinds, and `roundtrip_block_symbol_trace` proves the
//! combined sequence through one § 8.2 coder with shared CDF state, routing each token to
//! its scoped CDF row from `splot-core` defaults.
//!
//! Its coefficient coverage is the single-DC magnitude vocabulary plus the minimal
//! eob=2 multi-coefficient block (with no transform-type symbol, with the
//! `TX_SET_INTRA_1` `intra_tx_type` symbol, or with both `intra_tx_type` and the
//! `sec_tx_type` IST symbol); it does not emit blocks with eob > 2, luma DC magnitude
//! beyond the golomb-prefix cap (525), high-frequency coefficients, the
//! `most_probable_stx_set` IST-set symbol (the IST trace uses `sec_tx_type = 0`),
//! non-`TX_SET_INTRA_1` / non-`DC_PRED` transform types, the chroma
//! base-range/golomb tiers, V-plane coded coefficients, partition splits beyond the root
//! `do_split == false` (`PARTITION_NONE`), tile CDF lifecycle, packets, a public encoder
//! API, or modes beyond the DC minimal tier.

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_EOB_PT_16_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF, DEFAULT_SEC_TX_TYPE_CDF,
    DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF, DEFAULT_V_TXB_SKIP_CDF,
    DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
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
    /// An AV2 § 5.20.3.2 partition token (here, the root `do_split`).
    Partition(PartitionToken),
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
            Self::Partition(token) => token.symbol(),
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

/// Composes the minimal ordered intra DC coded *chroma* block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the coded luma `residual()`, then the coded U
/// `residual()` — `txb_skip == 0`, chroma `eob_pt_16`, chroma `coeff_base_eob`
/// (CDF), then the U DC `sign_bit` as a § 8.2.5 `L(1)` bypass literal (a chroma
/// sign is not a `dc_sign` CDF symbol) — then the all-zero V `txb_skip` at the
/// § 8.3.2 V context 6 (`EobU != 0` once U is coded), in `residual()` plane order
/// Y, U, V.
///
/// The luma and U planes each carry one nonzero DC coefficient of value `+1`; the
/// V plane is all-zero. This is the minimal block whose chroma plane is coded.
pub(crate) fn compose_minimal_intra_dc_coded_chroma_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = luma_dc_coded_tokens(
        MINIMAL_COEFF_CDF_Q_CTX,
        MINIMAL_CODED_DC_MAGNITUDE,
        MINIMAL_CODED_DC_NEGATIVE,
    )?;
    let u_coeffs =
        chroma_u_dc_coded_coeff_tokens(MINIMAL_COEFF_CDF_Q_CTX, MINIMAL_CODED_CHROMA_DC_MAGNITUDE)?;
    // mode prefix + luma + U coefficients + the U `sign_bit` bypass + the V
    // all-zero `txb_skip`.
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(u_coeffs.len()))
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "coded chroma block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "coded chroma block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    // The U DC sign is a § 5.20.7.27 `sign_bit L(1)` bypass literal, not a CDF
    // `dc_sign` (that path is the luma DC / directional luma axis signs).
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        MINIMAL_CODED_CHROMA_DC_NEGATIVE as u32,
    ));
    // The U plane is coded (`EobU != 0`), so the V `txb_skip` uses § 8.3.2 context
    // 6 (the `+6` EobU term), not the all-zero-U neutral context 0.
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        CHROMA_V_TXB_SKIP_CTX_EOBU,
    )));
    Ok(trace)
}

mod golomb;
// Re-exported for the sibling tests; the composers are not referenced by other
// non-test code in this module.
#[allow(unused_imports)]
pub(crate) use golomb::{
    compose_intra_dc_golomb_block_trace, compose_intra_dc_golomb_prefix_block_trace,
    compose_minimal_intra_dc_golomb_block_trace,
    compose_minimal_intra_dc_golomb_prefix_block_trace,
};

/// Composes the minimal eob=2 multi-coefficient luma block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the coded luma `residual()` for a block with
/// two scan positions — one nonzero AC coefficient (level 1) at scan index 1 and a
/// zero DC at scan index 0 — then the all-zero U and V `txb_skip`.
///
/// Per § 5.20.7.27 the residual is `all_zero=0`, `eob_pt_16=1` (eob 2), then the
/// base pass over `c = eob-1..0`: the AC `coeff_base_eob` at context 1 (the
/// EOB-position coefficient, level 1, at scan index 1 = raster position 4 = row 1
/// col 0) and the DC `coeff_base` at the § 8.3.2 low-frequency context derived from
/// the AC's `Level[]` (the AC is the DC's significant neighbour, so the context is
/// 1; derived via `coeff_base_lf_luma_context`, not hard-coded). The sign pass then
/// reads the AC `sign_bit` (an § 8.2.5 bypass literal — pos (1,0) is neither the
/// luma DC nor a directional axis under TX_CLASS_2D); the DC is zero, so it carries
/// no sign. The ten-token trace is `[0,0,0, 0, 1, 0, 0, 0, 1, 1]`.
///
/// Transform-type scope: § 5.20.7.27 calls `transform_type()` between `eob_pt_16`
/// and the base pass, and for `eob > 1` the `transform_type()` `eob == 1` shortcut
/// no longer infers `DCT_DCT`. This trace therefore assumes a transform-set
/// configuration where `transform_type()` reads NO `intra_tx_type` symbol — the
/// DCT-only set (`get_tx_set` returns `TX_SET_DCTONLY`) or `reduced_tx_set == 2` for
/// intra (§ 5.20.7.27, the `!(reduced_tx_set == 2 && is_inter == 0)` guard) — AND
/// `enable_intra_ist == 0`, since § 5.20.7.29 (line 16603) otherwise reads a
/// `sec_tx_type` (intra secondary transform) symbol before the base pass for an
/// `eob > 1` DCT_DCT block. Both are consistent with the block's plain DCT_DCT
/// transform; the general `eob > 1` `intra_tx_type` / `sec_tx_type` signaling
/// (`set > 0` and `reduced_tx_set != 2`, or `enable_intra_ist`) is a later brick.
///
/// This is the first multi-coefficient block trace. The § 8.2 roundtrip proves the
/// symbols are self-consistent; conformance of the data-dependent `coeff_base`
/// context against a real decoder is established at the packet milestone (AVM
/// cross-check).
pub(crate) fn compose_minimal_intra_two_coeff_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    // Derive the AC's raster position from the AV2 2D scan order (scan index 1 maps
    // to raster position 4 in the 4x4 order, not 1), then derive the DC's § 8.3.2
    // coeff_base low-frequency context from the AC's Level[] (the AC of level 1 is
    // the DC's significant neighbour).
    let mut scan = [0u16; TX_4X4_WIDTH * TX_4X4_HEIGHT];
    coefficient_scan_order(TX_4X4_WIDTH, TX_4X4_HEIGHT, TransformClass::TwoD, &mut scan).map_err(
        |_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient scan order",
        },
    )?;
    let ac_raster_pos = scan[EOB2_AC_SCAN_INDEX] as usize;
    let mut level = [0u32; TX_4X4_WIDTH * TX_4X4_HEIGHT];
    level[ac_raster_pos] = EOB2_AC_LEVEL as u32;
    let dc_ctx = coeff_base_lf_luma_context(
        0,
        TX_4X4_BWL,
        TX_4X4_WIDTH,
        TX_4X4_HEIGHT,
        TX_CLASS_2D,
        0,
        &level,
    );
    debug_assert_eq!(dc_ctx, COEFF_BASE_LF_CTX_EOB2_DC);
    let total = modes
        .len()
        .checked_add(7) // all_zero + eob_pt + AC base_eob + DC base + AC sign + U + V
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(coded_luma_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(eob_pt_16_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        EOB_CTX_LUMA_INTRA,
        EOB_PT_16_SYMBOL_EOB2,
    )));
    // Base pass (c = eob-1..0): the AC `coeff_base_eob` then the DC `coeff_base`.
    trace.push(BlockSymbolToken::Coeff(coeff_base_lf_eob_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        COEFF_BASE_LF_EOB_CTX_EOB2_AC,
        EOB2_AC_LEVEL,
    )));
    trace.push(BlockSymbolToken::Coeff(coeff_base_lf_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        dc_ctx,
        COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
        EOB2_DC_LEVEL,
    )));
    // Sign pass: the AC `sign_bit` (a §8.2.5 bypass literal); the zero DC has no sign.
    trace.push(BlockSymbolToken::bypass(1, EOB2_AC_NEGATIVE as u32));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes the minimal eob=2 multi-coefficient luma block trace WITH the
/// §5.20.8.2 `intra_tx_type` transform-type symbol, for the default-`reduced_tx_set`
/// `TX_SET_INTRA_1` configuration (removing the
/// [`compose_minimal_intra_two_coeff_block_trace`] `reduced_tx_set == 2` scope).
/// `transform_type()` is read right after `eob_pt_16` (§5.20.7.27 line 15474),
/// before the base pass; the 4x4 `DC_PRED` symbol is 0 (`DCT_DCT`). The eleven-token
/// trace is the eob=2 trace with that symbol inserted after `eob_pt_16`:
/// `[0,0,0, 0, 1, 0, 0, 0, 0, 1, 1]`. It still assumes `enable_intra_ist == 0` (no
/// `sec_tx_type`); that signaling is a later brick.
pub(crate) fn compose_minimal_intra_two_coeff_block_trace_with_tx_type()
-> Result<Vec<BlockSymbolToken>> {
    let base = compose_minimal_intra_two_coeff_block_trace()?;
    // Derive the insertion point from the `eob_pt_16` token kind so it tracks any
    // growth of the base trace, falling back to the known `EOB_PT_16_TRACE_INDEX`.
    let split = base
        .iter()
        .position(|token| {
            matches!(token, BlockSymbolToken::Coeff(coeff)
                if matches!(coeff.syntax(), CoefficientTokenSyntax::EobPt16))
        })
        .unwrap_or(EOB_PT_16_TRACE_INDEX)
        + 1;
    let total = base
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient tx-type block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient tx-type block trace",
        })?;
    trace.extend_from_slice(&base[..split]);
    trace.push(BlockSymbolToken::Coeff(intra_tx_type_set1_token(
        INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4,
        INTRA_TX_TYPE_DCT_DCT_SYMBOL,
    )));
    trace.extend_from_slice(&base[split..]);
    Ok(trace)
}

/// Composes the eob=2 trace with BOTH §5.20.8.2 transform-type symbols —
/// `intra_tx_type` AND `sec_tx_type` (the IST secondary transform) — for the
/// `enable_intra_ist == 1` configuration. `sec_tx_type` (§5.20.8.2 line 16613) is read
/// right after `intra_tx_type` (line 16529), before the base pass; for this 4x4 DCT_DCT
/// `DC_PRED` eob=2 block the IST condition holds (`eob 2 != 1 && !Lossless && TxType ==
/// DCT_DCT && YMode != PAETH && eob 2 <= eobLim = IST_4X4_HEIGHT = 8`), and symbol 0 is
/// `sec_tx_type = 0` (IST off, no `most_probable_stx_set`). The twelve-token trace is
/// the tx-type trace with that symbol inserted after `intra_tx_type`:
/// `[0,0,0, 0, 1, 0, 0, 0, 0, 0, 1, 1]`.
pub(crate) fn compose_minimal_intra_two_coeff_block_trace_with_ist() -> Result<Vec<BlockSymbolToken>>
{
    let base = compose_minimal_intra_two_coeff_block_trace_with_tx_type()?;
    // `sec_tx_type` is read right after `intra_tx_type`; derive the insertion point
    // from the `intra_tx_type` token kind, falling back to just after `eob_pt_16`.
    let split = base
        .iter()
        .position(|token| {
            matches!(token, BlockSymbolToken::Coeff(coeff)
                if matches!(coeff.syntax(), CoefficientTokenSyntax::IntraTxType))
        })
        .unwrap_or(EOB_PT_16_TRACE_INDEX + 1)
        + 1;
    let total = base
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient IST block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient IST block trace",
        })?;
    trace.extend_from_slice(&base[..split]);
    trace.push(BlockSymbolToken::Coeff(sec_tx_type_intra_token(
        SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4,
        SEC_TX_TYPE_IST_OFF_SYMBOL,
    )));
    trace.extend_from_slice(&base[split..]);
    Ok(trace)
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

/// Encodes an ordered block-symbol trace into AV2 § 8.2 entropy-coded bytes — the
/// encoder's production entropy-coding entry point. Each token is written to its
/// scoped default CDF row (a bypass literal writes its raw bits); `finish()`
/// terminates the § 8.2 stream (§ 8.2.4 padding) and yields the coded bytes that a
/// § 5.20.1 `tile_group_payload()` carries as a single tile's data.
///
/// This drives the same § 8.2 encode path as [`roundtrip_block_symbol_trace`] (which
/// calls it), but returns the coded bytes for downstream tile-group assembly rather
/// than re-decoding them. It does not assemble a tile-group payload, OBU, frame, or
/// packet — those are later bricks.
pub(crate) fn encode_block_symbol_trace(trace: &[BlockSymbolToken]) -> Result<Vec<u8>> {
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
    Ok(output.into_bytes())
}

/// Proves a block-symbol trace through one § 8.2 coder: encodes it via
/// [`encode_block_symbol_trace`], then decodes the bytes back through one symbol
/// decoder with the same shared CDF state, verifying every token reproduces. Returns
/// the coded bytes and the decoded symbols for assertions.
pub(crate) fn roundtrip_block_symbol_trace(
    trace: &[BlockSymbolToken],
) -> Result<BlockSymbolTraceRoundtrip> {
    let bytes = encode_block_symbol_trace(trace)?;

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
    do_split_root: [i32; DO_SPLIT_CDF_ROW_LEN],
    y_mode_set: [i32; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [i32; INTRA_MODE_CDF_ROW_LEN],
    uv_mode_non_directional: [i32; INTRA_MODE_CDF_ROW_LEN],
    luma_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip: [i32; V_TXB_SKIP_CDF_ROW_LEN],
    luma_txb_skip_64x64: [i32; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip_32x32: [i32; TXB_SKIP_CDF_ROW_LEN],
    eob_pt_16: [i32; EOB_PT_16_CDF_ROW_LEN],
    eob_pt_1024: [i32; EOB_PT_1024_CDF_ROW_LEN],
    eob_pt_1024_chroma: [i32; EOB_PT_1024_CDF_ROW_LEN],
    coeff_base_lf_eob_tx64: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    intra_tx_type_set1_4x4: [i32; INTRA_TX_TYPE_SET1_CDF_ROW_LEN],
    sec_tx_type_intra_4x4: [i32; SEC_TX_TYPE_INTRA_CDF_ROW_LEN],
    coeff_base_lf_eob: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_eob_ac: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_dc: [i32; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_br_lf: [i32; COEFF_BR_LF_CDF_ROW_LEN],
    dc_sign: [i32; DC_SIGN_CDF_ROW_LEN],
    v_txb_skip_eobu: [i32; V_TXB_SKIP_CDF_ROW_LEN],
    chroma_eob_pt_16: [i32; EOB_PT_16_CDF_ROW_LEN],
    coeff_base_lf_eob_uv: [i32; COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN],
}

impl BlockSymbolTraceCdfRows {
    fn from_defaults() -> Self {
        Self {
            do_split_root: DEFAULT_DO_SPLIT_CDF[ROOT_PARTITION_PLANE_START]
                [ROOT_64X64_DO_SPLIT_CTX],
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index_tile_origin: DEFAULT_Y_MODE_INDEX_CDF[TILE_ORIGIN_Y_MODE_INDEX_CTX],
            uv_mode_non_directional: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
                [NON_DIRECTIONAL_UV_MODE_CTX],
            luma_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][V_TXB_SKIP_CTX_NEUTRAL],
            luma_txb_skip_64x64: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_64X64_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip_32x32: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_32X32_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_1024: DEFAULT_EOB_PT_1024_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_1024_chroma: DEFAULT_EOB_PT_1024_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            coeff_base_lf_eob_tx64: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            intra_tx_type_set1_4x4: DEFAULT_INTRA_TX_TYPE_SET1_CDF
                [INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4],
            sec_tx_type_intra_4x4: DEFAULT_SEC_TX_TYPE_CDF[SEC_TX_TYPE_INTRA_BANK]
                [SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4],
            coeff_base_lf_eob: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            coeff_base_lf_eob_ac: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
            coeff_base_lf_dc: DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX][TX_SIZE_4X4_CTX]
                [COEFF_BASE_LF_CTX_EOB2_DC][COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX][COEFF_BR_LF_CTX_DC],
            dc_sign: DEFAULT_DC_SIGN_CDF[MINIMAL_COEFF_CDF_Q_CTX][DC_SIGN_PLANE_TYPE_LUMA]
                [DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
            v_txb_skip_eobu: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [CHROMA_V_TXB_SKIP_CTX_EOBU],
            chroma_eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [COEFF_BASE_LF_EOB_CTX_DC],
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
            BlockSymbolToken::Partition(partition) => match partition.selector() {
                PartitionCdfRowSelector::DoSplit {
                    plane_start: ROOT_PARTITION_PLANE_START,
                    ctx: ROOT_64X64_DO_SPLIT_CTX,
                } => Ok(self.do_split_root.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
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
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.luma_txb_skip_64x64.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_32X32_CTX,
                    ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.u_txb_skip_32x32.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt16 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_16.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt1024 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_1024.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt1024 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_CHROMA,
                } => Ok(self.eob_pt_1024_chroma.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob_tx64.as_mut_slice()),
                CoefficientCdfRowSelector::IntraTxTypeSet1 {
                    tx_size_sqr: INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4,
                } => Ok(self.intra_tx_type_set1_4x4.as_mut_slice()),
                CoefficientCdfRowSelector::SecTxTypeIntra {
                    tx_size_sqr: SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4,
                } => Ok(self.sec_tx_type_intra_4x4.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                } => Ok(self.coeff_base_lf_eob_ac.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: COEFF_BASE_LF_CTX_EOB2_DC,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } => Ok(self.coeff_base_lf_dc.as_mut_slice()),
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
                CoefficientCdfRowSelector::VTxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: CHROMA_V_TXB_SKIP_CTX_EOBU,
                } => Ok(self.v_txb_skip_eobu.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt16 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_CHROMA,
                } => Ok(self.chroma_eob_pt_16.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEobUv {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob_uv.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "block_symbol_trace_tests.rs"]
mod tests;
