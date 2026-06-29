// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 8.2 SELF-CONSISTENCY roundtrip tests for the FULL-range general 16x16 DCT_DCT luma
//! tokenizer (`ENC-COEFF-TOKENIZE-16X16-REFINE`): the WHOLE `eob_pt_256` range (eob
//! `1..=256`, eobPt `1..=9`), extending the base pass (eob `1..=32`) with the symbol-7
//! `eob_pt_extra` refinement for eobPt 8 (eob 65..=128) and eobPt 9 (eob 129..=256).
//!
//! These tests assert the EOB header token sequence BY HAND — the `eob_pt_256` symbol,
//! the `eob_pt_extra` bypass bit (eobPt 8/9 only), the `eob_extra` CDF flag, and the
//! `eob_extra_bit` literals — mirroring the decoder `read_nonzero_coeff_eob` /
//! `resolved_eob_pt` read order, then prove the block through the § 8.2
//! self-consistency roundtrip and recovery.
//!
//! ASYMMETRIC values + mixed signs: the EOB coefficient sits at a high scan index
//! (odd → positive per the sign convention), the DC and an LF coefficient differ in
//! magnitude/sign, so a swapped sign order or a level/position transposition cannot
//! masquerade as a match (`exit_symbol` only checks the bit COUNT, not the
//! value/position — the decode-verify-asymmetric-values lesson).
//!
//! HONESTY: this is AV2 § 8.2 SELF-CONSISTENCY — the same code authored the emission and
//! its inverse, so it proves the emitted (level, sign, position) triples are internally
//! reversible and that every reached § 8.3.2 context routes to a real generated default
//! row. It does NOT validate the § 8.3.2 CDF contexts against a real decoder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::test_support::{COEFF_COUNT, Q_CTX, block_from};
use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{CoefficientTokenSyntax, recover_quant_from_tokens_geom};
use crate::error::Error;

/// Tokenizes a 16x16 block through the FULL-range entry, proves it through the § 8.2
/// self-consistency block-symbol roundtrip, and asserts the recovery reproduces the
/// input exactly. An unrouted § 8.3.2 context surfaces here as a routing failure, NOT
/// a wrong hash.
fn assert_full_roundtrips(quant: &[i32; COEFF_COUNT]) -> Vec<BlockSymbolToken> {
    let trace = tokenize_general_16x16_luma_block_full(quant, Q_CTX);
    assert!(trace.is_ok(), "tokenize failed: {trace:?}");
    let trace = trace.unwrap();
    let proof = roundtrip_block_symbol_trace(&trace);
    assert!(
        proof.is_ok(),
        "roundtrip failed (unrouted 16x16 ctx?): {proof:?}"
    );
    assert!(!proof.unwrap().bytes().is_empty(), "empty proof");
    let recovered = recover_quant_from_tokens_geom(&trace, TxGeom::TX_16X16, Q_CTX);
    assert!(recovered.is_ok(), "recover failed: {recovered:?}");
    assert_eq!(recovered.unwrap().as_slice(), quant.as_slice());
    trace
}

/// Returns the EOB-header tokens after the leading `all_zero` token, up to (but not
/// including) the first `coeff_base_eob` base-pass token: the `eob_pt_256` symbol, the
/// optional `eob_pt_extra` bypass bit, the `eob_extra` CDF flag, and the
/// `eob_extra_bit` bypass literals — the exact token window the decoder
/// `read_nonzero_coeff_eob` reads. Element 0 is always the `eob_pt_256` symbol token.
fn eob_header_after_all_zero(trace: &[BlockSymbolToken]) -> Vec<BlockSymbolToken> {
    trace
        .iter()
        .skip(1)
        .take_while(|t| {
            !matches!(t, BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBaseEob))
        })
        .copied()
        .collect()
}

/// Asserts a token is an `eob_pt_256` symbol token with the given symbol.
fn assert_eob_pt_256(token: BlockSymbolToken, symbol: u8) {
    assert!(
        matches!(token, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256) && c.symbol() == symbol),
        "expected eob_pt_256 symbol {symbol}, got {token:?}"
    );
}

/// Asserts a token is a width-1 bypass literal with the given value.
fn assert_bypass_bit(token: BlockSymbolToken, value: u32) {
    assert!(
        matches!(token, BlockSymbolToken::Bypass { width: 1, value: v } if v == value),
        "expected bypass bit {value}, got {token:?}"
    );
}

/// Asserts a token is an `eob_extra` CDF flag with the given symbol.
fn assert_eob_extra(token: BlockSymbolToken, symbol: u8) {
    assert!(
        matches!(token, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == symbol),
        "expected eob_extra symbol {symbol}, got {token:?}"
    );
}

/// Collects the values of the width-1 bypass literals in `tokens`, asserting EVERY
/// token in the slice is such a literal (the `eob_extra_bit` trailer is pure bypass —
/// any non-bypass token there is an emission bug). Returns them in emission (MSB-first)
/// order.
fn eob_extra_bits(tokens: &[BlockSymbolToken]) -> Vec<u32> {
    for token in tokens {
        assert!(
            matches!(token, BlockSymbolToken::Bypass { width: 1, .. }),
            "expected an eob_extra_bit bypass literal, got {token:?}"
        );
    }
    tokens
        .iter()
        .filter_map(|t| match t {
            BlockSymbolToken::Bypass { width: 1, value } => Some(*value),
            _ => None,
        })
        .collect()
}

