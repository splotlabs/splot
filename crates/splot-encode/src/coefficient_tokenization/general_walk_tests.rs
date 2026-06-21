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
    // A nonzero at scan index >= 4 (eob >= 5, raster position 5 = scan index 4) is
    // rejected: eob >= 5 needs the deferred `eob_extra_bit` bypass literals.
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[5] = 1; // raster 5 is scan index 4 in the 4x4 2D order [0,4,1,8,5,...].
    let err = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedEob {
            scan_index: 4,
            position: 5,
            value: 1,
            max_scan_index: MAX_GENERAL_LF_SCAN_INDEX,
        }
    ));

    // A magnitude > 7 (past the base-range tier) is rejected via the magnitude error.
    let big = dc_ac_block(8, 1);
    let err = tokenize_general_lf_luma_block(&big, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationUnsupportedMagnitude {
            magnitude: 8,
            max_magnitude: MAX_BASE_BR_MAGNITUDE,
            ..
        }
    ));
}

#[test]
fn general_lf_eob1_dc_br_matches_existing() {
    // An eob==1 DC whose magnitude reaches the base-range tier (5, 6, 7) must produce
    // tokens consistent with the existing `luma_dc_coded_tokens` single-source path:
    // `coeff_base_eob` (level min(mag,5)) + `coeff_br` at the constant DC ctx 0.
    for magnitude in [5u32, 6, 7] {
        let dc = -(magnitude as i32);
        let quant = {
            let mut q = [0i32; TX_4X4_COEFF_COUNT];
            q[0] = dc;
            q
        };
        let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

        let existing = luma_dc_coded_tokens(Q_CTX, magnitude, dc < 0).unwrap();
        // The general walk's luma residual prefix matches the existing path exactly:
        // all_zero(0), eob_pt_16(0), coeff_base_eob(level 5 → symbol 4), coeff_br
        // (symbol mag-5, ctx 0), dc_sign.
        let general_luma: Vec<CoefficientEntropyToken> = trace
            .iter()
            .filter_map(|token| match token {
                BlockSymbolToken::Coeff(coeff) => Some(*coeff),
                _ => None,
            })
            .take(existing.len())
            .collect();
        assert_eq!(general_luma, existing, "magnitude {magnitude}");

        // The coeff_br token is present at the DC ctx 0 with symbol mag - 5.
        assert!(
            matches!(trace[3], BlockSymbolToken::Coeff(br)
                if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                    && matches!(br.selector(), CoefficientCdfRowSelector::CoeffBrLf { ctx: 0, .. })
                    && br.symbol() == (magnitude - 5) as u8),
            "DC coeff_br at ctx 0, symbol {} for magnitude {magnitude}",
            magnitude - 5
        );

        // Roundtrip + recover == input.
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert!(!proof.bytes().is_empty());
        let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
        assert_eq!(recovered, quant, "magnitude {magnitude}");
    }
}

