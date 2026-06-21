// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tests for the private encoder block-symbol trace composition
//! (the `block_symbol_trace` module), split out to keep each file under the
//! 1000-line source budget.

use super::*;
use crate::intra_mode_emission::{
    IntraModeCdfRowSelector, IntraModeSyntax, roundtrip_intra_mode_tokens,
};

#[test]
fn composes_ordered_mode_info_prefix() {
    let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();

    assert_eq!(trace.len(), 3);
    assert_eq!(trace[0].syntax(), IntraModeSyntax::YModeSet);
    assert_eq!(trace[1].syntax(), IntraModeSyntax::YModeIndex);
    assert_eq!(trace[2].syntax(), IntraModeSyntax::UvMode);
    assert!(matches!(
        trace[0].selector(),
        IntraModeCdfRowSelector::YModeSet
    ));
    assert!(matches!(
        trace[1].selector(),
        IntraModeCdfRowSelector::YModeIndex { ctx: 0 }
    ));
    assert!(matches!(
        trace[2].selector(),
        IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx: 0 }
    ));
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0]
    );
}

#[test]
fn composed_trace_matches_concatenated_emitters() {
    let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
    let luma = emit_minimal_dc_luma_intra_mode().unwrap();
    let uv = emit_minimal_dc_chroma_uv_mode().unwrap();

    let mut expected = luma.tokens().to_vec();
    expected.extend_from_slice(uv.tokens());
    assert_eq!(trace, expected);
}

#[test]
fn composed_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
    let proof = roundtrip_intra_mode_tokens(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0]);
    assert_eq!(proof.symbol_count(), 3);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
    let first = roundtrip_intra_mode_tokens(&trace).unwrap();
    let second = roundtrip_intra_mode_tokens(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_all_zero_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();

    assert_eq!(trace.len(), 4);
    assert!(matches!(trace[0], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[3], BlockSymbolToken::Coeff(_)));
    if let BlockSymbolToken::Mode(token) = trace[0] {
        assert_eq!(token.syntax(), IntraModeSyntax::YModeSet);
    }
    if let BlockSymbolToken::Mode(token) = trace[2] {
        assert_eq!(token.syntax(), IntraModeSyntax::UvMode);
    }
    // y_mode_set=0, y_mode_index=0, uv_mode=0, luma all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 1]
    );
}

#[test]
fn unified_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1]);
    assert_eq!(proof.symbol_count(), 4);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn unified_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn rejects_unsupported_unified_selector() {
    // A luma txb_skip token at a non-minimal coefficient CDF q-context is
    // outside the unified router's supported rows.
    let unsupported = BlockSymbolToken::Coeff(luma_all_zero_token(1));
    let err = roundtrip_block_symbol_trace(&[unsupported]).unwrap_err();

    assert!(matches!(
        err,
        Error::BlockSymbolTraceUnsupportedSelector { index: 0 }
    ));
}

#[test]
fn composes_complete_all_zero_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();

    assert_eq!(trace.len(), 6);
    // Mode prefix, then per-plane all_zero (Y, U, V) in residual() order.
    assert!(matches!(trace[0], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[3], BlockSymbolToken::Coeff(_)));
    assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
    assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
    // y_mode_set=0, y_mode_index=0, uv_mode=0, then luma/U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 1, 1, 1]
    );
}

#[test]
fn complete_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1, 1, 1]);
    assert_eq!(proof.symbol_count(), 6);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn complete_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_coded_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();

    assert_eq!(trace.len(), 9);
    // Mode prefix (3), then coded luma residual (txb_skip, eob_pt_16,
    // coeff_base_eob, dc_sign), then all-zero U and V txb_skip.
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..9] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // y_mode_set=0, y_mode_index=0, uv_mode=0, luma txb_skip=0 (coded),
    // eob_pt_16=0, coeff_base_eob=0 (mag 1), dc_sign=0 (positive),
    // then U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 0, 0, 1, 1]
    );
}

