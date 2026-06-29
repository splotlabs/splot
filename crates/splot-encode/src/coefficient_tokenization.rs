// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder coefficient-tokenization foundation.
//!
//! This module advances `ENC-COEFFICIENT-TOKENIZATION-MINIMAL`. It converts the
//! current private 4x4 DCT_DCT DC-only quantized coefficient block into ordered
//! AV2 § 5.20.7.27 / § 5.20.7.28 token facts and proves those facts can
//! roundtrip through `splot-core`'s AV2 § 8.2 symbol encoder/decoder. It also
//! hosts the multi-coefficient building blocks consumed by `block_symbol_trace`:
//! `ENC-COEFF-BASE-LF-CONTEXT` (the § 8.3.2 `coeff_base` low-frequency luma context
//! derivation) and `ENC-COEFF-BASE-LF-TOKEN` (the non-EOB `coeff_base`
//! low-frequency luma token and its `TileCoeffBaseLfCdf` row).
//!
//! It does not emit tile payloads, own tile CDF lifecycle, write packets, expose
//! a public encoder API, or implement coefficient base-range / `read_quant`
//! extension syntax beyond the declared minimal base-symbol tier.
//! The current spatial-context subset is the top-left neutral luma block only:
//! neighbor-derived `all_zero` / `dc_sign` contexts remain future tile-state work.

#![allow(dead_code)]

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF, DEFAULT_COEFF_BASE_LF_CDF,
    DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF, DEFAULT_COEFF_BR_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF,
    DEFAULT_EOB_PT_256_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF, DEFAULT_SEC_TX_TYPE_CDF,
    DEFAULT_TXB_SKIP_CDF,
};
use splot_recon::{PlaneId, PlaneRect, TransformClass, coefficient_scan_order};

use crate::error::{Error, Result};
use crate::quantization::QuantizedTransformBlock;

mod general_walk_geom;

mod coeff_base_lf;
#[allow(unused_imports)]
pub(crate) use coeff_base_lf::{
    coeff_base_hf_luma_context, coeff_base_lf_luma_context, coeff_base_lf_token,
    coeff_base_lf_token_sized, coeff_br_lf_luma_context,
};

mod multi_coeff;
#[allow(unused_imports)]
pub(crate) use multi_coeff::{
    coded_luma_all_zero_token, coeff_base_hf_eob_token, coeff_base_hf_eob_token_sized,
    coeff_base_hf_token, coeff_base_hf_token_sized, coeff_base_lf_eob_token,
    coeff_base_lf_eob_token_sized, coeff_br_hf_token, coeff_br_lf_token, eob_extra_token,
    eob_pt_16_token, eob_pt_token,
};

mod transform_type;
#[allow(unused_imports)]
pub(crate) use transform_type::{intra_tx_type_set1_token, sec_tx_type_intra_token};