#[test]
fn general_lf_eob2_ac_br_in_order_and_roundtrips() {
    // ASYMMETRIC eob==2: AC = +6 (the EOB coefficient, magnitude > 4 → coeff_br ctx 7)
    // and DC = -2 (the non-EOB coefficient, base tier).
    let quant = dc_ac_block(-2, 6);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // luma all_zero(0), eob_pt_16(1), AC coeff_base_eob, AC coeff_br, DC coeff_base,
    // AC sign bypass, DC dc_sign, U all_zero(1), V all_zero(1) = 9 tokens.
    assert_eq!(trace.len(), 9);

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
    // AC coeff_base_eob at ctx 1 (coeff_base_eob_ctx(c=1) = 1), symbol 4 (level
    // min(6,5)=5 → symbol 4).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && ac.symbol() == 4),
        "AC coeff_base_eob at context 1, symbol 4"
    );
    // AC coeff_br at the constant EOB-AC ctx 7, symbol 1 (mag 6 - 5).
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(br)
            if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                && matches!(br.selector(), CoefficientCdfRowSelector::CoeffBrLf { ctx: 7, .. })
                && br.symbol() == 1),
        "AC coeff_br at context 7, symbol 1"
    );
    // DC coeff_base at the DERIVED low-frequency context (computed, not a literal):
    // the AC neighbour of magnitude 6 clamps to the base magLimit 5 → mag 5 → ctx 3.
    let dc_ctx = derived_dc_ctx(6);
    assert_eq!(
        dc_ctx, 3,
        "the derived DC ctx for an AC of magnitude 6 is 3"
    );
    assert!(
        matches!(trace[4], BlockSymbolToken::Coeff(dc)
            if dc.selector() == CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx: Q_CTX,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: dc_ctx,
                tcq_ctx: 0,
            } && dc.symbol() == 2),
        "DC coeff_base at the derived low-frequency context {dc_ctx}, symbol 2"
    );
    // The AC sign_bit bypass(1, false) comes BEFORE the DC dc_sign (reverse-scan
    // AC-before-DC), AC = +6 is positive.
    assert!(matches!(
        trace[5],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    // DC dc_sign == 1 (negative, DC = -2).
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(dc)
            if matches!(dc.syntax(), CoefficientTokenSyntax::DcSign) && dc.symbol() == 1),
        "DC dc_sign == 1 (negative) after the AC sign bypass"
    );
    // Chroma U / V all_zero == 1.
    for token in &trace[7..9] {
        assert!(matches!(
            token,
            BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 1
        ));
    }

    // The full ordered symbol stream.
    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        vec![0, 1, 4, 1, 2, 0, 1, 1, 1]
    );

    // § 8.2 self-consistency roundtrip + recover == input.
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0, 1, 4, 1, 2, 0, 1, 1, 1]);
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn coeff_br_lf_luma_context_matches_decoder_derivation() {
    // The non-EOB DC `coeff_br` context is derived from the running `Level[]` (the
    // already-written EOB AC neighbour at raster 4), mirroring the decoder
    // `CoeffBrContext::ctx`. `mag = Min((min(Level[raster4], 5) + 1) >> 1, 6)`, and the
    // DC (`pos == 0`) yields `mag` directly.
    // Level[raster4] = 3 → min(3,5) = 3 → (3 + 1) >> 1 = 2 → DC ctx 2.
    assert_eq!(derived_dc_br_ctx(3), 2);
    // Level[raster4] = 6 → min(6,5) = 5 → (5 + 1) >> 1 = 3 → DC ctx 3.
    assert_eq!(derived_dc_br_ctx(6), 3);
    // Level[raster4] = 1 → min(1,5) = 1 → (1 + 1) >> 1 = 1 → DC ctx 1.
    assert_eq!(derived_dc_br_ctx(1), 1);
    // The test magnitudes used below: AC mag 2 → ctx 1; AC mag 7 → ctx 3.
    assert_eq!(derived_dc_br_ctx(2), 1);
    assert_eq!(derived_dc_br_ctx(7), 3);
    // An empty `Level[]` (no neighbour) yields mag 0 → DC ctx 0, the constant EOB-DC
    // `coeff_br` context.
    assert_eq!(
        coeff_br_lf_luma_context(
            0,
            TX_4X4_BWL,
            TX_4X4_WIDTH,
            TX_4X4_HEIGHT,
            TRANSFORM_CLASS_2D,
            &[0u32; TX_4X4_COEFF_COUNT],
        ),
        0
    );
}