#[test]
fn coded_block_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 0, 0, 1, 1]);
    assert_eq!(proof.symbol_count(), 9);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn coded_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_coded_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_br_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_br_block_trace().unwrap();

    assert_eq!(trace.len(), 10);
    // Mode prefix (3), coded luma residual with coeff_br (5), U/V txb_skip (2).
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..10] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // y_mode_set=0, y_mode_index=0, uv_mode=0, txb_skip=0, eob_pt_16=0,
    // coeff_base_eob=4 (level 5), coeff_br=1 (magnitude 6), dc_sign=0,
    // then U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 4, 1, 0, 1, 1]
    );
}

#[test]
fn br_block_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_br_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 4, 1, 0, 1, 1]);
    assert_eq!(proof.symbol_count(), 10);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn br_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_br_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn bypass_literals_interleave_with_cdf_symbols() {
    // §8.2.5 bypass literals (the foundation for non-luma-DC `sign_bit` and
    // the golomb tail) must roundtrip bit-exactly through the same coder that
    // carries the CDF symbols.
    let mut trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    trace.push(BlockSymbolToken::bypass(1, 1)); // a 1-bit sign-like literal
    trace.push(BlockSymbolToken::bypass(4, 13)); // a wider literal
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    // The all-zero block symbols, then the two bypass values (value-as-u8).
    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1, 1, 1, 1, 13]);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn bypass_literal_roundtrip_is_deterministic() {
    let mut trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
    trace.push(BlockSymbolToken::bypass(3, 5));
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn wide_bypass_literals_roundtrip_full_width() {
    // Literals wider than 8 bits (the golomb tail uses up to L(32)) must
    // roundtrip their FULL value: the budget scales with the bit width, and
    // the full-width check rejects truncation — so `roundtrip_block_symbol_trace`
    // returning Ok proves the exact value was reproduced, not just its low byte.
    let mut trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
    trace.push(BlockSymbolToken::bypass(16, 0x1234));
    trace.push(BlockSymbolToken::bypass(32, 0xDEAD_BEEF));
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert!(!proof.bytes().is_empty());
    // The u8 view truncates the wide values to their low byte; the Ok above is
    // the full-width proof.
    assert_eq!(proof.decoded_symbols().last(), Some(&0xEFu8));
}

#[test]
fn composes_coded_chroma_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_coded_chroma_block_trace().unwrap();

    assert_eq!(trace.len(), 12);
    // Mode prefix (3), coded luma residual (4 CDF), coded U residual (3 CDF +
    // 1 bypass sign), V all-zero txb_skip (1 CDF).
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..10] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // The U DC sign is a bypass literal, not a CDF symbol.
    assert!(matches!(
        trace[10],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    assert!(matches!(trace[11], BlockSymbolToken::Coeff(_)));
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
    );
}

#[test]
fn coded_chroma_block_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_dc_coded_chroma_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(
        proof.decoded_symbols(),
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert!(!proof.bytes().is_empty());
}

#[test]
fn coded_chroma_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_coded_chroma_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_golomb_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_golomb_block_trace().unwrap();

    assert_eq!(trace.len(), 13);
    // Mode prefix (3), luma level tokens (4 CDF), dc_sign (1 CDF), golomb
    // bypass (3), U/V all-zero (2 CDF). Per §5.20.7.27 the sign precedes the
    // §5.20.7.28 read_quant golomb bits.
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..8] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // The golomb `coeff_rem` bits are §8.2.5 bypass literals, after dc_sign.
    for token in &trace[8..11] {
        assert!(matches!(token, BlockSymbolToken::Bypass { width: 1, .. }));
    }
    for token in &trace[11..13] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // modes; txb_skip=0, eob_pt=0, coeff_base_eob=4, coeff_br=3; dc_sign=0;
    // golomb q_length 0,1 + coeff_rem 0; U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 4, 3, 0, 0, 1, 0, 1, 1]
    );
}