mod general_coded;
#[allow(unused_imports)]
pub(crate) use general_coded::{
    general_intra_16x16_luma_dc_coded_tokens, general_intra_32x32_chroma_u_dc_coded_tokens,
    general_intra_32x32_chroma_v_dc_coded_tokens, general_intra_64x64_luma_2d_base_tokens,
    general_intra_64x64_luma_dc_coded_tokens, general_intra_64x64_luma_eob3_base_tokens,
    general_intra_64x64_luma_two_coeff_tokens, general_intra_64x64_luma_two_nonzero_base_tokens,
    general_intra_64x64_luma_visible_ac_tokens,
};

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const LUMA_PLANE_TYPE: usize = 0;
const INTRA_NON_FSC_TXB_SKIP_BANK: usize = 0;
const TX_SIZE_4X4_CTX: usize = 0;
const TX_SIZE_64X64_CTX: usize = 4;
const TX_SIZE_32X32_CTX: usize = 3;
const TX_SIZE_16X16_CTX: usize = 2;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_U_TXB_SKIP_CTX_NEUTRAL: usize = 6;
const EOB_CTX_LUMA_INTRA: usize = 0;
const EOB_CTX_CHROMA: usize = 2;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const COEFF_BASE_LF_EOB_CTX_EOB2_AC: usize = 1;
const COEFF_BR_LF_CTX_DC: usize = 0;
const COEFF_BR_LF_CTX_EOB_AC: usize = 7;
const COEFF_BR_LF_CTX_DC_BR_AC_LOW: usize = 1;
const COEFF_BR_LF_CTX_DC_BR_AC_MID: usize = 2;
const COEFF_BR_LF_CTX_DC_BR_AC_HIGH: usize = 3;
const COEFF_BASE_LF_CTX_EOB2_DC: usize = 1;
const COEFF_BASE_LF_CTX_EOB2_DC_BR: usize = 3;
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;
const COEFF_BASE_LF_CDF_ROW_LEN: usize = 7;
const COEFF_BASE_LF_CTX_COUNT: usize = 33;
const COEFF_BASE_LF_EOB_CTX_COUNT: usize = 4;
const COEFF_BR_LF_CTX_COUNT: usize = 14;
const COEFF_BASE_LF_EOB_CDF_ROW_LEN: usize = 6;
const COEFF_BR_LF_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_EOB_CTX_COUNT: usize = 4;
const COEFF_BR_CTX_COUNT: usize = 7;
const COEFF_BASE_EOB_CDF_ROW_LEN: usize = 4;
const COEFF_BR_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_CTX_COUNT: usize = 20;
const COEFF_BASE_CDF_ROW_LEN: usize = 5;
const INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4: usize = 0;
const INTRA_TX_TYPE_SET1_TX_SIZE_SQR_COUNT: usize = 3;
const INTRA_TX_TYPE_SET1_CDF_ROW_LEN: usize = 8;
const SEC_TX_TYPE_INTRA_BANK: usize = 0;
const SEC_TX_TYPE_TX_SIZE_SQR_4X4: usize = 0;
const SEC_TX_TYPE_TX_SIZE_SQR_COUNT: usize = 5;
const SEC_TX_TYPE_CDF_ROW_LEN: usize = 5;
const DC_SIGN_GROUP_VISIBLE: usize = 0;
const DC_SIGN_CTX_NEUTRAL: usize = 0;
const MAX_BASE_EOB_MAGNITUDE: u32 = 4;
const MAX_BASE_BR_MAGNITUDE: u32 = LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE;
const COEFF_CDF_Q_CTX_0_MAX_QINDEX: u32 = 90;
const COEFF_CDF_Q_CTX_1_MAX_QINDEX: u32 = 140;
const COEFF_CDF_Q_CTX_2_MAX_QINDEX: u32 = 190;
const NEUTRAL_SPATIAL_CONTEXT_ORIGIN: usize = 0;

/// Tokenizes the current 4x4 DCT_DCT DC-only quantized coefficient subset.
pub(crate) fn tokenize_quantized_4x4_dct_dct_dc_only(
    block: &QuantizedTransformBlock,
) -> Result<CoefficientTokenizationPlan> {
    tokenize_coefficients(CoefficientTokenizationInput::from_quantized(block))
}

/// Returns the AV2 § 5.20.7.27 luma `all_zero` (`txb_skip`) token for an
/// all-zero transform block at the given coefficient CDF q-context.
///
/// This is the first `residual()` symbol of an all-zero luma block; no further
/// luma coefficient symbols follow it.
pub(crate) const fn luma_all_zero_token(coeff_cdf_q_ctx: usize) -> CoefficientEntropyToken {
    luma_all_zero_token_sized(coeff_cdf_q_ctx, TX_SIZE_4X4_CTX)
}