#[test]
fn general_lf_eob2_dc_br_in_order_and_roundtrips() {
    // ASYMMETRIC eob==2: DC = -6 (the NON-EOB coefficient, magnitude > 4 → interleaved
    // coeff_br at the DERIVED ctx) and AC = +2 (the EOB coefficient, base tier).
    let quant = dc_ac_block(-6, 2);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // luma all_zero(0), eob_pt_16(1), AC coeff_base_eob, DC coeff_base, DC coeff_br,
    // AC sign bypass, DC dc_sign, U all_zero(1), V all_zero(1) = 9 tokens.
    assert_eq!(trace.len(), 9);

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
    // AC coeff_base_eob at ctx 1 (coeff_base_eob_ctx(c=1) = 1), symbol 1 (level
    // min(2,5)=2 → symbol 1). The AC stays base tier (no AC coeff_br).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && ac.symbol() == 1),
        "AC coeff_base_eob at context 1, symbol 1"
    );
    // DC coeff_base at the DERIVED low-frequency context (AC mag 2 → mag 1 → ctx 1),
    // symbol 5 (min(mag 6, 5)).
    let dc_ctx = derived_dc_ctx(2);
    assert_eq!(
        dc_ctx, 1,
        "the derived DC base ctx for an AC of magnitude 2 is 1"
    );
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(dc)
            if dc.selector() == CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx: Q_CTX,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: dc_ctx,
                tcq_ctx: 0,
            } && dc.symbol() == 5),
        "DC coeff_base at the derived context {dc_ctx}, symbol 5"
    );
    // DC coeff_br at the DERIVED ctx (AC mag 2 → DC br ctx 1), symbol 1 (mag 6 - 5).
    let dc_br_ctx = derived_dc_br_ctx(2);
    assert_eq!(
        dc_br_ctx, 1,
        "the derived DC br ctx for an AC of magnitude 2 is 1"
    );
    assert!(
        matches!(trace[4], BlockSymbolToken::Coeff(br)
            if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                && br.selector() == CoefficientCdfRowSelector::CoeffBrLf {
                    coeff_cdf_q_ctx: Q_CTX,
                    ctx: dc_br_ctx,
                }
                && br.symbol() == 1),
        "DC coeff_br at the derived context {dc_br_ctx}, symbol 1"
    );
    // The AC sign_bit bypass(1, false) comes BEFORE the DC dc_sign (reverse-scan
    // AC-before-DC); AC = +2 is positive.
    assert!(matches!(
        trace[5],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    // DC dc_sign == 1 (negative, DC = -6).
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(dc)
            if matches!(dc.syntax(), CoefficientTokenSyntax::DcSign) && dc.symbol() == 1),
        "DC dc_sign == 1 (negative) after the AC sign bypass"
    );
    // Chroma U / V all_zero == 1.
    for token in &trace[7..9] {
        assert!(matches!(
            token,
            BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 1
        ));
    }

    // The full ordered symbol stream.
    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        vec![0, 1, 1, 5, 1, 0, 1, 1, 1]
    );

    // § 8.2 self-consistency roundtrip + recover == input.
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0, 1, 1, 5, 1, 0, 1, 1, 1]);
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_both_coeffs_br() {
    // Both coefficients carry a coeff_br: DC = -6 (non-EOB, derived ctx) and AC = +7
    // (EOB, constant ctx 7). ASYMMETRIC signs/magnitudes.
    let quant = dc_ac_block(-6, 7);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // all_zero(0), eob_pt_16(1), AC coeff_base_eob, AC coeff_br, DC coeff_base,
    // DC coeff_br, AC sign bypass, DC dc_sign, U all_zero(1), V all_zero(1) = 10.
    assert_eq!(trace.len(), 10);

    // AC coeff_base_eob (ctx 1, level min(7,5)=5 → symbol 4).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && ac.symbol() == 4),
        "AC coeff_base_eob at context 1, symbol 4"
    );
    // AC coeff_br at the constant EOB-AC ctx 7, symbol 2 (mag 7 - 5).
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(br)
            if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                && matches!(br.selector(), CoefficientCdfRowSelector::CoeffBrLf { ctx: 7, .. })
                && br.symbol() == 2),
        "AC coeff_br at context 7, symbol 2"
    );
    // DC coeff_base at the derived ctx (AC mag 7 → base mag 5 → ctx 3), symbol 5.
    let dc_ctx = derived_dc_ctx(7);
    assert_eq!(
        dc_ctx, 3,
        "the derived DC base ctx for an AC of magnitude 7 is 3"
    );
    assert!(
        matches!(trace[4], BlockSymbolToken::Coeff(dc)
            if dc.selector() == CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx: Q_CTX,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: dc_ctx,
                tcq_ctx: 0,
            } && dc.symbol() == 5),
        "DC coeff_base at the derived context {dc_ctx}, symbol 5"
    );
    // DC coeff_br at the derived ctx (AC mag 7 → br mag 3 → ctx 3), symbol 1 (mag 6-5).
    let dc_br_ctx = derived_dc_br_ctx(7);
    assert_eq!(
        dc_br_ctx, 3,
        "the derived DC br ctx for an AC of magnitude 7 is 3"
    );
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(br)
            if matches!(br.syntax(), CoefficientTokenSyntax::CoeffBr)
                && br.selector() == CoefficientCdfRowSelector::CoeffBrLf {
                    coeff_cdf_q_ctx: Q_CTX,
                    ctx: dc_br_ctx,
                }
                && br.symbol() == 1),
        "DC coeff_br at the derived context {dc_br_ctx}, symbol 1"
    );

    // Full ordered symbol stream: AC eob_base(4), AC br(2), DC base(5), DC br(1),
    // AC sign(+ → 0), DC sign(- → 1), U/V all_zero(1).
    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        vec![0, 1, 4, 2, 5, 1, 0, 1, 1, 1]
    );

    // § 8.2 self-consistency roundtrip + recover == input.
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0, 1, 4, 2, 5, 1, 0, 1, 1, 1]);
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob2_rejects_oversized() {
    // Both coefficients now accept magnitude `1..=7`; only a magnitude `> 7` (the
    // `read_quant` golomb tail) is rejected, at EITHER position.

    // The EOB AC magnitude limit is 7; magnitude 8 is rejected.
    let big_ac = dc_ac_block(-1, 8);
    let err = tokenize_general_lf_luma_block(&big_ac, Q_CTX).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude {
                magnitude: 8,
                max_magnitude: MAX_BASE_BR_MAGNITUDE,
                ..
            }
        ),
        "EOB AC magnitude 8 exceeds the base-range tier (7): {err:?}"
    );

    // The non-EOB DC magnitude limit is now also 7; magnitude 8 at the DC (with an AC
    // making it non-EOB) is rejected.
    let big_dc = dc_ac_block(8, 1);
    let err = tokenize_general_lf_luma_block(&big_dc, Q_CTX).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude {
                magnitude: 8,
                max_magnitude: MAX_BASE_BR_MAGNITUDE,
                ..
            }
        ),
        "non-EOB DC magnitude 8 exceeds the base-range tier (7): {err:?}"
    );
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