#[test]
fn base_pass_eob_32_unchanged_by_full_entry() {
    let pairs = [(0usize, 4u32), (5, 1), (17, 2), (31, 3)];
    let quant = block_from(&pairs);
    let full = tokenize_general_16x16_luma_block_full(&quant, Q_CTX).unwrap();
    let base = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(full, base, "FULL entry must match base entry for eob <= 32");
    assert_full_roundtrips(&quant);
}

#[test]
fn eob_pt_7_eob_48_uses_symbol_6_no_eob_pt_extra() {
    let quant = block_from(&[(0, 5), (3, 2), (47, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 6);
    assert_eob_extra(header[1], 0); // high refinement bit 0
    let bits = eob_extra_bits(&header[2..]);
    assert_eq!(bits, vec![1, 1, 1, 1], "eobPt 7 → 4 eob_extra_bit literals");
    assert_eq!(header.len(), 6);
}

#[test]
fn eob_pt_8_eob_96_uses_symbol_7_eob_pt_extra_0() {
    let quant = block_from(&[(0, 6), (4, 3), (95, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 7); // both eobPt 8 and 9 use symbol 7
    assert_bypass_bit(header[1], 0); // eob_pt_extra 0 → eobPt 8 (= 8 + 0)
    assert_eob_extra(header[2], 0); // high refinement bit 0
    let bits = eob_extra_bits(&header[3..]);
    assert_eq!(
        bits,
        vec![1, 1, 1, 1, 1],
        "eobPt 8 → 5 eob_extra_bit literals"
    );
    assert_eq!(header.len(), 8);
}

#[test]
fn eob_pt_9_eob_200_uses_symbol_7_eob_pt_extra_1() {
    let quant = block_from(&[(0, 7), (5, 2), (199, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 7); // symbol 7 (shared by eobPt 8 and 9)
    assert_bypass_bit(header[1], 1); // eob_pt_extra 1 → eobPt 9 (= 8 + 1)
    assert_eob_extra(header[2], 1); // high refinement bit 1
    let bits = eob_extra_bits(&header[3..]);
    assert_eq!(
        bits,
        vec![0, 0, 0, 1, 1, 1],
        "eobPt 9 → 6 eob_extra_bit literals"
    );
    assert_eq!(header.len(), 9);
}

#[test]
fn near_full_eob_256_uses_symbol_7_eob_pt_extra_1() {
    let quant = block_from(&[(0, 4), (2, 3), (255, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 7);
    assert_bypass_bit(header[1], 1); // eob_pt_extra 1 → eobPt 9
    assert_eob_extra(header[2], 1);
    let bits = eob_extra_bits(&header[3..]);
    assert_eq!(
        bits,
        vec![1, 1, 1, 1, 1, 1],
        "eob 256 → six 1 eob_extra_bits"
    );
    assert_eq!(header.len(), 9);
}

#[test]
fn eob_pt_extra_bit_position_is_between_symbol_and_eob_extra() {
    let quant = block_from(&[(0, 6), (95, 1)]);
    let trace = tokenize_general_16x16_luma_block_full(&quant, Q_CTX).unwrap();
    let header = eob_header_after_all_zero(&trace);
    assert!(matches!(header[0], BlockSymbolToken::Coeff(c)
        if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256)));
    assert!(
        matches!(header[1], BlockSymbolToken::Bypass { width: 1, .. }),
        "eob_pt_extra bypass bit must directly follow the eob_pt_256 symbol"
    );
    assert!(
        matches!(header[2], BlockSymbolToken::Coeff(c)
        if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra)),
        "the eob_extra CDF flag must directly follow the eob_pt_extra bypass bit"
    );
}

#[test]
fn eobpt_8_and_9_share_symbol_7_distinguished_only_by_eob_pt_extra() {
    let eob_pt_8 =
        tokenize_general_16x16_luma_block_full(&block_from(&[(0, 5), (95, 1)]), Q_CTX).unwrap();
    let eob_pt_9 =
        tokenize_general_16x16_luma_block_full(&block_from(&[(0, 5), (199, 1)]), Q_CTX).unwrap();
    let h8 = eob_header_after_all_zero(&eob_pt_8);
    let h9 = eob_header_after_all_zero(&eob_pt_9);
    assert_eob_pt_256(h8[0], 7);
    assert_eob_pt_256(h9[0], 7);
    assert_bypass_bit(h8[1], 0);
    assert_bypass_bit(h9[1], 1);
}

#[test]
fn full_range_admits_the_whole_1_to_256_window() {
    let quant = block_from(&[(0, 1), (255, 1)]);
    let trace = tokenize_general_16x16_luma_block_full(&quant, Q_CTX);
    assert!(trace.is_ok(), "eob 256 must be accepted: {trace:?}");
}

#[test]
fn golomb_magnitude_overflow_still_rejected_at_full_range() {
    let mut quant = block_from(&[(199, 1)]);
    quant[0] = i32::MAX;
    let err = tokenize_general_16x16_luma_block_full(&quant, Q_CTX).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude { .. }
        ),
        "an over-cap golomb magnitude must be rejected; got {err:?}"
    );
}