#[test]
fn golomb_block_trace_roundtrips_and_decodes_to_magnitude() {
    let trace = compose_minimal_intra_dc_golomb_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    let decoded = proof.decoded_symbols();

    assert_eq!(decoded, &[0, 0, 0, 0, 0, 4, 3, 0, 0, 1, 0, 1, 1]);
    assert!(!proof.bytes().is_empty());

    // Reconstruct the golomb extension `x` from the decoded bypass bits the
    // way the decoder's read_quant finite-q path does, and confirm it yields
    // the encoded magnitude (this is the conformance check, since the
    // roundtrip alone only proves the bits are self-consistent). The golomb
    // bits follow the dc_sign at index 7, so they are at indices 8..11.
    let golomb = &decoded[8..11];
    let mut q = 0u32;
    let mut idx = 0;
    while idx < golomb.len() && golomb[idx] == 0 {
        q += 1;
        idx += 1;
    }
    // idx now points at the terminating q_length_bit (1); coeff_rem follows.
    let coeff_rem = u32::from(golomb[idx + 1]);
    let x = (q << GOLOMB_DC_M) + coeff_rem;
    assert_eq!(
        GOLOMB_MAXLEVEL + x,
        MINIMAL_GOLOMB_DC_MAGNITUDE,
        "golomb bits decode to the encoded magnitude"
    );
}

#[test]
fn golomb_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_golomb_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn golomb_block_trace_rejects_out_of_finite_q_range() {
    // A parameterized compose returning Result must reject magnitudes outside the
    // finite-q range at runtime (not via a release-stripped debug_assert): below
    // maxLevel (8) and at/above maxLevel+10 (18, the golomb-prefix path) both yield
    // a typed error, not a non-conformant trace.
    for magnitude in [0, 7, GOLOMB_FINITE_Q_MAGNITUDE_MAX + 1, 100] {
        let err = compose_intra_dc_golomb_block_trace(magnitude, false).unwrap_err();
        assert!(matches!(
            err,
            Error::BlockSymbolTraceGolombMagnitudeOutOfRange {
                magnitude: m,
                min: GOLOMB_MAXLEVEL,
                max: GOLOMB_FINITE_Q_MAGNITUDE_MAX,
            } if m == magnitude
        ));
    }
    // The boundary values are accepted.
    assert!(compose_intra_dc_golomb_block_trace(GOLOMB_MAXLEVEL, false).is_ok());
    assert!(compose_intra_dc_golomb_block_trace(GOLOMB_FINITE_Q_MAGNITUDE_MAX, false).is_ok());
}

#[test]
fn golomb_block_trace_covers_finite_q_range() {
    // Every finite-q magnitude (maxLevel..=maxLevel+9 = 8..=17) composes a trace
    // that roundtrips through one §8.2 coder and whose decoded golomb bits
    // reconstruct that magnitude via the decoder's read_quant finite-q arithmetic.
    // The level/sign prefix is identical across the tier; only the golomb
    // q_length/coeff_rem bits vary, so this proves the whole claimed range, not
    // just the canonical +10 case.
    for magnitude in GOLOMB_MAXLEVEL..=GOLOMB_FINITE_Q_MAGNITUDE_MAX {
        let trace = compose_intra_dc_golomb_block_trace(magnitude, false).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        let decoded = proof.decoded_symbols();
        assert!(!proof.bytes().is_empty());

        // Fixed prefix: 3 modes, txb_skip=0, eob_pt=0, coeff_base_eob=4,
        // coeff_br=3, dc_sign=0. The golomb bits start at index 8.
        assert_eq!(&decoded[0..8], &[0, 0, 0, 0, 0, 4, 3, 0]);
        let golomb = &decoded[8..];
        let mut q = 0u32;
        let mut idx = 0;
        while idx < golomb.len() && golomb[idx] == 0 {
            q += 1;
            idx += 1;
        }
        // idx points at the terminating q_length_bit (1); coeff_rem follows. The
        // trailing U/V all_zero=1 symbols are never reached (the loop stops at the
        // terminating 1).
        let coeff_rem = u32::from(golomb[idx + 1]);
        let x = (q << GOLOMB_DC_M) + coeff_rem;
        assert_eq!(
            GOLOMB_MAXLEVEL + x,
            magnitude,
            "golomb bits reconstruct the encoded magnitude across the finite-q range"
        );
    }
}

