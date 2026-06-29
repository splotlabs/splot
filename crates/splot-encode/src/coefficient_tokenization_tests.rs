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

fn raw_input(
    plane: PlaneId,
    block: PlaneRect,
    width: usize,
    height: usize,
    coefficients: &[i32],
) -> CoefficientTokenizationInput<'_> {
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
                assert_eq!(tokens[2].symbol, 4, "magnitude {magnitude} coeff_base_eob");
            }
        }
    }
}

#[test]
fn base_range_tier_tokens_roundtrip() {
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
fn general_intra_64x64_luma_all_zero_token_targets_tx_64x64_neutral() {
    let token = general_intra_64x64_luma_all_zero_token(0);

    assert_eq!(
        token,
        CoefficientEntropyToken {
            syntax: CoefficientTokenSyntax::AllZero,
            selector: CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx: 0,
                plane_type: LUMA_PLANE_TYPE,
                tx_size: TX_SIZE_64X64_CTX,
                ctx: TXB_SKIP_CTX_NEUTRAL,
            },
            symbol: 1,
        }
    );
    assert_eq!(general_intra_64x64_luma_all_zero_token(0).symbol(), 1);
}

#[test]
fn general_intra_32x32_chroma_u_all_zero_token_targets_tx_32x32_ctx6() {
    let token = general_intra_32x32_chroma_u_all_zero_token(0);

    assert_eq!(
        token,
        CoefficientEntropyToken {
            syntax: CoefficientTokenSyntax::AllZero,
            selector: CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx: 0,
                plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
                tx_size: TX_SIZE_32X32_CTX,
                ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
            },
            symbol: 1,
        }
    );
    assert_eq!(general_intra_32x32_chroma_u_all_zero_token(0).symbol(), 1);
}

#[test]
fn general_intra_64x64_luma_dc_coded_tokens_use_eob_pt_1024_and_tx_64x64() {
    let tokens = general_intra_64x64_luma_dc_coded_tokens(0, 6, true).unwrap();
    let syntaxes: Vec<_> = tokens.iter().map(|t| t.syntax()).collect();
    assert_eq!(
        syntaxes,
        vec![
            CoefficientTokenSyntax::AllZero,
            CoefficientTokenSyntax::EobPt1024,
            CoefficientTokenSyntax::CoeffBaseEob,
            CoefficientTokenSyntax::CoeffBr,
            CoefficientTokenSyntax::DcSign,
        ]
    );
    assert!(matches!(
        tokens[0].selector(),
        CoefficientCdfRowSelector::TxbSkip {
            tx_size: TX_SIZE_64X64_CTX,
            ..
        }
    ));
    assert!(matches!(
        tokens[1].selector(),
        CoefficientCdfRowSelector::EobPt1024 {
            coeff_cdf_q_ctx: 0,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        }
    ));
    assert!(matches!(
        tokens[2].selector(),
        CoefficientCdfRowSelector::CoeffBaseLfEob {
            tx_size: TX_SIZE_64X64_CTX,
            ..
        }
    ));
    let symbols: Vec<_> = tokens.iter().map(|t| t.symbol()).collect();
    assert_eq!(symbols, vec![0, 0, 4, 1, 1]);

    let small = general_intra_64x64_luma_dc_coded_tokens(0, 2, false).unwrap();
    assert_eq!(small.len(), 4);
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
fn general_intra_16x16_luma_dc_coded_tokens_use_eob_pt_256_and_tx_16x16() {
    let tokens = general_intra_16x16_luma_dc_coded_tokens(0, 6, true).unwrap();
    let syntaxes: Vec<_> = tokens.iter().map(|t| t.syntax()).collect();
    assert_eq!(
        syntaxes,
        vec![
            CoefficientTokenSyntax::AllZero,
            CoefficientTokenSyntax::EobPt256,
            CoefficientTokenSyntax::CoeffBaseEob,
            CoefficientTokenSyntax::CoeffBr,
            CoefficientTokenSyntax::DcSign,
        ]
    );
    assert!(matches!(
        tokens[0].selector(),
        CoefficientCdfRowSelector::TxbSkip {
            tx_size: TX_SIZE_16X16_CTX,
            ..
        }
    ));
    assert!(matches!(
        tokens[1].selector(),
        CoefficientCdfRowSelector::EobPt256 {
            coeff_cdf_q_ctx: 0,
            eob_ctx: EOB_CTX_LUMA_INTRA,
        }
    ));
    assert!(matches!(
        tokens[2].selector(),
        CoefficientCdfRowSelector::CoeffBaseLfEob {
            tx_size: TX_SIZE_16X16_CTX,
            ctx: COEFF_BASE_LF_EOB_CTX_DC,
            ..
        }
    ));
    let symbols: Vec<_> = tokens.iter().map(|t| t.symbol()).collect();
    assert_eq!(symbols, vec![0, 0, 4, 1, 1]);

    let small = general_intra_16x16_luma_dc_coded_tokens(0, 2, false).unwrap();
    assert_eq!(small.len(), 4);
    assert!(
        small
            .iter()
            .all(|t| t.syntax() != CoefficientTokenSyntax::CoeffBr)
    );
}

#[test]
fn general_intra_16x16_luma_dc_tokens_roundtrip_through_entropy_proof() {
    let tokens = general_intra_16x16_luma_dc_coded_tokens(0, 6, true).unwrap();
    let expected: Vec<u8> = tokens.iter().map(|token| token.symbol()).collect();
    assert_eq!(expected, vec![0, 0, 4, 1, 1]);

    let proof = roundtrip_entropy_tokens(&tokens).unwrap();
    assert_eq!(proof.decoded_symbols(), expected.as_slice());
    assert_eq!(proof.symbol_count(), tokens.len() as u64);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn general_intra_16x16_luma_dc_block_with_chroma_tail_roundtrips_through_one_coder() {
    use crate::block_symbol_trace::{BlockSymbolToken, roundtrip_block_symbol_trace};

    let luma = general_intra_16x16_luma_dc_coded_tokens(0, 6, true).unwrap();
    let mut trace: Vec<BlockSymbolToken> = luma.into_iter().map(BlockSymbolToken::Coeff).collect();
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(0)));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(0, 0)));

    let expected: Vec<u8> = trace.iter().map(|token| token.symbol()).collect();
    assert_eq!(expected, vec![0, 0, 4, 1, 1, 1, 1]);

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), expected.as_slice());
    assert_eq!(proof.symbol_count(), trace.len() as u64);
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

