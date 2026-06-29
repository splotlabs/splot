// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientTokenSyntax, TX_SIZE_4X4_CTX, luma_dc_coded_tokens,
};

const Q_CTX: usize = 0;
/// The 4x4 2D scan order `[0, 4, 1, 8, ...]`: scan index 1 maps to raster 4.
const AC_RASTER_POS: usize = 4;

/// A raster `[i32; 16]` with `dc` at raster position 0 and `ac` at raster
/// position 4 (the scan-index-1 low-frequency AC), all other positions zero.
fn dc_ac_block(dc: i32, ac: i32) -> [i32; TX_4X4_COEFF_COUNT] {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[0] = dc;
    quant[AC_RASTER_POS] = ac;
    quant
}

/// The § 8.3.2 derived DC `coeff_base` low-frequency context for an AC neighbour of
/// magnitude `ac_mag` at raster position 4 — computed via the same helper the
/// tokenizer uses, NOT a hard-coded literal.
fn derived_dc_ctx(ac_mag: u32) -> usize {
    let mut level = [0u32; TX_4X4_COEFF_COUNT];
    level[AC_RASTER_POS] = ac_mag;
    coeff_base_lf_luma_context(
        0,
        TX_4X4_BWL,
        TX_4X4_WIDTH,
        TX_4X4_HEIGHT,
        TRANSFORM_CLASS_2D,
        0,
        &level,
    )
}

/// The § 8.3.2 derived NON-EOB DC `coeff_br` low-frequency context for an
/// already-written EOB AC neighbour of magnitude `ac_mag` at raster position 4 —
/// computed via the same helper the tokenizer uses, NOT a hard-coded literal.
fn derived_dc_br_ctx(ac_mag: u32) -> usize {
    let mut level = [0u32; TX_4X4_COEFF_COUNT];
    level[AC_RASTER_POS] = ac_mag;
    coeff_br_lf_luma_context(
        0,
        TX_4X4_BWL,
        TX_4X4_WIDTH,
        TX_4X4_HEIGHT,
        TRANSFORM_CLASS_2D,
        true,
        &level,
    )
}

#[derive(Clone, Copy, Debug)]
struct GeneralLfEob2Case {
    dc: i32,
    ac: i32,
    expected_symbols: &'static [u8],
    ac_base_symbol: u8,
    ac_br_symbol: Option<u8>,
    dc_ctx: usize,
    dc_base_symbol: u8,
    dc_br: Option<(usize, u8)>,
}

fn assert_general_lf_eob2_case(case: GeneralLfEob2Case) {
    let quant = dc_ac_block(case.dc, case.ac);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), case.expected_symbols.len());
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 0
    ));
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 1
    ));
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && ac.symbol() == case.ac_base_symbol),
        "AC coeff_base_eob at context 1, symbol {}",
        case.ac_base_symbol
    );

    let mut index = 3;
    if let Some(ac_br_symbol) = case.ac_br_symbol {
        assert!(
            matches!(trace[index], BlockSymbolToken::Coeff(br)
                if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                    && matches!(br.selector(), CoefficientCdfRowSelector::CoeffBrLf { ctx: 7, .. })
                    && br.symbol() == ac_br_symbol),
            "AC coeff_br at context 7, symbol {ac_br_symbol}"
        );
        index += 1;
    }

    let ac_magnitude = case.ac.unsigned_abs();
    assert_eq!(
        derived_dc_ctx(ac_magnitude),
        case.dc_ctx,
        "derived DC ctx for AC magnitude {ac_magnitude}"
    );
    assert!(
        matches!(trace[index], BlockSymbolToken::Coeff(dc)
            if dc.selector() == CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx: Q_CTX,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: case.dc_ctx,
                tcq_ctx: 0,
            } && dc.symbol() == case.dc_base_symbol),
        "DC coeff_base at context {}, symbol {}",
        case.dc_ctx,
        case.dc_base_symbol
    );
    index += 1;

    if let Some((dc_br_ctx, dc_br_symbol)) = case.dc_br {
        assert_eq!(
            derived_dc_br_ctx(ac_magnitude),
            dc_br_ctx,
            "derived DC br ctx for AC magnitude {ac_magnitude}"
        );
        assert!(
            matches!(trace[index], BlockSymbolToken::Coeff(br)
                if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                    && br.selector() == CoefficientCdfRowSelector::CoeffBrLf {
                        coeff_cdf_q_ctx: Q_CTX,
                        ctx: dc_br_ctx,
                    }
                    && br.symbol() == dc_br_symbol),
            "DC coeff_br at context {dc_br_ctx}, symbol {dc_br_symbol}"
        );
        index += 1;
    }

    assert!(matches!(
        trace[index],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    index += 1;
    assert!(
        matches!(trace[index], BlockSymbolToken::Coeff(dc)
            if matches!(dc.syntax(), CoefficientTokenSyntax::DcSign)
                && dc.symbol() == u8::from(case.dc < 0)),
        "DC sign follows the AC sign bypass"
    );
    index += 1;
    assert_eq!(trace.len() - index, 2);
    for token in &trace[index..] {
        assert!(matches!(
            token,
            BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 1
        ));
    }

    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        case.expected_symbols
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), case.expected_symbols);
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn tokenizes_general_lf_eob2_in_order() {
    assert_general_lf_eob2_case(GeneralLfEob2Case {
        dc: -2,
        ac: 3,
        expected_symbols: &[0, 1, 2, 2, 0, 1, 1, 1],
        ac_base_symbol: 2,
        ac_br_symbol: None,
        dc_ctx: 2,
        dc_base_symbol: 2,
        dc_br: None,
    });
}

