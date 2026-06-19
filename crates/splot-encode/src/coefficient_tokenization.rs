// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder coefficient-tokenization foundation.
//!
//! This module advances `ENC-COEFFICIENT-TOKENIZATION-MINIMAL`. It converts the
//! current private 4x4 DCT_DCT DC-only quantized coefficient block into ordered
//! AV2 § 5.20.7.27 / § 5.20.7.28 token facts and proves those facts can
//! roundtrip through `splot-core`'s AV2 § 8.2 symbol encoder/decoder.
//!
//! It does not emit tile payloads, own tile CDF lifecycle, write packets, expose
//! a public encoder API, or implement coefficient base-range / `read_quant`
//! extension syntax beyond the declared minimal base-symbol tier.
//! The current spatial-context subset is the top-left neutral luma block only:
//! neighbor-derived `all_zero` / `dc_sign` contexts remain future tile-state work.

#![allow(dead_code)]

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_EOB_PT_16_CDF, DEFAULT_TXB_SKIP_CDF,
};
use splot_recon::{PlaneId, PlaneRect, TransformClass, coefficient_scan_order};

use crate::error::{Error, Result};
use crate::quantization::QuantizedTransformBlock;

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const LUMA_PLANE_TYPE: usize = 0;
const TX_SIZE_4X4_CTX: usize = 0;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const EOB_CTX_LUMA_INTRA: usize = 0;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const DC_SIGN_GROUP_VISIBLE: usize = 0;
const DC_SIGN_CTX_NEUTRAL: usize = 0;
const MAX_BASE_EOB_MAGNITUDE: u32 = 4;
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
    all_zero_token(coeff_cdf_q_ctx, true)
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
    /// `coeff_base_eob` in AV2 § 5.20.7.27.
    CoeffBaseEob,
    /// `dc_sign` in AV2 § 5.20.7.27.
    DcSign,
}

impl CoefficientTokenSyntax {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AllZero => "all_zero",
            Self::EobPt16 => "eob_pt_16",
            Self::CoeffBaseEob => "coeff_base_eob",
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
    /// `TileCoeffBaseLfEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`.
    CoeffBaseLfEob {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
    },
    /// `TileDcSignCdf[coeff_cdf_q_ctx][plane_type][group][ctx]`.
    DcSign {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        group: usize,
        ctx: usize,
    },
}

impl CoefficientCdfRowSelector {
    const fn syntax_name(self) -> &'static str {
        match self {
            Self::TxbSkip { .. } => CoefficientTokenSyntax::AllZero.as_str(),
            Self::EobPt16 { .. } => CoefficientTokenSyntax::EobPt16.as_str(),
            Self::CoeffBaseLfEob { .. } => CoefficientTokenSyntax::CoeffBaseEob.as_str(),
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

/// Result of proving token values through AV2 § 8.2 symbol bytes.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CoefficientTokenRoundtrip {
    bytes: Vec<u8>,
    decoded_symbols: Vec<u8>,
    symbol_count: u64,
}

impl CoefficientTokenRoundtrip {
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

/// Writes token records with the § 8.2 symbol encoder and decodes them back.
pub(crate) fn roundtrip_entropy_tokens(
    tokens: &[CoefficientEntropyToken],
) -> Result<CoefficientTokenRoundtrip> {
    let mut encode_cdfs = CoefficientTokenCdfRows::from_defaults();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new()
            .with_max_output_bytes(64)
            .with_max_operations(64),
    );
    for token in tokens.iter().copied() {
        encoder
            .write_symbol(
                encode_cdfs.row_mut(token.selector())?,
                Symbol::new(token.symbol()),
            )
            .map_err(|source| Error::CoefficientTokenizationSymbolWrite {
                syntax: token.syntax_name(),
                source,
            })?;
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::CoefficientTokenizationSymbolEncodeFinish { source })?;
    let bytes = output.into_bytes();