// Reconstructs the encoded magnitude from a decoded golomb-*prefix* trace the way
// the decoder's read_quant golomb-prefix path does. Returns the magnitude.
fn reconstruct_golomb_prefix_magnitude(decoded: &[u8]) -> u32 {
    // After 3 modes + 4 level + dc_sign the golomb bits start at index 8.
    let after_sign = &decoded[8..];
    // The first GOLOMB_PREFIX_Q_ZEROS bits are q_length zeros (q == cMax).
    let qz = GOLOMB_PREFIX_Q_ZEROS as usize;
    assert!(
        after_sign[..qz].iter().all(|&b| b == 0),
        "q_length is cMax zeros"
    );
    let golomb = &after_sign[qz..];
    // golomb_length unary: count zeros up to the terminating 1.
    let mut gz = 0usize;
    while golomb[gz] == 0 {
        gz += 1;
    }
    let length = gz as u32 + GOLOMB_DC_K;
    let coeff_rem = u32::from(golomb[gz + 1]); // the L(length) literal value
    let x = GOLOMB_PREFIX_XBASE_BIAS + (1 << length) + coeff_rem;
    GOLOMB_MAXLEVEL + x
}

#[test]
fn composes_golomb_prefix_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_golomb_prefix_block_trace().unwrap();

    assert_eq!(trace.len(), 17);
    // Mode prefix (3), level tokens + dc_sign (5 CDF), q_length + golomb_length
    // (6 width-1 bypass), coeff_rem (1 width-2 bypass), U/V all-zero (2 CDF).
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..8] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    for token in &trace[8..14] {
        assert!(matches!(token, BlockSymbolToken::Bypass { width: 1, .. }));
    }
    assert!(matches!(
        trace[14],
        BlockSymbolToken::Bypass { width: 2, value: 0 }
    ));
    for token in &trace[15..17] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // modes; txb_skip=0, eob_pt=0, coeff_base_eob=4, coeff_br=3; dc_sign=0;
    // 5 q_length zeros; golomb_length 1 (0 zeros); coeff_rem 0; U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1]
    );
}

#[test]
fn golomb_prefix_block_trace_roundtrips_and_decodes_to_magnitude() {
    let trace = compose_minimal_intra_dc_golomb_prefix_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    let decoded = proof.decoded_symbols();

    assert_eq!(
        decoded,
        &[0, 0, 0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1]
    );
    assert!(!proof.bytes().is_empty());
    assert_eq!(
        reconstruct_golomb_prefix_magnitude(decoded),
        MINIMAL_GOLOMB_PREFIX_DC_MAGNITUDE
    );
}

#[test]
fn golomb_prefix_block_trace_covers_supported_range() {
    // Every supported golomb-prefix magnitude (18..=525, golomb length 2..=8)
    // composes a trace that roundtrips through one §8.2 coder and whose decoded
    // golomb-prefix bits reconstruct that magnitude via the decoder's read_quant
    // golomb-prefix arithmetic.
    for magnitude in GOLOMB_PREFIX_MAGNITUDE_MIN..=GOLOMB_PREFIX_MAGNITUDE_MAX {
        let trace = compose_intra_dc_golomb_prefix_block_trace(magnitude, false).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert!(!proof.bytes().is_empty());
        // Fixed prefix: 3 modes, txb_skip=0, eob_pt=0, coeff_base_eob=4,
        // coeff_br=3, dc_sign=0.
        assert_eq!(&proof.decoded_symbols()[0..8], &[0, 0, 0, 0, 0, 4, 3, 0]);
        assert_eq!(
            reconstruct_golomb_prefix_magnitude(proof.decoded_symbols()),
            magnitude,
            "golomb-prefix bits reconstruct the encoded magnitude across the range"
        );
    }
}