/// Returns the AV2 § 5.20.7.27 luma `all_zero` (`txb_skip == 1`) token at the neutral
/// spatial context for a transform of the given § 8.3.2 `txSzCtx` (`tx_size`). The
/// `all_zero` path of the size-generic general walk uses this so a 16x16 all-zero block
/// routes its `txb_skip` through the `TX_16X16` row, not the `TX_4X4` row (the coded path
/// already routes through `geom.tx_size_ctx`). The CDF row is keyed by
/// `(q, plane, tx_size, ctx)` — symbol-agnostic — so this shares the same row as the
/// coded `txb_skip == 0` at the same size.
pub(crate) const fn luma_all_zero_token_sized(
    coeff_cdf_q_ctx: usize,
    tx_size: usize,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: true as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 U-plane `all_zero` (`txb_skip`) token for an
/// all-zero chroma U transform block at the neutral spatial context.
///
/// Per AV2 § 8.3.2 the U `txb_skip` reuses `TileTxbSkipCdf` at the same
/// `is_inter || fsc_mode` bank as luma (0 for this intra non-FSC block) and is
/// distinguished only by `ctx`: the § 8.3.2 neutral context `6` (above/left
/// reductions are 0, plus the fixed `+6` for the U plane).
pub(crate) const fn chroma_u_all_zero_token(coeff_cdf_q_ctx: usize) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: true as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 V-plane `all_zero` (`txb_skip`) token for an
/// all-zero chroma V transform block at the given V `txb_skip` context.
///
/// The V `txb_skip` uses the dedicated `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]`.
/// For a block whose chroma block size equals its transform size with an
/// all-zero U plane, the § 8.3.2 context is 0 (no chroma-larger-than-tx or
/// `EobU != 0` contributions).
pub(crate) const fn chroma_v_all_zero_token(
    coeff_cdf_q_ctx: usize,
    ctx: usize,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::VTxbSkip {
            coeff_cdf_q_ctx,
            ctx,
        },
        symbol: true as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 luma `all_zero` (`txb_skip`) token for an all-zero
/// `TX_64X64` luma transform block on the general intra decode path, at the given
/// coefficient CDF q-context.
///
/// This is the general-path counterpart of [`luma_all_zero_token`]: identical
/// except for the `TX_64X64` `txSzCtx` (`4`) of the single 64x64 transform that
/// fills a 64x64 superblock leaf, rather than the minimal-tier `TX_4X4` ctx. The
/// luma `ctx` is `0` because the transform fills the block (§ 8.3.2
/// `txb_skip_ctx_luma` with `tx_fills_block`).
pub(crate) const fn general_intra_64x64_luma_all_zero_token(
    coeff_cdf_q_ctx: usize,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size: TX_SIZE_64X64_CTX,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: true as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 U-plane `all_zero` (`txb_skip`) token for an
/// all-zero `TX_32X32` chroma U transform block on the general intra decode path,
/// at the given coefficient CDF q-context.
///
/// The general-path counterpart of [`chroma_u_all_zero_token`]: identical except
/// for the `TX_32X32` `txSzCtx` (`3`) of the 32x32 chroma transform of a 64x64
/// 4:2:0 block. The `ctx` is the § 8.3.2 neutral U context `6` (above/left
/// reductions `0`, plus the fixed `+6` for the U plane).
pub(crate) const fn general_intra_32x32_chroma_u_all_zero_token(
    coeff_cdf_q_ctx: usize,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
            tx_size: TX_SIZE_32X32_CTX,
            ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: true as u8,
    }
}

/// Returns the ordered AV2 § 5.20.7.27 coded luma DC-only coefficient tokens for a
/// single nonzero DC coefficient of unsigned `magnitude` (`1..=MAX_BASE_BR_MAGNITUDE`)
/// and the given sign, at the neutral top-left luma context:
///
/// 1. `all_zero == 0` (`txb_skip`, the block is coded),
/// 2. `eob_pt_16 == 0` (EOB point 0: a single coefficient at scan position 0),
/// 3. `coeff_base_eob == min(magnitude, LF_NUM_BASE_LEVELS + 1) - 1` (the
///    low-frequency EOB base level; its level is `coeff_base_eob + 1`),
/// 4. `coeff_br == magnitude - (LF_NUM_BASE_LEVELS + 1)` *only when*
///    `magnitude > LF_NUM_BASE_LEVELS` (i.e. the base level reached its max 5 and
///    the base-range extension applies), and
/// 5. `dc_sign` (0 positive, 1 negative).
///
/// This is the single source of the coded DC token shape: `tokenize_coefficients`
/// delegates to it, and the `coded_dc_tokens_match_tokenizer` test asserts that.
/// The caller guarantees `1 <= magnitude <= MAX_BASE_BR_MAGNITUDE`
/// (`tokenize_coefficients` validates it); the arithmetic is saturating so the
/// function is total for any input.
pub(crate) fn luma_dc_coded_tokens(
    coeff_cdf_q_ctx: usize,
    magnitude: u32,
    negative: bool,
) -> Result<Vec<CoefficientEntropyToken>> {
    debug_assert!(magnitude >= 1, "coded DC magnitude must be nonzero");
    let needs_br = magnitude > LF_NUM_BASE_LEVELS;
    let len = if needs_br { 5 } else { 4 };
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(len)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "coded DC coefficient tokens",
        })?;
    tokens.push(all_zero_token(coeff_cdf_q_ctx, false));
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt16,
        selector: CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: magnitude.min(LF_NUM_BASE_LEVELS + 1).saturating_sub(1) as u8,
    });
    if needs_br {
        tokens.push(CoefficientEntropyToken {
            syntax: CoefficientTokenSyntax::CoeffBr,
            selector: CoefficientCdfRowSelector::CoeffBrLf {
                coeff_cdf_q_ctx,
                ctx: COEFF_BR_LF_CTX_DC,
            },
            symbol: magnitude.saturating_sub(LF_NUM_BASE_LEVELS + 1) as u8,
        });
    }
    tokens.push(luma_dc_sign_token(coeff_cdf_q_ctx, negative));
    Ok(tokens)
}

