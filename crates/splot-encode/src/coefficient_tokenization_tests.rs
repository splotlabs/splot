// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

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
    ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &uniform(sample)).unwrap()
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
fn coded_dc_tokens_match_tokenizer() {
    // `tokenize_coefficients` delegates to `luma_dc_coded_tokens`, so they
    // must agree across the full supported magnitude/sign range (base tier
    // 1..=4 and base-range tier 5..=7) — this guards the trace accessor.
    let max = MAX_BASE_BR_MAGNITUDE as i32;
    for mag in 1..=max {
        for dc in [mag, -mag] {
            let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
            coefficients[0] = dc;
            let plan =
                tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients))
                    .unwrap();
            let expected = luma_dc_coded_tokens(0, dc.unsigned_abs(), dc < 0).unwrap();
            assert_eq!(plan.tokens(), expected.as_slice(), "dc = {dc}");
        }
    }
}

#[test]
fn base_range_tier_emits_coeff_br() {
    // Magnitude 5..=7 saturates coeff_base_eob at 4 (level 5) and emits one
    // coeff_br = magnitude - 5 (§5.20.7.27); magnitude 4 stays base-only.
    // Magnitude 8 (coeff_br=3, level == maxLevel) needs the golomb tail and
    // is rejected (see `rejects_magnitude_outside_base_range_tier`).
    for (magnitude, expected) in [(4u32, None), (5, Some(0u8)), (6, Some(1)), (7, Some(2))] {
        let tokens = luma_dc_coded_tokens(0, magnitude, false).unwrap();
        let br = tokens
            .iter()
            .find(|t| t.syntax == CoefficientTokenSyntax::CoeffBr)
            .copied();
        match expected {
            None => assert!(
                br.is_none(),
                "magnitude {magnitude} should not emit coeff_br"
            ),
            Some(sym) => {
                let br = br.expect("base-range magnitude must emit coeff_br");
                assert_eq!(br.symbol, sym, "magnitude {magnitude} coeff_br symbol");
                assert_eq!(
                    br.selector,
                    CoefficientCdfRowSelector::CoeffBrLf {
                        coeff_cdf_q_ctx: 0,
                        ctx: COEFF_BR_LF_CTX_DC,
                    }
                );
                // The base-eob symbol saturates at 4 (level 5) in the br tier.
                assert_eq!(tokens[2].symbol, 4, "magnitude {magnitude} coeff_base_eob");
            }
        }
    }
}

#[test]
fn base_range_tier_tokens_roundtrip() {
    // The 5-token br sequence roundtrips through the §8.2 coder.
    let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
    coefficients[0] = 6; // coeff_base_eob=4, coeff_br=1, dc_sign=0
    let plan =
        tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients)).unwrap();
    let proof = roundtrip_entropy_tokens(plan.tokens()).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 4, 1, 0]);
    assert_eq!(proof.symbol_count(), 5);
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
    let err =
        tokenize_coefficients(raw_input(PlaneId::U, rect(4, 4), 4, 4, &coefficients)).unwrap_err();

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
    let err =
        tokenize_coefficients(raw_input(PlaneId::Y, rect(2, 4), 2, 4, &coefficients)).unwrap_err();

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
    let err =
        tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients)).unwrap_err();

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
fn rejects_magnitude_outside_base_range_tier() {
    // Magnitude 28 is beyond the base+range tier (max 7); the golomb tail
    // that would encode it is a later brick.
    let block = quantized(7, 0);
    let err = tokenize_quantized_4x4_dct_dct_dc_only(&block).unwrap_err();

    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedMagnitude {
            plane: PlaneId::Y,
            coefficient_index: 0,
            magnitude: 28,
            max_magnitude: MAX_BASE_BR_MAGNITUDE,
            ..
        }
    ));
}

#[test]
fn rejects_max_level_magnitude_requiring_golomb() {
    // Magnitude 8 reaches maxLevel (LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1),
    // so AV2 §5.20.7.28 read_quant emits the golomb tail; until that brick
    // lands, magnitude 8 must be rejected rather than emit an incomplete
    // (coeff_br=3, no golomb) token stream.
    assert_eq!(MAX_BASE_BR_MAGNITUDE, 7);
    let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
    coefficients[0] = 8;
    let err =
        tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients)).unwrap_err();

    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedMagnitude {
            magnitude: 8,
            max_magnitude: 7,
            ..
        }
    ));
}

#[test]
fn rejects_wrong_coefficient_count() {
    let coefficients = [0; DCT_DCT_4X4_COEFF_COUNT - 1];
    let err =
        tokenize_coefficients(raw_input(PlaneId::Y, rect(4, 4), 4, 4, &coefficients)).unwrap_err();

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