#[test]
fn general_lf_eob2_roundtrips_and_recovers_quant() {
    let quant = dc_ac_block(-2, 3);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0, 1, 2, 2, 0, 1, 1, 1]);
    assert!(!proof.bytes().is_empty());

    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob1_dc_only_matches_existing() {
    let dc = -3;
    let quant = {
        let mut q = [0i32; TX_4X4_COEFF_COUNT];
        q[0] = dc;
        q
    };
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let magnitude = dc.unsigned_abs();
    let existing = luma_dc_coded_tokens(Q_CTX, magnitude, dc < 0).unwrap();
    let general_luma: Vec<CoefficientEntropyToken> = trace
        .iter()
        .filter_map(|token| match token {
            BlockSymbolToken::Coeff(coeff) => Some(*coeff),
            _ => None,
        })
        .take(existing.len())
        .collect();
    assert_eq!(general_luma, existing);

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_all_zero_emits_single_txb_skip() {
    let quant = [0i32; TX_4X4_COEFF_COUNT];
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 1);
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 1
    ));

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[1]);

    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_recover_quant_is_deterministic() {
    let quant = dc_ac_block(-2, 3);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let first = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    let second = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, quant);

    let proof_a = roundtrip_block_symbol_trace(&trace).unwrap();
    let proof_b = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof_a.bytes(), proof_b.bytes());
    assert_eq!(proof_a.decoded_symbols(), proof_b.decoded_symbols());
}

#[test]
fn general_lf_rejects_out_of_scope() {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[10] = 1; // raster 10 is scan index 11 — now in scope.
    assert!(
        tokenize_general_lf_luma_block(&quant, Q_CTX).is_ok(),
        "scan index 11 (eob 12) is now in scope"
    );
    assert_eq!(MAX_GENERAL_SCAN_INDEX, 15);

    assert_eq!(MAX_BASE_BR_MAGNITUDE, 7);
    let golomb = dc_ac_block(8, 1);
    let trace = tokenize_general_lf_luma_block(&golomb, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, golomb);

    let too_big = dc_ac_block(526, 1);
    let err = tokenize_general_lf_luma_block(&too_big, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedMagnitude {
            magnitude: 526,
            max_magnitude: 525,
            ..
        }
    ));
}