#[test]
fn golomb_prefix_block_trace_rejects_out_of_range() {
    // Below the minimum (finite-q range) and above the cap (wider coeff_rem, a
    // later brick) yield a typed runtime error, not a non-conformant trace.
    for magnitude in [
        GOLOMB_PREFIX_MAGNITUDE_MIN - 1,
        GOLOMB_PREFIX_MAGNITUDE_MAX + 1,
    ] {
        let err = compose_intra_dc_golomb_prefix_block_trace(magnitude, false).unwrap_err();
        assert!(matches!(
            err,
            Error::BlockSymbolTraceGolombMagnitudeOutOfRange {
                magnitude: m,
                min: GOLOMB_PREFIX_MAGNITUDE_MIN,
                max: GOLOMB_PREFIX_MAGNITUDE_MAX,
            } if m == magnitude
        ));
    }
    assert!(compose_intra_dc_golomb_prefix_block_trace(GOLOMB_PREFIX_MAGNITUDE_MIN, false).is_ok());
    assert!(compose_intra_dc_golomb_prefix_block_trace(GOLOMB_PREFIX_MAGNITUDE_MAX, false).is_ok());
}

#[test]
fn golomb_prefix_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_dc_golomb_prefix_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_two_coeff_block_trace_in_order() {
    let trace = compose_minimal_intra_two_coeff_block_trace().unwrap();

    assert_eq!(trace.len(), 10);
    // Mode prefix (3); coded all_zero, eob_pt_16, AC coeff_base_eob, DC coeff_base
    // (4 CDF); AC sign_bit (1 bypass); U/V all-zero (2 CDF).
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..7] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    assert!(matches!(
        trace[7],
        BlockSymbolToken::Bypass { width: 1, value: 0 }
    ));
    for token in &trace[8..10] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    // The AC coeff_base_eob is at context 1; the DC coeff_base is at the DERIVED
    // low-frequency context 1 (proving the composer used coeff_base_lf_luma_context,
    // not a hard-coded literal).
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(ac)
            if matches!(ac.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })),
        "AC coeff_base_eob at context 1"
    );
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(dc)
            if matches!(dc.selector(), CoefficientCdfRowSelector::CoeffBaseLf { ctx: 1, tcq_ctx: 0, .. })),
        "DC coeff_base at derived low-frequency context 1"
    );
    // modes; all_zero=0, eob_pt_16=1 (eob 2), AC coeff_base_eob=0 (level 1), DC
    // coeff_base=0 (level 0); AC sign_bit=0; U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 0, 0, 0, 1, 1]
    );
}

#[test]
fn eob2_ac_scan_index_maps_to_raster_four() {
    // The composer must seed the AC's Level[] at its scan-derived raster position,
    // not at the scan index: in the 4x4 2D scan order `[0, 4, 1, ...]`, scan index
    // 1 maps to raster position 4 (row 1, col 0), not raster 1.
    let mut scan = [0u16; TX_4X4_WIDTH * TX_4X4_HEIGHT];
    coefficient_scan_order(TX_4X4_WIDTH, TX_4X4_HEIGHT, TransformClass::TwoD, &mut scan).unwrap();
    assert_eq!(scan[EOB2_AC_SCAN_INDEX], 4);
}

