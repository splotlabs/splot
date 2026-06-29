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
    assert!(matches!(trace[0], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
    assert!(matches!(trace[3], BlockSymbolToken::Coeff(_)));
    assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
    assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
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
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..9] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
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
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..10] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
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
    let mut trace = compose_minimal_intra_dc_complete_all_zero_block_trace().unwrap();
    trace.push(BlockSymbolToken::bypass(1, 1)); // a 1-bit sign-like literal
    trace.push(BlockSymbolToken::bypass(4, 13)); // a wider literal
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

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
    let mut trace = compose_minimal_intra_dc_all_zero_block_trace().unwrap();
    trace.push(BlockSymbolToken::bypass(16, 0x1234));
    trace.push(BlockSymbolToken::bypass(32, 0xDEAD_BEEF));
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();

    assert!(!proof.bytes().is_empty());
    assert_eq!(proof.decoded_symbols().last(), Some(&0xEFu8));
}

#[test]
fn composes_coded_chroma_block_trace_in_order() {
    let trace = compose_minimal_intra_dc_coded_chroma_block_trace().unwrap();

    assert_eq!(trace.len(), 12);
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..10] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
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
    for token in &trace[0..3] {
        assert!(matches!(token, BlockSymbolToken::Mode(_)));
    }
    for token in &trace[3..8] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
    for token in &trace[8..11] {
        assert!(matches!(token, BlockSymbolToken::Bypass { width: 1, .. }));
    }
    for token in &trace[11..13] {
        assert!(matches!(token, BlockSymbolToken::Coeff(_)));
    }
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

    let golomb = &decoded[8..11];
    let mut q = 0u32;
    let mut idx = 0;
    while idx < golomb.len() && golomb[idx] == 0 {
        q += 1;
        idx += 1;
    }
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
    assert!(compose_intra_dc_golomb_block_trace(GOLOMB_MAXLEVEL, false).is_ok());
    assert!(compose_intra_dc_golomb_block_trace(GOLOMB_FINITE_Q_MAGNITUDE_MAX, false).is_ok());
}

#[test]
fn golomb_block_trace_covers_finite_q_range() {
    for magnitude in GOLOMB_MAXLEVEL..=GOLOMB_FINITE_Q_MAGNITUDE_MAX {
        let trace = compose_intra_dc_golomb_block_trace(magnitude, false).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        let decoded = proof.decoded_symbols();
        assert!(!proof.bytes().is_empty());

        assert_eq!(&decoded[0..8], &[0, 0, 0, 0, 0, 4, 3, 0]);
        let golomb = &decoded[8..];
        let mut q = 0u32;
        let mut idx = 0;
        while idx < golomb.len() && golomb[idx] == 0 {
            q += 1;
            idx += 1;
        }
        let coeff_rem = u32::from(golomb[idx + 1]);
        let x = (q << GOLOMB_DC_M) + coeff_rem;
        assert_eq!(
            GOLOMB_MAXLEVEL + x,
            magnitude,
            "golomb bits reconstruct the encoded magnitude across the finite-q range"
        );
    }
}

fn reconstruct_golomb_prefix_magnitude(decoded: &[u8]) -> u32 {
    let after_sign = &decoded[8..];
    let qz = GOLOMB_PREFIX_Q_ZEROS as usize;
    assert!(
        after_sign[..qz].iter().all(|&b| b == 0),
        "q_length is cMax zeros"
    );
    let golomb = &after_sign[qz..];
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
    for magnitude in GOLOMB_PREFIX_MAGNITUDE_MIN..=GOLOMB_PREFIX_MAGNITUDE_MAX {
        let trace = compose_intra_dc_golomb_prefix_block_trace(magnitude, false).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert!(!proof.bytes().is_empty());
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
    assert_eq!(
        trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 0, 0, 0, 1, 1]
    );
}

#[test]
fn eob2_ac_scan_index_maps_to_raster_four() {
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

    assert_eq!(trace.len(), base.len() + 1);
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(t)
            if matches!(t.selector(), CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr: 0 })),
        "intra_tx_type (DCT_DCT) after eob_pt"
    );
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
    let trace = vec![BlockSymbolToken::Partition(
        crate::partition_emission::emit_root_do_split_none(),
    )];
    assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[0]);
    assert_eq!(proof.symbol_count(), 1);
}

#[test]
fn root_partition_split_roundtrips_as_do_split_then_do_square_split() {
    let trace = vec![
        BlockSymbolToken::Partition(crate::partition_emission::emit_root_do_split_split()),
        BlockSymbolToken::Partition(crate::partition_emission::emit_root_do_square_split_square()),
    ];

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert_eq!(proof.decoded_symbols(), &[1, 1]);
    assert_eq!(proof.symbol_count(), 2);
}