#[test]
fn chroma_u_dc_coded_coeff_tokens_emit_chroma_contexts() {
    let tokens = chroma_u_dc_coded_coeff_tokens(0, 3).unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(
        tokens[0].selector(),
        CoefficientCdfRowSelector::TxbSkip {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            tx_size: 0,
            ctx: 6,
        }
    );
    assert_eq!(tokens[0].symbol(), 0);
    assert_eq!(
        tokens[1].selector(),
        CoefficientCdfRowSelector::EobPt16 {
            coeff_cdf_q_ctx: 0,
            eob_ctx: 2,
        }
    );
    assert_eq!(
        tokens[2].selector(),
        CoefficientCdfRowSelector::CoeffBaseLfEobUv {
            coeff_cdf_q_ctx: 0,
            ctx: 0,
        }
    );
    assert_eq!(tokens[2].symbol(), 2); // magnitude 3 → coeff_base_eob = 2
}

#[test]
fn chroma_u_dc_coded_coeff_tokens_roundtrip_through_generic_helper() {
    let tokens = chroma_u_dc_coded_coeff_tokens(0, 3).unwrap();
    let proof = roundtrip_entropy_tokens(&tokens).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 2]);
    assert_eq!(proof.symbol_count(), 3);
}

#[test]
fn chroma_u_dc_coded_coeff_tokens_reject_out_of_tier_magnitude() {
    for magnitude in [0u32, 5, 28] {
        let err = chroma_u_dc_coded_coeff_tokens(0, magnitude).unwrap_err();
        assert!(
            matches!(
                err,
                Error::CoefficientTokenizationUnsupportedChromaMagnitude {
                    plane: PlaneId::U,
                    max_magnitude: 4,
                    ..
                }
            ),
            "magnitude {magnitude}"
        );
    }
}

const LF_CTX_BWL: u32 = 2;
const LF_CTX_TXW: usize = 4;
const LF_CTX_TXH: usize = 4;
const LF_CTX_2D: usize = 0;