    let mut decode_cdfs = CoefficientTokenCdfRows::from_defaults();
    let mut decoder = SymbolDecoder::with_config(&bytes, SymbolDecoderConfig::new())
        .map_err(|source| Error::CoefficientTokenizationSymbolDecodeInit { source })?;
    let mut decoded_symbols = Vec::new();
    decoded_symbols
        .try_reserve_exact(tokens.len())
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "roundtrip decoded symbols",
        })?;
    for token in tokens.iter().copied() {
        let decoded = decoder
            .read_symbol(decode_cdfs.row_mut(token.selector())?)
            .map_err(|source| Error::CoefficientTokenizationSymbolRead {
                syntax: token.syntax_name(),
                source,
            })?
            .get();
        if decoded != token.symbol() {
            return Err(Error::CoefficientTokenizationSymbolMismatch {
                syntax: token.syntax_name(),
                expected: token.symbol(),
                actual: decoded,
            });
        }
        decoded_symbols.push(decoded);
    }
    let summary = decoder
        .finish()
        .map_err(|source| Error::CoefficientTokenizationSymbolDecodeFinish { source })?;

    Ok(CoefficientTokenRoundtrip {
        bytes,
        decoded_symbols,
        symbol_count: summary.symbol_count,
    })
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
    if magnitude > MAX_BASE_EOB_MAGNITUDE {
        return Err(Error::CoefficientTokenizationUnsupportedMagnitude {
            plane: input.plane,
            block: input.block,
            coefficient_index: 0,
            magnitude,
            max_magnitude: MAX_BASE_EOB_MAGNITUDE,
        });
    }

    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(4)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "DC coefficient tokens",
        })?;
    tokens.push(all_zero_token(input.coeff_cdf_q_ctx, false));
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::EobPt16,
        selector: CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        },
        symbol: 0,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::CoeffBaseEob,
        selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
        },
        symbol: (magnitude - 1) as u8,
    });
    tokens.push(CoefficientEntropyToken {
        syntax: CoefficientTokenSyntax::DcSign,
        selector: CoefficientCdfRowSelector::DcSign {
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            plane_type: LUMA_PLANE_TYPE,
            group: DC_SIGN_GROUP_VISIBLE,
            ctx: DC_SIGN_CTX_NEUTRAL,
        },
        symbol: u8::from(dc < 0),
    });

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CoefficientTokenCdfRows {
    txb_skip: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    eob_pt_16: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    dc_sign: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
}

