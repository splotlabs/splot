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

#[test]
fn tokenizes_general_lf_eob2_in_order() {
    // Asymmetric input: DC = -2 (raster 0), AC = +3 (raster 4 = scan index 1).
    let quant = dc_ac_block(-2, 3);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // luma all_zero(0), eob_pt_16(1), AC coeff_base_eob, DC coeff_base, AC sign
    // bypass, DC dc_sign, U all_zero(1), V all_zero(1).
    assert_eq!(trace.len(), 8);

    // coded luma all_zero == 0.
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 0
    ));
    // eob_pt_16 == 1 (eob 2).
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 1
    ));
    // AC coeff_base_eob at context 1 (coeff_base_eob_ctx(c=1) = 1), symbol 2 (level 3).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && ac.symbol() == 2),
        "AC coeff_base_eob at context 1, symbol 2"
    );
    // DC coeff_base at the DERIVED low-frequency context (computed, not a literal),
    // symbol 2 (min(mag 2, 5)).
    let dc_ctx = derived_dc_ctx(3);
    assert_eq!(
        dc_ctx, 2,
        "the derived DC ctx for an AC of magnitude 3 is 2"
    );
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(dc)
            if dc.selector() == CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx: Q_CTX,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: dc_ctx,
                tcq_ctx: 0,
            } && dc.symbol() == 2),
        "DC coeff_base at the derived low-frequency context {dc_ctx}, symbol 2"
    );
    // The AC sign_bit bypass(1, false) comes BEFORE the DC dc_sign — the reverse-scan
    // AC-before-DC contract.
    assert!(matches!(
        trace[4],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(dc)
            if matches!(dc.syntax(), CoefficientTokenSyntax::DcSign) && dc.symbol() == 1),
        "DC dc_sign == 1 (negative) after the AC sign bypass"
    );
    // Chroma U / V all_zero == 1.
    for token in &trace[6..8] {
        assert!(matches!(
            token,
            BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 1
        ));
    }

    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        vec![0, 1, 2, 2, 0, 1, 1, 1]
    );
}

#[test]
fn general_lf_eob2_roundtrips_and_recovers_quant() {
    let quant = dc_ac_block(-2, 3);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // § 8.2 self-consistency roundtrip through one coder.
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0, 1, 2, 2, 0, 1, 1, 1]);
    assert!(!proof.bytes().is_empty());

    // recover_quant rebuilds the exact signed block from the emitted triples.
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob1_dc_only_matches_existing() {
    // An eob=1 single-DC block must produce the same DC tokens as the existing
    // `luma_dc_coded_tokens` single-source-of-truth path.
    let dc = -3;
    let quant = {
        let mut q = [0i32; TX_4X4_COEFF_COUNT];
        q[0] = dc;
        q
    };
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let magnitude = dc.unsigned_abs();
    let existing = luma_dc_coded_tokens(Q_CTX, magnitude, dc < 0).unwrap();
    // The general walk's luma residual is: all_zero(0), eob_pt_16(0), DC
    // coeff_base_eob, DC dc_sign — identical token records to the existing path
    // (which emits all_zero, eob_pt_16, coeff_base_eob, dc_sign for a base-tier DC).
    let general_luma: Vec<CoefficientEntropyToken> = trace
        .iter()
        .filter_map(|token| match token {
            BlockSymbolToken::Coeff(coeff) => Some(*coeff),
            _ => None,
        })
        .take(existing.len())
        .collect();
    assert_eq!(general_luma, existing);

    // And it roundtrips and recovers the DC.
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

    // The roundtrip itself is also deterministic.
    let proof_a = roundtrip_block_symbol_trace(&trace).unwrap();
    let proof_b = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof_a.bytes(), proof_b.bytes());
    assert_eq!(proof_a.decoded_symbols(), proof_b.decoded_symbols());
}

#[test]
fn general_lf_rejects_out_of_scope() {
    // A nonzero at scan index >= 2 (raster position 1 = scan index 2) is rejected.
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[1] = 1; // raster 1 is scan index 2 in the 4x4 2D order.
    let err = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedEob {
            scan_index: 2,
            position: 1,
            value: 1,
            max_scan_index: MAX_GENERAL_LF_SCAN_INDEX,
        }
    ));

    // A magnitude > 4 (the base tier) is rejected via the existing magnitude error.
    let big = dc_ac_block(5, 1);
    let err = tokenize_general_lf_luma_block(&big, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedMagnitude {
            magnitude: 5,
            max_magnitude: MAX_BASE_EOB_MAGNITUDE,
            ..
        }
    ));
}

#[test]
fn general_lf_sign_swap_negative_test() {
    // Build a recovered block with the AC-before-DC sign order deliberately swapped:
    // the negative test must NOT equal the input (the reverse-scan AC-before-DC
    // sign contract is load-bearing). DC = -2, AC = +3 → swapped signs give DC = +2,
    // AC = -3.
    let quant = dc_ac_block(-2, 3);
    let swapped = dc_ac_block(2, -3);
    assert_ne!(swapped, quant);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    // The honest recovery equals the input; the deliberately swapped block does not.
    assert_eq!(recovered, quant);
    assert_ne!(recovered, swapped);
}