#[test]
fn general_lf_eob1_dc_br_matches_existing() {
    for magnitude in [5u32, 6, 7] {
        let dc = -(magnitude as i32);
        let quant = {
            let mut q = [0i32; TX_4X4_COEFF_COUNT];
            q[0] = dc;
            q
        };
        let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

        let existing = luma_dc_coded_tokens(Q_CTX, magnitude, dc < 0).unwrap();
        let general_luma: Vec<CoefficientEntropyToken> = trace
            .iter()
            .filter_map(|token| match token {
                BlockSymbolToken::Coeff(coeff) => Some(*coeff),
                _ => None,
            })
            .take(existing.len())
            .collect();
        assert_eq!(general_luma, existing, "magnitude {magnitude}");

        assert!(
            matches!(trace[3], BlockSymbolToken::Coeff(br)
                if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                    && matches!(br.selector(), CoefficientCdfRowSelector::CoeffBrLf { ctx: 0, .. })
                    && br.symbol() == (magnitude - 5) as u8),
            "DC coeff_br at ctx 0, symbol {} for magnitude {magnitude}",
            magnitude - 5
        );

        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert!(!proof.bytes().is_empty());
        let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
        assert_eq!(recovered, quant, "magnitude {magnitude}");
    }
}

#[test]
fn general_lf_eob2_ac_br_in_order_and_roundtrips() {
    assert_general_lf_eob2_case(GeneralLfEob2Case {
        dc: -2,
        ac: 6,
        expected_symbols: &[0, 1, 4, 1, 2, 0, 1, 1, 1],
        ac_base_symbol: 4,
        ac_br_symbol: Some(1),
        dc_ctx: 3,
        dc_base_symbol: 2,
        dc_br: None,
    });
}

#[test]
fn coeff_br_lf_luma_context_matches_decoder_derivation() {
    assert_eq!(derived_dc_br_ctx(3), 2);
    assert_eq!(derived_dc_br_ctx(6), 3);
    assert_eq!(derived_dc_br_ctx(1), 1);
    assert_eq!(derived_dc_br_ctx(2), 1);
    assert_eq!(derived_dc_br_ctx(7), 3);
    assert_eq!(
        coeff_br_lf_luma_context(
            0,
            TX_4X4_BWL,
            TX_4X4_WIDTH,
            TX_4X4_HEIGHT,
            TRANSFORM_CLASS_2D,
            true,
            &[0u32; TX_4X4_COEFF_COUNT],
        ),
        0
    );
}

#[test]
fn general_lf_eob2_dc_br_in_order_and_roundtrips() {
    assert_general_lf_eob2_case(GeneralLfEob2Case {
        dc: -6,
        ac: 2,
        expected_symbols: &[0, 1, 1, 5, 1, 0, 1, 1, 1],
        ac_base_symbol: 1,
        ac_br_symbol: None,
        dc_ctx: 1,
        dc_base_symbol: 5,
        dc_br: Some((1, 1)),
    });
}

#[test]
fn general_lf_both_coeffs_br() {
    assert_general_lf_eob2_case(GeneralLfEob2Case {
        dc: -6,
        ac: 7,
        expected_symbols: &[0, 1, 4, 2, 5, 1, 0, 1, 1, 1],
        ac_base_symbol: 4,
        ac_br_symbol: Some(2),
        dc_ctx: 3,
        dc_base_symbol: 5,
        dc_br: Some((3, 1)),
    });
}

#[test]
fn general_lf_eob2_single_and_double_golomb_accepted() {
    let golomb_ac = dc_ac_block(-1, 8);
    let trace = tokenize_general_lf_luma_block(&golomb_ac, Q_CTX).unwrap();
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), golomb_ac);

    let golomb_dc = dc_ac_block(8, 1);
    let trace = tokenize_general_lf_luma_block(&golomb_dc, Q_CTX).unwrap();
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), golomb_dc);

    let two_golomb = dc_ac_block(8, 8);
    let trace = tokenize_general_lf_luma_block(&two_golomb, Q_CTX).unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(
        recover_quant_from_tokens(&trace, Q_CTX).unwrap(),
        two_golomb
    );
}

#[test]
fn general_lf_sign_swap_negative_test() {
    let quant = dc_ac_block(-2, 3);
    let swapped = dc_ac_block(2, -3);
    assert_ne!(swapped, quant);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
    assert_ne!(recovered, swapped);
}

#[test]
fn general_lf_eob2_dc_br_ctx_mid_routes_and_roundtrips() {
    assert_eq!(
        derived_dc_br_ctx(4),
        2,
        "AC magnitude 4 derives DC coeff_br ctx 2"
    );
    let quant = dc_ac_block(-6, 4);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}