impl CoefficientTokenCdfRows {
    fn from_defaults() -> Self {
        Self {
            txb_skip: [
                DEFAULT_TXB_SKIP_CDF[0][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[1][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[2][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[3][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
            ],
            eob_pt_16: [
                DEFAULT_EOB_PT_16_CDF[0][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[1][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[2][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[3][EOB_CTX_LUMA_INTRA],
            ],
            coeff_base_lf_eob: [
                DEFAULT_COEFF_BASE_LF_EOB_CDF[0][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[1][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[2][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[3][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            ],
            dc_sign: [
                DEFAULT_DC_SIGN_CDF[0][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[1][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[2][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[3][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
            ],
        }
    }

    fn row_mut(&mut self, selector: CoefficientCdfRowSelector) -> Result<&mut [i32]> {
        match selector {
            CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: TXB_SKIP_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.txb_skip[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobPt16 {
                coeff_cdf_q_ctx,
                eob_ctx: EOB_CTX_LUMA_INTRA,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.eob_pt_16[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: COEFF_BASE_LF_EOB_CTX_DC,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_base_lf_eob[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                group: DC_SIGN_GROUP_VISIBLE,
                ctx: DC_SIGN_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.dc_sign[coeff_cdf_q_ctx].as_mut_slice())
            }
            selector => Err(Error::CoefficientTokenizationUnsupportedCdfSelector {
                syntax: selector.syntax_name(),
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::forward_transform::ForwardTransformBlock;
    use crate::quantization::{FixedQuantizationParams, QuantizedTransformBlock};
    use splot_recon::BitDepth as ReconBitDepth;

    const SCAN_4X4_2D: [u16; 16] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15];

    fn rect(width: usize, height: usize) -> PlaneRect {
        rect_at(0, 0, width, height)
    }

    fn rect_at(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn uniform(sample: i32) -> [i32; DCT_DCT_4X4_COEFF_COUNT] {
        [sample; DCT_DCT_4X4_COEFF_COUNT]
    }

    fn transform(sample: i32) -> ForwardTransformBlock {
        ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &uniform(sample))
            .unwrap()
    }

    fn quantized(sample: i32, qindex: u32) -> QuantizedTransformBlock {
        QuantizedTransformBlock::dct_dct_4x4_dc_only(
            &transform(sample),
            FixedQuantizationParams::new(ReconBitDepth::Eight, qindex).unwrap(),
        )
        .unwrap()
    }

    fn quantized_base_tier(sample: i32) -> QuantizedTransformBlock {
        (0..=255)
            .map(|qindex| quantized(sample, qindex))
            .find(|block| {
                let magnitude = block.quantized()[0].unsigned_abs();
                (1..=MAX_BASE_EOB_MAGNITUDE).contains(&magnitude)
            })
            .expect("base-tier qindex must exist for coefficient tokenization test sample")
    }

    fn raw_input<'a>(
        plane: PlaneId,
        block: PlaneRect,
        width: usize,
        height: usize,
        coefficients: &'a [i32],
    ) -> CoefficientTokenizationInput<'a> {
        CoefficientTokenizationInput {
            plane,
            block,
            width,
            height,
            coeff_cdf_q_ctx: 0,
            coefficients,
        }
    }

    #[test]
    fn derives_coeff_cdf_q_context_from_qindex() {
        assert_eq!(coeff_cdf_q_context(0), 0);
        assert_eq!(coeff_cdf_q_context(90), 0);
        assert_eq!(coeff_cdf_q_context(91), 1);
        assert_eq!(coeff_cdf_q_context(140), 1);
        assert_eq!(coeff_cdf_q_context(141), 2);
        assert_eq!(coeff_cdf_q_context(190), 2);
        assert_eq!(coeff_cdf_q_context(191), 3);
        assert_eq!(coeff_cdf_q_context(255), 3);
    }

    #[test]
    fn all_zero_block_emits_skip_token_only() {
        let block = quantized(0, 0);
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();

        assert_eq!(plan.plane(), PlaneId::Y);
        assert_eq!(plan.block(), rect(4, 4));
        assert_eq!(plan.scan(), &SCAN_4X4_2D);
        assert_eq!(plan.begin_position(), 0);
        assert_eq!(plan.eob(), 0);
        assert_eq!(plan.sign_magnitude(), None);
        assert_eq!(plan.tokens(), &[all_zero_token(0, true)]);
    }

    #[test]
    fn all_zero_block_uses_derived_q_context() {
        let block = quantized(0, 120);
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();

        assert_eq!(plan.tokens(), &[all_zero_token(1, true)]);
    }

    #[test]
    fn positive_dc_only_block_emits_ordered_base_tokens() {
        let block = quantized_base_tier(1);
        let magnitude = block.quantized()[0].unsigned_abs();
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();

        assert_eq!(plan.eob(), 1);
        assert_eq!(
            plan.sign_magnitude(),
            Some(CoefficientSignMagnitude {
                scan_index: 0,
                coefficient_index: 0,
                row: 0,
                col: 0,
                magnitude,
                negative: false,
            })
        );
        assert_eq!(
            plan.tokens(),
            &[
                all_zero_token(0, false),
                CoefficientEntropyToken {
                    syntax: CoefficientTokenSyntax::EobPt16,
                    selector: CoefficientCdfRowSelector::EobPt16 {
                        coeff_cdf_q_ctx: 0,
                        eob_ctx: EOB_CTX_LUMA_INTRA,
                    },
                    symbol: 0,
                },
                CoefficientEntropyToken {
                    syntax: CoefficientTokenSyntax::CoeffBaseEob,
                    selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
                        coeff_cdf_q_ctx: 0,
                        tx_size: TX_SIZE_4X4_CTX,
                        ctx: COEFF_BASE_LF_EOB_CTX_DC,
                    },
                    symbol: (magnitude - 1) as u8,
                },
                CoefficientEntropyToken {
                    syntax: CoefficientTokenSyntax::DcSign,
                    selector: CoefficientCdfRowSelector::DcSign {
                        coeff_cdf_q_ctx: 0,
                        plane_type: LUMA_PLANE_TYPE,
                        group: DC_SIGN_GROUP_VISIBLE,
                        ctx: DC_SIGN_CTX_NEUTRAL,
                    },
                    symbol: 0,
                },
            ]
        );
    }

    #[test]
    fn negative_dc_only_block_emits_negative_dc_sign() {
        let block = quantized_base_tier(-1);
        let magnitude = block.quantized()[0].unsigned_abs();
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();

        assert_eq!(
            plan.sign_magnitude(),
            Some(CoefficientSignMagnitude {
                scan_index: 0,
                coefficient_index: 0,
                row: 0,
                col: 0,
                magnitude,
                negative: true,
            })
        );
        assert_eq!(
            plan.tokens().last().copied(),
            Some(CoefficientEntropyToken {
                syntax: CoefficientTokenSyntax::DcSign,
                selector: CoefficientCdfRowSelector::DcSign {
                    coeff_cdf_q_ctx: 0,
                    plane_type: LUMA_PLANE_TYPE,
                    group: DC_SIGN_GROUP_VISIBLE,
                    ctx: DC_SIGN_CTX_NEUTRAL,
                },
                symbol: 1,
            })
        );
    }

    #[test]
    fn accepts_lf_base_tier_boundary_magnitude() {
        let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        coefficients[0] = MAX_BASE_EOB_MAGNITUDE as i32;
        let plan =
            tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients)).unwrap();
        let expected_symbol = (MAX_BASE_EOB_MAGNITUDE - 1) as u8;

        assert_eq!(
            plan.sign_magnitude(),
            Some(CoefficientSignMagnitude {
                scan_index: 0,
                coefficient_index: 0,
                row: 0,
                col: 0,
                magnitude: MAX_BASE_EOB_MAGNITUDE,
                negative: false,
            })
        );
        assert_eq!(
            plan.tokens()[2],
            CoefficientEntropyToken {
                syntax: CoefficientTokenSyntax::CoeffBaseEob,
                selector: CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: 0,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                },
                symbol: expected_symbol,
            }
        );

        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0, 0, expected_symbol, 0]);
    }

    #[test]
    fn all_zero_tokens_roundtrip_through_symbol_coder() {
        let block = quantized(0, 0);
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();
        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();

        assert_eq!(proof.decoded_symbols(), &[1]);
        assert_eq!(proof.symbol_count(), 1);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn dc_tokens_roundtrip_through_symbol_coder() {
        let block = quantized_base_tier(-1);
        let plan = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap();
        let expected: Vec<u8> = plan.tokens().iter().map(|token| token.symbol()).collect();
        let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();

        assert_eq!(proof.decoded_symbols(), expected.as_slice());
        assert_eq!(proof.symbol_count(), plan.tokens().len() as u64);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn rejects_non_luma_plane() {
        let coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        let err = tokenize_coefficients(raw_input(PlaneId::U, rect(4, 4), 4, 4, &coefficients))
            .unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationUnsupportedPlane { plane: PlaneId::U }
        ));
    }

    #[test]
    fn rejects_non_origin_spatial_context() {
        let coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        let err = tokenize_coefficients(raw_input(
            PlaneId::Y,
            rect_at(4, 0, 4, 4),
            4,
            4,
            &coefficients,
        ))
        .unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationUnsupportedSpatialContext {
                plane: PlaneId::Y,
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_4x4_shape() {
        let coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        let err = tokenize_coefficients(raw_input(PlaneId::Y, rect(2, 4), 2, 4, &coefficients))
            .unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationUnsupportedShape {
                plane: PlaneId::Y,
                expected_width: 4,
                expected_height: 4,
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_dc_coefficient() {
        let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        coefficients[4] = 1;
        let err = tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients))
            .unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationNonDcCoefficient {
                plane: PlaneId::Y,
                coefficient_index: 4,
                value: 1,
                ..
            }
        ));
    }

    #[test]
    fn rejects_magnitude_outside_base_symbol_tier() {
        let block = quantized(7, 0);
        let err = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude {
                plane: PlaneId::Y,
                coefficient_index: 0,
                magnitude: 28,
                max_magnitude: MAX_BASE_EOB_MAGNITUDE,
                ..
            }
        ));
    }

    #[test]
    fn rejects_wrong_coefficient_count() {
        let coefficients = [0; DCT_DCT_4X4_COEFF_COUNT - 1];
        let err = tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients))
            .unwrap_err();

        assert!(matches!(
            err,
            Error::CoefficientTokenizationInputLengthMismatch {
                plane: PlaneId::Y,
                expected: 16,
                actual: 15,
                ..
            }
        ));
    }
}