/// Returns the AV2 § 5.20.7.27 luma DC `dc_sign` CDF token (`TileDcSignCdf` at the
/// neutral luma plane-type/group/ctx). The luma DC sign is CDF-coded (unlike a
/// chroma or non-axis luma sign, which is a `sign_bit` bypass literal).
pub(crate) const fn luma_dc_sign_token(
    coeff_cdf_q_ctx: usize,
    negative: bool,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::DcSign,
        selector: CoefficientCdfRowSelector::DcSign {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            group: DC_SIGN_GROUP_VISIBLE,
            ctx: DC_SIGN_CTX_NEUTRAL,
        },
        symbol: negative as u8,
    }
}

/// Returns the four fixed AV2 § 5.20.7.27 luma DC *level* CDF tokens for the
/// golomb (`read_quant`) tier — `all_zero == 0`, `eob_pt_16 == 0`,
/// `coeff_base_eob == LF_NUM_BASE_LEVELS` (level 5, its maximum), and `coeff_br ==
/// COEFF_BASE_RANGE` (so the level reaches `maxLevel = LF_NUM_BASE_LEVELS +
/// COEFF_BASE_RANGE + 1 = 8`). These precede the § 5.20.7.28 golomb `coeff_rem`
/// bypass bits (the caller appends them) and the `dc_sign` CDF token; the golomb
/// extension `x = magnitude - maxLevel` is encoded by the bypass bits, not here.
pub(crate) fn luma_dc_golomb_level_tokens(
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(4)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "golomb-tier luma DC level tokens",
        })?;
    tokens.push(all_zero_token(coeff_cdf_q_ctx, false));
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt16,
        selector: CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: LF_NUM_BASE_LEVELS as u8,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBr,
        selector: CoefficientCdfRowSelector::CoeffBrLf {
            coeff_cdf_q_ctx,
            ctx: COEFF_BR_LF_CTX_DC,
        },
        symbol: COEFF_BASE_RANGE as u8,
    });
    Ok(tokens)
}

/// Returns the ordered AV2 § 5.20.7.27 coded U-plane DC-only *CDF* coefficient
/// tokens for a single nonzero chroma U DC coefficient of unsigned `magnitude`
/// (`1..=MAX_BASE_EOB_MAGNITUDE`, the base tier), at the neutral top-left chroma
/// context:
///
/// 1. `all_zero == 0` (U `txb_skip`, the block is coded),
/// 2. `eob_pt_16 == 0` (a single coefficient at scan position 0), and
/// 3. `coeff_base_eob == magnitude - 1` via the chroma `TileCoeffBaseLfEobUvCdf`.
///
/// The coefficient's `sign_bit` is **not** included: per § 5.20.7.27 a chroma DC
/// sign is an `L(1)` bypass literal (`sign_bit`), not a CDF symbol — the caller
/// appends it as a bypass token. Per § 8.3.2 the chroma contexts differ from luma:
/// the eob context is `2` (`eobCtx = (plane > 0) ? 2 : is_inter`) and
/// `coeff_base_eob` uses the dedicated chroma low-frequency CDF (DC ctx `0`).
/// Verified against the decoder's `base_eob_selector` derivation.
pub(crate) fn chroma_u_dc_coded_coeff_tokens(
    coeff_cdf_q_ctx: usize,
    magnitude: u32,
) -> Result<Vec<CoefficientEntropyToken>> {
    if !(1..=MAX_BASE_EOB_MAGNITUDE).contains(&magnitude) {
        return Err(Error::CoefficientTokenizationUnsupportedChromaMagnitude {
            plane: PlaneId::U,
            magnitude,
            max_magnitude: MAX_BASE_EOB_MAGNITUDE,
        });
    }
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(3)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "coded chroma U DC coefficient tokens",
        })?;
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: false as u8,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt16,
        selector: CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_CHROMA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEobUv {
            coeff_cdf_q_ctx,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: (magnitude - 1) as u8,
    });
    Ok(tokens)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoefficientTokenizationInput<'a> {
    plane: PlaneId,
    block: PlaneRect,
    width: usize,
    height: usize,
    coeff_cdf_q_ctx: usize,
    coefficients: &'a [i32],
}

impl<'a> CoefficientTokenizationInput<'a> {
    const fn from_quantized(block: &'a QuantizedTransformBlock) -> Self {
        Self {
            plane: block.plane(),
            block: block.block(),
            width: DCT_DCT_4X4_WIDTH,
            height: DCT_DCT_4X4_HEIGHT,
            coeff_cdf_q_ctx: coeff_cdf_q_context(block.params().qindex()),
            coefficients: block.quantized(),
        }
    }
}