#[test]
fn coeff_base_lf_dc_context_for_single_ac_neighbour_is_one() {
    let mut level = [0u32; 16];
    level[1] = 1;
    let ctx =
        coeff_base_lf_luma_context(0, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 0, &level);
    assert_eq!(ctx, 1);
}

#[test]
fn coeff_base_lf_dc_context_clamps_neighbour_magnitude() {
    let mut level = [0u32; 16];
    level[1] = 10;
    let ctx =
        coeff_base_lf_luma_context(0, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 0, &level);
    assert_eq!(ctx, 3);
}

#[test]
fn coeff_base_lf_context_bands_match_spec_mapping() {
    let zero = [0u32; 16];
    assert_eq!(
        coeff_base_lf_luma_context(0, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 0, &zero),
        0
    );
    assert_eq!(
        coeff_base_lf_luma_context(1, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 1, &zero),
        9
    );
    assert_eq!(
        coeff_base_lf_luma_context(5, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 2, &zero),
        16
    );
}

#[test]
fn coeff_base_lf_context_out_of_range_neighbours_do_not_panic() {
    let zero = [0u32; 16];
    assert_eq!(
        coeff_base_lf_luma_context(15, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 3, &zero),
        16
    );
    let short: [u32; 1] = [7];
    assert_eq!(
        coeff_base_lf_luma_context(0, LF_CTX_BWL, LF_CTX_TXW, LF_CTX_TXH, LF_CTX_2D, 0, &short),
        0
    );
}

#[test]
fn coeff_base_lf_token_carries_non_eob_base_level() {
    let token = coeff_base_lf_token(
        0,
        COEFF_BASE_LF_CTX_EOB2_DC,
        COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
        0,
    );
    assert_eq!(token.syntax(), CoefficientTokenSyntax::CoeffBase);
    assert_eq!(token.symbol(), 0);
    assert!(matches!(
        token.selector(),
        CoefficientCdfRowSelector::CoeffBaseLf {
            coeff_cdf_q_ctx: 0,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: 1,
            tcq_ctx: 0,
        }
    ));
}

#[test]
fn coeff_base_lf_token_roundtrips_through_generic_helper() {
    for level in [0u8, 2, 5] {
        let token = coeff_base_lf_token(
            0,
            COEFF_BASE_LF_CTX_EOB2_DC,
            COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            level,
        );
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.decoded_symbols(), &[level]);
        assert_eq!(proof.symbol_count(), 1);
    }
}

#[test]
fn eob_extra_token_roundtrips_through_entropy_proof() {
    for flag in [false, true] {
        let token = eob_extra_token(0, flag);
        assert!(matches!(
            token.selector(),
            CoefficientCdfRowSelector::EobExtra { coeff_cdf_q_ctx: 0 }
        ));
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.decoded_symbols(), &[u8::from(flag)]);
        assert_eq!(proof.symbol_count(), 1);
    }
}