#[test]
fn two_coeff_block_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_two_coeff_block_trace().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 0, 0, 0, 1, 1]);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn two_coeff_block_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_two_coeff_block_trace().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_two_coeff_with_tx_type_inserts_intra_tx_type_after_eob_pt() {
    let base = compose_minimal_intra_two_coeff_block_trace().unwrap();
    let trace = compose_minimal_intra_two_coeff_block_trace_with_tx_type().unwrap();

    // The tx-type trace is the eob=2 trace plus one intra_tx_type token after the
    // eob_pt_16 token (index 4).
    assert_eq!(trace.len(), base.len() + 1);
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(t)
            if matches!(t.selector(), CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr: 0 })),
        "intra_tx_type (DCT_DCT) after eob_pt"
    );
    // modes; all_zero=0, eob_pt_16=1, intra_tx_type=0 (DCT_DCT), AC coeff_base_eob=0,
    // DC coeff_base=0, AC sign_bit=0, U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1]
    );
}

#[test]
fn two_coeff_with_tx_type_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_two_coeff_block_trace_with_tx_type().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1]);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn two_coeff_with_tx_type_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_two_coeff_block_trace_with_tx_type().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn composes_ist_trace_inserts_sec_tx_type_after_intra_tx_type() {
    let base = compose_minimal_intra_two_coeff_block_trace_with_tx_type().unwrap();
    let trace = compose_minimal_intra_two_coeff_block_trace_with_ist().unwrap();

    // The IST trace is the tx-type trace plus one sec_tx_type token right after the
    // intra_tx_type token (index 5).
    assert_eq!(trace.len(), base.len() + 1);
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(t)
            if matches!(t.selector(), CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr: 0 })),
        "intra_tx_type at index 5"
    );
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(t)
            if matches!(t.selector(), CoefficientCdfRowSelector::SecTxTypeIntra { tx_size_sqr: 0 })),
        "sec_tx_type (IST off) right after intra_tx_type"
    );
    // modes; all_zero=0, eob_pt_16=1, intra_tx_type=0, sec_tx_type=0, AC coeff_base_eob=0,
    // DC coeff_base=0, AC sign_bit=0, U/V all_zero=1.
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1]
    );
}

#[test]
fn ist_trace_roundtrips_through_one_coder() {
    let trace = compose_minimal_intra_two_coeff_block_trace_with_ist().unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(
        proof.decoded_symbols(),
        &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1]
    );
    assert!(!proof.bytes().is_empty());
}

#[test]
fn ist_trace_roundtrip_is_deterministic() {
    let trace = compose_minimal_intra_two_coeff_block_trace_with_ist().unwrap();
    let first = roundtrip_block_symbol_trace(&trace).unwrap();
    let second = roundtrip_block_symbol_trace(&trace).unwrap();

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.decoded_symbols(), second.decoded_symbols());
}

#[test]
fn encode_block_symbol_trace_emits_decodable_all_zero_payload() {
    // The production entropy-coding entry point emits the §8.2 bytes for the complete
    // all-zero intra block; they are exactly the bytes the roundtrip proves decodable.
    let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    let bytes = encode_block_symbol_trace(&trace).unwrap();
    assert!(!bytes.is_empty());

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.bytes(), bytes.as_slice());
    assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 1, 1, 1]);
}

#[test]
fn encode_block_symbol_trace_is_deterministic() {
    let trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    assert_eq!(
        encode_block_symbol_trace(&trace).unwrap(),
        encode_block_symbol_trace(&trace).unwrap()
    );
}

#[test]
fn root_do_split_none_roundtrips_as_partition_none() {
    // The §5.20.3.2 do_split=false (PARTITION_NONE) root symbol — the first symbol the
    // decoder reads on the general intra tile path — round-trips through one §8.2 coder at
    // TileDoSplitCdf[plane_start 0][ctx 12] (the ctx pinned against the q80 decode).
    let trace = vec![BlockSymbolToken::Partition(
        crate::partition_emission::emit_root_do_split_none(),
    )];
    assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0]);
    assert_eq!(proof.symbol_count(), 1);
}