/// Tokenization result for one private encoder transform block.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CoefficientTokenizationPlan {
    plane: PlaneId,
    block: PlaneRect,
    scan: [u16; DCT_DCT_4X4_COEFF_COUNT],
    begin_position: usize,
    eob: usize,
    sign_magnitude: Option<CoefficientSignMagnitude>,
    tokens: Vec<CoefficientEntropyToken>,
}

impl CoefficientTokenizationPlan {
    /// Returns the source plane identity.
    pub(crate) const fn plane(&self) -> PlaneId {
        self.plane
    }

    /// Returns the visible-plane-relative transform block rectangle.
    pub(crate) const fn block(&self) -> PlaneRect {
        self.block
    }

    /// Returns the AV2 § 5.20.7.30 scan order used by this plan.
    pub(crate) const fn scan(&self) -> &[u16; DCT_DCT_4X4_COEFF_COUNT] {
        &self.scan
    }

    /// Returns the current ordinary-path begin scan position.
    pub(crate) const fn begin_position(&self) -> usize {
        self.begin_position
    }

    /// Returns the end-of-block value.
    pub(crate) const fn eob(&self) -> usize {
        self.eob
    }

    /// Returns DC sign/magnitude facts when the block is nonzero.
    pub(crate) const fn sign_magnitude(&self) -> Option<CoefficientSignMagnitude> {
        self.sign_magnitude
    }

    /// Returns ordered entropy-token records.
    pub(crate) fn tokens(&self) -> &[CoefficientEntropyToken] {
        &self.tokens
    }
}

/// Sign and magnitude facts for one tokenized nonzero coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoefficientSignMagnitude {
    scan_index: usize,
    coefficient_index: usize,
    row: usize,
    col: usize,
    magnitude: u32,
    negative: bool,
}

impl CoefficientSignMagnitude {
    /// Returns the coefficient scan index.
    pub(crate) const fn scan_index(self) -> usize {
        self.scan_index
    }

    /// Returns the row-major coefficient index.
    pub(crate) const fn coefficient_index(self) -> usize {
        self.coefficient_index
    }

    /// Returns the coefficient row.
    pub(crate) const fn row(self) -> usize {
        self.row
    }

    /// Returns the coefficient column.
    pub(crate) const fn col(self) -> usize {
        self.col
    }

    /// Returns the absolute coefficient magnitude.
    pub(crate) const fn magnitude(self) -> u32 {
        self.magnitude
    }

    /// Returns whether the coefficient is negative.
    pub(crate) const fn negative(self) -> bool {
        self.negative
    }
}

/// AV2 entropy-token syntax covered by the current private subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoefficientTokenSyntax {
    /// `all_zero` in AV2 § 5.20.7.27.
    AllZero,
    /// `eob_pt_16` in AV2 § 5.20.7.27.
    EobPt16,
    /// `eob_pt_1024` in AV2 § 5.20.7.27 — the `eob_pt` symbol for the `TX_64X64`
    /// EOB-point size class (1024 scan positions), distinct from `eob_pt_16` only in
    /// which `TileEobPt*Cdf` bank it reads.
    EobPt1024,
    /// `eob_pt_256` in AV2 § 5.20.7.27 — the `eob_pt` symbol for the `TX_16X16`
    /// EOB-point size class (256 scan positions, `eobMultisize == 4`).
    EobPt256,
    /// `eob_extra` in AV2 § 5.20.7.27 — the CDF-coded first refinement bit read when
    /// `eob_pt >= 3`. (Any further refinement bits are `eob_extra_bit` § 8.2.5 bypass
    /// literals, modeled by [`CoefficientTokenSyntax`] callers via the bypass token kind.)
    EobExtra,
    /// `coeff_base_eob` in AV2 § 5.20.7.27.
    CoeffBaseEob,
    /// `coeff_base` (the non-EOB base level) in AV2 § 5.20.7.27.
    CoeffBase,
    /// `intra_tx_type` (the primary transform type) in AV2 § 5.20.8.2
    /// `transform_type()` (called from § 5.20.7.27 `coeffs()`).
    IntraTxType,
    /// `sec_tx_type` (the IST secondary transform) in AV2 § 5.20.8.2
    /// `transform_type()`, read right after `intra_tx_type`.
    SecTxType,
    /// `coeff_br` (base range) in AV2 § 5.20.7.27.
    CoeffBr,
    /// `dc_sign` in AV2 § 5.20.7.27.
    DcSign,
}

impl CoefficientTokenSyntax {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AllZero => "all_zero",
            Self::EobPt16 => "eob_pt_16",
            Self::EobPt1024 => "eob_pt_1024",
            Self::EobPt256 => "eob_pt_256",
            Self::EobExtra => "eob_extra",
            Self::CoeffBaseEob => "coeff_base_eob",
            Self::CoeffBase => "coeff_base",
            Self::IntraTxType => "intra_tx_type",
            Self::SecTxType => "sec_tx_type",
            Self::CoeffBr => "coeff_br",
            Self::DcSign => "dc_sign",
        }
    }
}