#[test]
fn general_lf_eob2_dc_br_ctx_mid_routes_and_roundtrips() {
    // Regression for the ctx-2 routing hole (#422): an in-scope eob==2 block whose
    // EOB AC has magnitude 3..=4 derives a NON-EOB DC coeff_br at ctx 2 (between the
    // routed ctx 1 and ctx 3). Such a block must tokenize AND roundtrip — i.e. the
    // ctx-2 CDF row must be routed — not fail with UnsupportedSelector.
    assert_eq!(
        derived_dc_br_ctx(4),
        2,
        "AC magnitude 4 derives DC coeff_br ctx 2"
    );
    // DC = -6 (non-EOB, magnitude > 4 → coeff_br at ctx 2), AC = +4 (EOB).
    let quant = dc_ac_block(-6, 4);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

/// The 4x4 2D scan raster positions for scan indices 0..=3 (`[0, 4, 1, 8, ...]`):
/// scan 0 → raster 0 (DC), scan 1 → raster 4, scan 2 → raster 1, scan 3 → raster 8.
const SCAN_RASTER_0_3: [usize; 4] = [0, 4, 1, 8];

/// Builds a signed raster `[i32; 16]` from `eob` magnitudes assigned to scan
/// positions 0..eob, with a deterministic asymmetric sign pattern (scan-even
/// negative, scan-odd positive) so a swapped sign order cannot masquerade as a
/// match. `mags[c]` is the unsigned magnitude at scan index `c`.
fn scan_block(eob: usize, mags: &[u32]) -> [i32; TX_4X4_COEFF_COUNT] {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    for (c, &mag) in mags.iter().enumerate().take(eob) {
        if mag == 0 {
            continue;
        }
        let raster = SCAN_RASTER_0_3[c];
        // Asymmetric: even scan index negative, odd positive.
        let value = if c % 2 == 0 {
            -(mag as i32)
        } else {
            mag as i32
        };
        quant[raster] = value;
    }
    quant
}

#[test]
fn general_lf_eob3_all_mag1_exact_stream() {
    // eob == 3, all three coefficients magnitude 1. Scan positions raster [0, 4, 1].
    // Signs: scan 0 (DC) negative, scan 1 (AC raster 4) positive, scan 2 (AC raster 1)
    // negative.
    let quant = scan_block(3, &[1, 1, 1]);
    assert_eq!(quant[0], -1);
    assert_eq!(quant[4], 1);
    assert_eq!(quant[1], -1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // all_zero(0), eob_pt_16(2), eob_extra(0), then the reverse-scan base pass over
    // c = 2,1,0: scan-2 EOB coeff_base_eob, scan-1 coeff_base, DC coeff_base; then the
    // reverse-scan sign pass (scan-2 bypass, scan-1 bypass, DC dc_sign); then U/V
    // all_zero(1). No coeff_br (all magnitudes 1 < the base tier).
    // = 3 header + 3 base + 3 sign + 2 chroma = 11 tokens.
    assert_eq!(trace.len(), 11);

    // eob_pt_16 symbol 2 (eobPt 3).
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 2),
        "eob_pt_16 symbol 2 (eobPt 3)"
    );
    // eob_extra flag 0 (eob 3 = 3 + 0).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0 (eob 3)"
    );
    // Scan-2 EOB coeff_base_eob at coeff_base_eob_ctx(c=2) = 1, level 1 → symbol 0.
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && c.symbol() == 0),
        "scan-2 coeff_base_eob at ctx 1 (coeff_base_eob_ctx(c=2)), symbol 0"
    );

    // Full ordered symbol stream:
    // all_zero(0), eob_pt(2), eob_extra(0),
    // base: scan-2 eob(0), scan-1 base(1), DC base(1),
    // sign: scan-2 bypass(neg→1), scan-1 bypass(pos→0), DC dc_sign(neg→1),
    // chroma U(1), V(1).
    assert_eq!(
        trace.iter().map(|t| t.symbol()).collect::<Vec<_>>(),
        vec![0, 2, 0, 0, 1, 1, 1, 0, 1, 1, 1]
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob4_all_mag1_exact_stream() {
    // eob == 4, all four coefficients magnitude 1. Scan positions raster [0, 4, 1, 8].
    let quant = scan_block(4, &[1, 1, 1, 1]);
    assert_eq!(quant[0], -1);
    assert_eq!(quant[4], 1);
    assert_eq!(quant[1], -1);
    assert_eq!(quant[8], 1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // 3 header + 4 base + 4 sign + 2 chroma = 13 tokens.
    assert_eq!(trace.len(), 13);

    // eob_pt_16 symbol 2 (eobPt 3).
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 2),
        "eob_pt_16 symbol 2 (eobPt 3)"
    );
    // eob_extra flag 1 (eob 4 = 3 + 1).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 1),
        "eob_extra flag 1 (eob 4)"
    );
    // Scan-3 EOB coeff_base_eob at coeff_base_eob_ctx(c=3) = 2, level 1 → symbol 0.
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 2, .. })
                && c.symbol() == 0),
        "scan-3 coeff_base_eob at ctx 2 (coeff_base_eob_ctx(c=3)), symbol 0"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