#[test]
fn entropy_proof_routes_full_4x4_lf_context_banks() {
    for ctx in 0..COEFF_BASE_LF_EOB_CTX_COUNT {
        let token = coeff_base_lf_eob_token(0, ctx, 1);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
    for ctx in 0..COEFF_BASE_LF_CTX_COUNT {
        let token = coeff_base_lf_token(0, ctx, COEFF_BASE_LF_TCQ_CTX_NEUTRAL, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
    for ctx in 0..COEFF_BR_LF_CTX_COUNT {
        let token = coeff_br_lf_token(0, ctx, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
}

#[test]
fn entropy_proof_routes_full_4x4_hf_context_banks() {
    for ctx in 0..COEFF_BASE_EOB_CTX_COUNT {
        let token = coeff_base_hf_eob_token(0, ctx, 1);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
    for ctx in 0..COEFF_BASE_CTX_COUNT {
        let token = coeff_base_hf_token(0, ctx, COEFF_BASE_LF_TCQ_CTX_NEUTRAL, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
    for ctx in 0..COEFF_BR_CTX_COUNT {
        let token = coeff_br_hf_token(0, ctx, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.symbol_count(), 1);
    }
}

#[test]
fn multi_coeff_token_accessors_carry_expected_symbols() {
    assert_eq!(coded_luma_all_zero_token(0).symbol(), 0);
    assert_eq!(eob_pt_16_token(0, EOB_CTX_LUMA_INTRA, 1).symbol(), 1);
    let ac = coeff_base_lf_eob_token(0, COEFF_BASE_LF_EOB_CTX_EOB2_AC, 1);
    assert_eq!(ac.symbol(), 0);
    assert!(matches!(
        ac.selector(),
        CoefficientCdfRowSelector::CoeffBaseLfEob {
            coeff_cdf_q_ctx: 0,
            tx_size: TX_SIZE_4X4_CTX,
            ctx: 1,
        }
    ));
}

#[test]
fn multi_coeff_eob2_cdf_subsequence_roundtrips() {
    let tokens = [
        coded_luma_all_zero_token(0),
        eob_pt_16_token(0, EOB_CTX_LUMA_INTRA, 1),
        coeff_base_lf_eob_token(0, COEFF_BASE_LF_EOB_CTX_EOB2_AC, 1),
        coeff_base_lf_token(
            0,
            COEFF_BASE_LF_CTX_EOB2_DC,
            COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            0,
        ),
    ];
    let proof = roundtrip_entropy_tokens(&tokens).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 1, 0, 0]);
    assert_eq!(proof.symbol_count(), 4);
}

#[test]
fn intra_tx_type_set1_dct_dct_is_symbol_zero() {
    use splot_core::tables::conversion::MD_IDX_TO_TYPE;
    const DCT_DCT: i32 = 0;
    const SIZE_CLASS_4X4: usize = 0;
    const DC_PRED: usize = 0;
    assert_eq!(MD_IDX_TO_TYPE[SIZE_CLASS_4X4][DC_PRED][0], DCT_DCT);
}

#[test]
fn intra_tx_type_set1_token_roundtrips_through_generic_helper() {
    let token = intra_tx_type_set1_token(INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4, 0);
    assert_eq!(token.syntax(), CoefficientTokenSyntax::IntraTxType);
    assert_eq!(token.symbol(), 0);
    assert!(matches!(
        token.selector(),
        CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr: 0 }
    ));
    let proof = roundtrip_entropy_tokens(&[token]).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0]);
    assert_eq!(proof.symbol_count(), 1);
}

#[test]
fn intra_tx_type_set1_token_roundtrips_every_tx_size_sqr_row() {
    for tx_size_sqr in 0..INTRA_TX_TYPE_SET1_TX_SIZE_SQR_COUNT {
        let token = intra_tx_type_set1_token(tx_size_sqr, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0], "tx_size_sqr {tx_size_sqr}");
    }
}

#[test]
fn sec_tx_type_intra_off_token_roundtrips_through_generic_helper() {
    let token = sec_tx_type_intra_token(SEC_TX_TYPE_TX_SIZE_SQR_4X4, 0);
    assert_eq!(token.syntax(), CoefficientTokenSyntax::SecTxType);
    assert_eq!(token.symbol(), 0);
    assert!(matches!(
        token.selector(),
        CoefficientCdfRowSelector::SecTxTypeIntra { tx_size_sqr: 0 }
    ));
    let proof = roundtrip_entropy_tokens(&[token]).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0]);
    assert_eq!(proof.symbol_count(), 1);
}

#[test]
fn sec_tx_type_intra_token_roundtrips_every_tx_size_sqr_row() {
    for tx_size_sqr in 0..SEC_TX_TYPE_TX_SIZE_SQR_COUNT {
        let token = sec_tx_type_intra_token(tx_size_sqr, 0);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0], "tx_size_sqr {tx_size_sqr}");
    }
}

#[test]
fn sec_tx_type_intra_token_roundtrips_every_symbol_value() {
    const STX_TYPES: u8 = 4;
    for symbol in 0..STX_TYPES {
        let token = sec_tx_type_intra_token(SEC_TX_TYPE_TX_SIZE_SQR_4X4, symbol);
        let proof = roundtrip_entropy_tokens(&[token]).unwrap();
        assert_eq!(proof.decoded_symbols(), &[symbol], "sec_tx_type {symbol}");
    }
}