/// Scoped default-CDF selector for one token record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoefficientCdfRowSelector {
    /// `TileTxbSkipCdf[coeff_cdf_q_ctx][plane_type][tx_size][ctx]`.
    TxbSkip {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        tx_size: usize,
        ctx: usize,
    },
    /// `TileEobPt16Cdf[coeff_cdf_q_ctx][eob_ctx]`.
    EobPt16 {
        coeff_cdf_q_ctx: usize,
        eob_ctx: usize,
    },
    /// `TileEobPt1024Cdf[coeff_cdf_q_ctx][eob_ctx]` (the `TX_64X64` EOB-point size
    /// class).
    EobPt1024 {
        coeff_cdf_q_ctx: usize,
        eob_ctx: usize,
    },
    /// `TileEobPt256Cdf[coeff_cdf_q_ctx][eob_ctx]` (the `TX_16X16` EOB-point size
    /// class).
    EobPt256 {
        coeff_cdf_q_ctx: usize,
        eob_ctx: usize,
    },
    /// `TileEobExtraCdf[coeff_cdf_q_ctx]` (the `eob_extra` CDF; indexed only by the
    /// coefficient-CDF q-context — no `txSz`/`eobPt` dimension).
    EobExtra { coeff_cdf_q_ctx: usize },
    /// `TileCoeffBaseLfEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`.
    CoeffBaseLfEob {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
    },
    /// `TileCoeffBaseLfCdf[coeff_cdf_q_ctx][tx_size][ctx][tcq_ctx]` (the non-EOB
    /// low-frequency luma `coeff_base` CDF). `tcq_ctx = (tcqState >> 1) & 1` is 0
    /// when TCQ is off.
    CoeffBaseLf {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    /// `TileCoeffBaseCdf[coeff_cdf_q_ctx][tx_size][ctx][tcq_ctx]` (the non-EOB
    /// HIGH-frequency luma `coeff_base` CDF; AV2 § 8.3.2 `DEFAULT_COEFF_BASE_CDF`,
    /// dims `[q][tx_size][ctx][tcq][row]` with a 4-symbol row). Distinct from
    /// [`Self::CoeffBaseLf`]: the HF non-EOB base CDF is 4-symbol (vs the LF 6-symbol
    /// `DEFAULT_COEFF_BASE_LF_CDF`), and the HF `coeff_base` context
    /// (`coeff_base_hf_luma_context`) has no near-DC `magLimit = 5` carve-out and no
    /// DC band (see the decoder `CoeffBaseContext::select` `is_lf == false` branch in
    /// `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs:428-440`).
    /// `tcq_ctx = (tcqState >> 1) & 1` is 0 when TCQ is off.
    CoeffBase {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    /// `TileDcSignCdf[coeff_cdf_q_ctx][plane_type][group][ctx]`.
    DcSign {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        group: usize,
        ctx: usize,
    },
    /// `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]` (the V-plane `all_zero` CDF).
    VTxbSkip { coeff_cdf_q_ctx: usize, ctx: usize },
    /// `TileCoeffBrLfCdf[coeff_cdf_q_ctx][ctx]` (the low-frequency `coeff_br` CDF).
    CoeffBrLf { coeff_cdf_q_ctx: usize, ctx: usize },
    /// `TileCoeffBaseEobCdf[coeff_cdf_q_ctx][tx_size][ctx]` (the HIGH-frequency
    /// `coeff_base_eob` CDF; AV2 § 8.3.2 `DEFAULT_COEFF_BASE_EOB_CDF`). Distinct from
    /// [`Self::CoeffBaseLfEob`]: the HF EOB-position base CDF is 4-symbol (vs the LF
    /// 6-symbol `DEFAULT_COEFF_BASE_LF_EOB_CDF`). The `coeff_base_eob` *context*
    /// (`coeff_base_eob_ctx`) is LF/HF-independent and is shared with the LF path.
    CoeffBaseEob {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
    },
    /// `TileCoeffBrCdf[coeff_cdf_q_ctx][ctx]` (the HIGH-frequency base-range CDF; AV2
    /// § 8.3.2 `DEFAULT_COEFF_BR_CDF`, dims `[q][ctx][row]` with NO transform-size
    /// dimension). Distinct from [`Self::CoeffBrLf`]: the HF `coeff_br` context for a
    /// non-DC luma coefficient is plain `mag` (range `0..=6`) with NO `+7` offset (the
    /// LF non-DC branch adds `+7`; see the decoder `CoeffBrContext::ctx` `is_lf=false`
    /// branch in `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`).
    CoeffBr { coeff_cdf_q_ctx: usize, ctx: usize },
    /// `TileCoeffBaseLfEobUvCdf[coeff_cdf_q_ctx][ctx]` (the chroma low-frequency
    /// `coeff_base_eob` CDF).
    CoeffBaseLfEobUv { coeff_cdf_q_ctx: usize, ctx: usize },
    /// `TileIntraTxTypeSet1Cdf[tx_size_sqr]` (the `intra_tx_type` CDF for the
    /// `TX_SET_INTRA_1` transform set; the CDF has no coefficient-CDF q-context).
    IntraTxTypeSet1 { tx_size_sqr: usize },
    /// `TileSecTxTypeCdf[0][tx_size_sqr]` (the intra `sec_tx_type` IST CDF, `is_inter
    /// = 0`; the CDF has no coefficient-CDF q-context).
    SecTxTypeIntra { tx_size_sqr: usize },
}

impl CoefficientCdfRowSelector {
    const fn syntax_name(self) -> &'static str {
        match self {
            Self::TxbSkip { .. } | Self::VTxbSkip { .. } => {
                CoefficientTokenSyntax::AllZero.as_str()
            }
            Self::EobPt16 { .. } => CoefficientTokenSyntax::EobPt16.as_str(),
            Self::EobPt1024 { .. } => CoefficientTokenSyntax::EobPt1024.as_str(),
            Self::EobPt256 { .. } => CoefficientTokenSyntax::EobPt256.as_str(),
            Self::EobExtra { .. } => CoefficientTokenSyntax::EobExtra.as_str(),
            Self::CoeffBrLf { .. } | Self::CoeffBr { .. } => {
                CoefficientTokenSyntax::CoeffBr.as_str()
            }
            Self::CoeffBaseLf { .. } | Self::CoeffBase { .. } => {
                CoefficientTokenSyntax::CoeffBase.as_str()
            }
            Self::CoeffBaseEob { .. } => CoefficientTokenSyntax::CoeffBaseEob.as_str(),
            Self::IntraTxTypeSet1 { .. } => CoefficientTokenSyntax::IntraTxType.as_str(),
            Self::SecTxTypeIntra { .. } => CoefficientTokenSyntax::SecTxType.as_str(),
            Self::CoeffBaseLfEob { .. } | Self::CoeffBaseLfEobUv { .. } => {
                CoefficientTokenSyntax::CoeffBaseEob.as_str()
            }
            Self::DcSign { .. } => CoefficientTokenSyntax::DcSign.as_str(),
        }
    }
}