/// EXHAUSTIVE routing fuzz: enumerate every in-scope eob 3..=4 block over a
/// magnitude tier set chosen to hit the base/`coeff_br` boundaries ({0,1,4,5,7}),
/// with the eob-1 (EOB) position forced nonzero and all lower positions free
/// (including 0). Every such block MUST tokenize, roundtrip through
/// `roundtrip_block_symbol_trace` (so every reached `coeff_base`/`coeff_base_eob`/
/// `coeff_br`/`eob_extra` CDF context is routed), and recover via
/// `recover_quant_from_tokens` exactly. This is how the complete reachable context
/// set is discovered — an unrouted context fails `roundtrip_block_symbol_trace`
/// with `BlockSymbolTraceUnsupportedSelector` rather than producing wrong output.
#[test]
fn general_lf_eob3_4_exhaustive_routing_fuzz() {
    // {0, 1, 4, 5, 7}: 0 (zero non-EOB), 1 (base low), 4 (base max, no coeff_br),
    // 5 (first coeff_br), 7 (coeff_br max). The EOB position never takes 0.
    const TIERS: [u32; 5] = [0, 1, 4, 5, 7];
    const NONZERO_TIERS: [u32; 4] = [1, 4, 5, 7];

    let mut covered = 0usize;
    for eob in 3..=4usize {
        // The EOB coefficient (scan index eob-1) must be nonzero.
        for &eob_mag in &NONZERO_TIERS {
            // Lower positions (scan 0..eob-1) range over the full tier set (incl. 0).
            let lower = eob - 1;
            let combos = TIERS.len().pow(lower as u32);
            for combo in 0..combos {
                let mut mags = [0u32; 4];
                let mut rem = combo;
                for slot in mags.iter_mut().take(lower) {
                    *slot = TIERS[rem % TIERS.len()];
                    rem /= TIERS.len();
                }
                mags[eob - 1] = eob_mag;

                let quant = scan_block(eob, &mags);
                let trace = tokenize_general_lf_luma_block(&quant, Q_CTX);
                assert!(
                    trace.is_ok(),
                    "tokenize failed eob {eob} mags {mags:?}: {trace:?}"
                );
                let trace = trace.unwrap();
                // An unrouted CDF context surfaces here as
                // `BlockSymbolTraceUnsupportedSelector` rather than a wrong hash.
                let proof = roundtrip_block_symbol_trace(&trace);
                assert!(
                    proof.is_ok(),
                    "roundtrip failed (unrouted ctx?) eob {eob} mags {mags:?}: {proof:?}"
                );
                let proof = proof.unwrap();
                assert!(
                    !proof.bytes().is_empty(),
                    "empty proof eob {eob} mags {mags:?}"
                );
                let recovered = recover_quant_from_tokens(&trace, Q_CTX);
                assert!(
                    recovered.is_ok(),
                    "recover failed eob {eob} mags {mags:?}: {recovered:?}"
                );
                assert_eq!(
                    recovered.unwrap(),
                    quant,
                    "recover != input eob {eob} mags {mags:?}"
                );
                covered += 1;
            }
        }
    }
    // eob 3: 4 eob-mags * 5^2 lower = 100; eob 4: 4 * 5^3 = 500. Total 600.
    assert_eq!(covered, 600, "expected 600 enumerated in-scope blocks");
}
