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
    // The FULL entry must be byte-identical to the base entry over the base-pass window
    // (eob 1..=32). A block reaching eob 32 (eobPt 6, symbol 5) tokenizes the SAME way
    // through both entries.
    let pairs = [(0usize, 4u32), (5, 1), (17, 2), (31, 3)];
    let quant = block_from(&pairs);
    let full = tokenize_general_16x16_luma_block_full(&quant, Q_CTX).unwrap();
    let base = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(full, base, "FULL entry must match base entry for eob <= 32");
    assert_full_roundtrips(&quant);
}

#[test]
fn eob_pt_7_eob_48_uses_symbol_6_no_eob_pt_extra() {
    // eobPt 7 (eob 33..=64, base 33): the PLAIN `eob_pt_256` symbol 6 — NO `eob_pt_extra`
    // bit (only symbol 7 carries it). eob 48 → offset 48 - 33 = 15, width eobPt - 3 = 4:
    // eob_extra (high bit) = (15 >> 4) & 1 = 0, eob_extra_bits = 15 & 15 = 0b1111 (4 bits).
    // EOB nonzero at scan index 47 (odd → positive, HF magnitude 1 to stay base-range).
    let quant = block_from(&[(0, 5), (3, 2), (47, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    // eob_pt_256 symbol 6, then DIRECTLY the eob_extra CDF flag (NO eob_pt_extra bypass).
    assert_eob_pt_256(header[0], 6);
    assert_eob_extra(header[1], 0); // high refinement bit 0
    // Exactly 4 eob_extra_bit literals MSB-first: 0b1111 = [1, 1, 1, 1].
    let bits = eob_extra_bits(&header[2..]);
    assert_eq!(bits, vec![1, 1, 1, 1], "eobPt 7 → 4 eob_extra_bit literals");
    // No eob_pt_extra bypass means the header is symbol + eob_extra + 4 bits = 6 tokens.
    assert_eq!(header.len(), 6);
}

#[test]
fn eob_pt_8_eob_96_uses_symbol_7_eob_pt_extra_0() {
    // eobPt 8 (eob 65..=128, base 65): `eob_pt_256` symbol 7 + an `eob_pt_extra` bypass
    // bit 0 + `eob_extra` + 5 `eob_extra_bit` literals. eob 96 → offset 96 - 65 = 31,
    // width eobPt - 3 = 5: eob_extra (high bit) = (31 >> 5) & 1 = 0, eob_extra_bits =
    // 31 & 31 = 0b11111 (5 bits). HAND-COMPUTED header sequence (mirroring the decoder
    // `read_nonzero_coeff_eob`): [eob_pt_256=7, eob_pt_extra=0, eob_extra=0, 1,1,1,1,1].
    // EOB nonzero at scan index 95 (odd → positive, HF magnitude 1).
    let quant = block_from(&[(0, 6), (4, 3), (95, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 7); // both eobPt 8 and 9 use symbol 7
    assert_bypass_bit(header[1], 0); // eob_pt_extra 0 → eobPt 8 (= 8 + 0)
    assert_eob_extra(header[2], 0); // high refinement bit 0
    // Exactly 5 eob_extra_bit literals MSB-first: 0b11111 = [1, 1, 1, 1, 1].
    let bits = eob_extra_bits(&header[3..]);
    assert_eq!(
        bits,
        vec![1, 1, 1, 1, 1],
        "eobPt 8 → 5 eob_extra_bit literals"
    );
    // Full header: symbol + eob_pt_extra + eob_extra + 5 bits = 8 tokens.
    assert_eq!(header.len(), 8);
}

#[test]
fn eob_pt_9_eob_200_uses_symbol_7_eob_pt_extra_1() {
    // eobPt 9 (eob 129..=256, base 129): `eob_pt_256` symbol 7 + an `eob_pt_extra` bypass
    // bit 1 + `eob_extra` + 6 `eob_extra_bit` literals. eob 200 → offset 200 - 129 = 71,
    // width eobPt - 3 = 6: eob_extra (high bit) = (71 >> 6) & 1 = 1, eob_extra_bits =
    // 71 & 63 = 7 = 0b000111 (6 bits). HAND-COMPUTED header sequence: [eob_pt_256=7,
    // eob_pt_extra=1, eob_extra=1, 0,0,0,1,1,1]. EOB nonzero at scan 199 (odd → positive).
    let quant = block_from(&[(0, 7), (5, 2), (199, 1)]);
    let trace = assert_full_roundtrips(&quant);
    let header = eob_header_after_all_zero(&trace);

    assert_eob_pt_256(header[0], 7); // symbol 7 (shared by eobPt 8 and 9)
    assert_bypass_bit(header[1], 1); // eob_pt_extra 1 → eobPt 9 (= 8 + 1)
    assert_eob_extra(header[2], 1); // high refinement bit 1
    // Exactly 6 eob_extra_bit literals MSB-first: 0b000111 = [0, 0, 0, 1, 1, 1].
    let bits = eob_extra_bits(&header[3..]);
    assert_eq!(
        bits,
        vec![0, 0, 0, 1, 1, 1],
        "eobPt 9 → 6 eob_extra_bit literals"
    );
    // Full header: symbol + eob_pt_extra + eob_extra + 6 bits = 9 tokens.
    assert_eq!(header.len(), 9);
}

#[test]
fn near_full_eob_256_uses_symbol_7_eob_pt_extra_1() {
    // eob 256 (eobPt 9, the largest a Quant[256] block can reach): base 129, offset
    // 256 - 129 = 127, width 6: eob_extra (high bit) = (127 >> 6) & 1 = 1, eob_extra_bits
    // = 127 & 63 = 63 = 0b111111 (6 ones). HAND-COMPUTED header: [eob_pt_256=7,
    // eob_pt_extra=1, eob_extra=1, 1,1,1,1,1,1]. EOB nonzero at scan 255 (odd → positive).
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
    // Guards the TOKEN ORDER that the §8.2 roundtrip cannot catch but a real decoder
    // depends on: the `eob_pt_extra` bypass bit is emitted AFTER the `eob_pt_256` symbol
    // and BEFORE the `eob_extra` CDF flag (the decoder `read_nonzero_coeff_eob` reads
    // `eob_pt_256` → `eob_pt_extra` literal → `eob_extra` CDF). For an eobPt-8 block the
    // first three header tokens MUST be exactly: EobPt256 symbol, Bypass(1), EobExtra.
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
    // The HIGHEST-RISK invariant: eobPt 8 and eobPt 9 emit the SAME `eob_pt_256` symbol
    // (7); ONLY the `eob_pt_extra` bypass bit (0 vs 1) distinguishes them. A wrong symbol
    // (e.g. 8 for eobPt 9) would desync a real decoder but the §8.2 roundtrip cannot
    // catch it — so assert the symbol/bit split directly.
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
    // The FULL entry's `max_eob_pt = 9` admits the WHOLE `eob_pt_256` range: eob `1..=256`
    // (eobPt `1..=9`). A `Quant[256]` block is structurally bounded to eob `<= 256` (a
    // nonzero past scan index 255 cannot exist), so eob `> 256` is unreachable for a
    // well-formed 16x16 block — the only eob rejection (`max_eob_pt`) now never fires. A
    // block whose EOB is the very last coefficient (scan 255 → eob 256) is accepted.
    let quant = block_from(&[(0, 1), (255, 1)]);
    let trace = tokenize_general_16x16_luma_block_full(&quant, Q_CTX);
    assert!(trace.is_ok(), "eob 256 must be accepted: {trace:?}");
}

#[test]
fn golomb_magnitude_overflow_still_rejected_at_full_range() {
    // The golomb cap rejection is independent of the eob window: an LF DC magnitude far
    // beyond the per-`m` golomb cap is rejected with a typed error even when the EOB is
    // in the refined range. (A modest DC magnitude that exceeds the first golomb m's cap.)
    let mut quant = block_from(&[(199, 1)]);
    // Place a huge magnitude at the DC raster (scan 0 == raster 0).
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