/// Ordered entropy-token record for the current private subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoefficientEntropyToken {
    syntax: CoefficientTokenSyntax,
    selector: CoefficientCdfRowSelector,
    symbol: u8,
}

impl CoefficientEntropyToken {
    /// Returns the token syntax.
    pub(crate) const fn syntax(self) -> CoefficientTokenSyntax {
        self.syntax
    }

    /// Returns the scoped CDF row selector.
    pub(crate) const fn selector(self) -> CoefficientCdfRowSelector {
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

fn tokenize_coefficients(
    input: CoefficientTokenizationInput<'_>,
) -> Result<CoefficientTokenizationPlan> {
    validate_input(input)?;
    let mut scan = [0u16; DCT_DCT_4X4_COEFF_COUNT];
    coefficient_scan_order(
        DCT_DCT_4X4_WIDTH,
        DCT_DCT_4X4_HEIGHT,
        TransformClass::TwoD,
        &mut scan,
    )
    .map_err(|source| Error::CoefficientTokenizationScan {
        plane: input.plane,
        block: input.block,
        source,
    })?;

    if let Some((index, value)) = first_non_dc(input.coefficients) {
        return Err(Error::CoefficientTokenizationNonDcCoefficient {
            plane: input.plane,
            block: input.block,
            coefficient_index: index,
            value,
        });
    }

    let dc = input.coefficients[0];
    if dc == 0 {
        let mut tokens = Vec::new();
        tokens.try_reserve_exact(1).map_err(|_| {
            Error::CoefficientTokenizationAllocationFailed {
                context: "all-zero coefficient tokens",
            }
        })?;
        tokens.push(all_zero_token(input.coeff_cdf_q_ctx, true));
        return Ok(CoefficientTokenizationPlan {
            plane: input.plane,
            block: input.block,
            scan,
            begin_position: 0,
            eob: 0,
            sign_magnitude: None,
            tokens,
        });
    }

    let magnitude = dc.unsigned_abs();
    if magnitude > MAX_BASE_BR_MAGNITUDE {
        return Err(Error::CoefficientTokenizationUnsupportedMagnitude {
            plane: input.plane,
            block: input.block,
            coefficient_index: 0,
            magnitude,
            max_magnitude: MAX_BASE_BR_MAGNITUDE,
        });
    }

    let tokens = luma_dc_coded_tokens(input.coeff_cdf_q_ctx, magnitude, dc < 0)?;

    Ok(CoefficientTokenizationPlan {
        plane: input.plane,
        block: input.block,
        scan,
        begin_position: 0,
        eob: 1,
        sign_magnitude: Some(CoefficientSignMagnitude {
            scan_index: 0,
            coefficient_index: 0,
            row: 0,
            col: 0,
            magnitude,
            negative: dc < 0,
        }),
        tokens,
    })
}

fn validate_input(input: CoefficientTokenizationInput<'_>) -> Result<()> {
    if input.plane != PlaneId::Y {
        return Err(Error::CoefficientTokenizationUnsupportedPlane { plane: input.plane });
    }
    if input.width != DCT_DCT_4X4_WIDTH
        || input.height != DCT_DCT_4X4_HEIGHT
        || input.block.width() != DCT_DCT_4X4_WIDTH
        || input.block.height() != DCT_DCT_4X4_HEIGHT
    {
        return Err(Error::CoefficientTokenizationUnsupportedShape {
            plane: input.plane,
            block: input.block,
            expected_width: DCT_DCT_4X4_WIDTH,
            expected_height: DCT_DCT_4X4_HEIGHT,
        });
    }
    if input.coefficients.len() != DCT_DCT_4X4_COEFF_COUNT {
        return Err(Error::CoefficientTokenizationInputLengthMismatch {
            plane: input.plane,
            block: input.block,
            expected: DCT_DCT_4X4_COEFF_COUNT,
            actual: input.coefficients.len(),
        });
    }
    if input.block.x() != NEUTRAL_SPATIAL_CONTEXT_ORIGIN
        || input.block.y() != NEUTRAL_SPATIAL_CONTEXT_ORIGIN
    {
        return Err(Error::CoefficientTokenizationUnsupportedSpatialContext {
            plane: input.plane,
            block: input.block,
        });
    }
    Ok(())
}

fn first_non_dc(coefficients: &[i32]) -> Option<(usize, i32)> {
    coefficients
        .iter()
        .copied()
        .enumerate()
        .skip(1)
        .find(|(_, value)| *value != 0)
}

const fn all_zero_token(coeff_cdf_q_ctx: usize, all_zero: bool) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: all_zero as u8,
    }
}

/// Returns the AV2 § 5.20.7.27 luma `all_zero == 0` (`txb_skip`) token for a *coded*
/// block at the given `tx_size` `txSzCtx` and the neutral luma context. The general
/// walk uses this for a non-4x4 block (e.g. the 16x16 base pass) so its `txb_skip`
/// routes the size's `TileTxbSkipCdf[q][luma][txSz][ctx]` row.
pub(crate) const fn coded_luma_all_zero_token_sized(
    coeff_cdf_q_ctx: usize,
    tx_size: usize,
) -> CoefficientEntropyToken {
    CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::AllZero,
        selector: CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            tx_size,
            ctx: TXB_SKIP_CTX_NEUTRAL,
        },
        symbol: false as u8,
    }
}

const fn coeff_cdf_q_context(qindex: u32) -> usize {
    if qindex <= COEFF_CDF_Q_CTX_0_MAX_QINDEX {
        0
    } else if qindex <= COEFF_CDF_Q_CTX_1_MAX_QINDEX {
        1
    } else if qindex <= COEFF_CDF_Q_CTX_2_MAX_QINDEX {
        2
    } else {
        3
    }
}

mod cdf_rows;

mod entropy_proof;
#[allow(unused_imports)]
pub(crate) use entropy_proof::roundtrip_entropy_tokens;

mod general_walk;
#[allow(unused_imports)]
pub(crate) use general_walk::tokenize_general_lf_luma_block;

mod general_walk_16x16;
#[allow(unused_imports)]
pub(crate) use general_walk_16x16::tokenize_general_16x16_luma_block;
#[allow(unused_imports)]
pub(crate) use general_walk_16x16::tokenize_general_16x16_luma_block_full;

mod general_walk_golomb;

mod general_walk_recover;
#[allow(unused_imports)]
pub(crate) use general_walk_recover::{recover_quant_from_tokens, recover_quant_from_tokens_geom};

#[cfg(test)]
#[path = "coefficient_tokenization_tests.rs"]
mod tests;
