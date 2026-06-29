// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 8.2 SELF-CONSISTENCY roundtrip tests for the general 16x16 DCT_DCT luma base-pass
//! tokenizer (`ENC-COEFF-TOKENIZE-16X16-BASE`). The SAME size-generic codepath the 4x4
//! walk uses, specialized to a `Quant[256]` block in the base pass (eob `1..=32`).
//!
//! HONESTY: the `roundtrip_block_symbol_trace` / `roundtrip_entropy_tokens` proofs are
//! AV2 § 8.2 SELF-CONSISTENCY — the same code authored the emission and its inverse, so
//! they prove the encoder's emitted (level, sign, position) triples are internally
//! reversible and that every reached § 8.3.2 context routes to a real generated default
//! row. They do NOT validate the § 8.3.2 CDF contexts against a real decoder; context
//! conformance is deferred to the splot-decode cross-check brick.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::general_walk::coeff_base_eob_ctx_geom;
use super::test_support::{COEFF_COUNT, Q_CTX, block_from, scan_16x16};
use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientTokenSyntax, TX_SIZE_16X16_CTX,
    recover_quant_from_tokens_geom,
};
use crate::error::Error;

/// Tokenizes a 16x16 block, proves it through BOTH § 8.2 self-consistency proofs (the
/// block-symbol router via `roundtrip_block_symbol_trace`, which carries the V-plane
/// `all_zero` the entropy-proof router does not), and asserts the recovery reproduces
/// the input exactly. An unrouted § 8.3.2 context surfaces here as
/// `BlockSymbolTraceUnsupportedSelector` (a routing failure), NOT a wrong hash.
fn assert_16x16_roundtrips(quant: &[i32; COEFF_COUNT]) -> Vec<BlockSymbolToken> {
    let trace = tokenize_general_16x16_luma_block(quant, Q_CTX);
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

#[test]
fn tx_geom_16x16_descriptor_is_consistent() {
    let geom = TxGeom::TX_16X16;
    assert_eq!(geom.width, 16);
    assert_eq!(geom.height, 16);
    assert_eq!(geom.bwl, 4);
    assert_eq!(geom.coeff_count, COEFF_COUNT);
    assert_eq!(geom.max_scan_index, 255);
    assert_eq!(geom.num_coeffs, 256);
    assert_eq!(geom.tx_size_ctx, TX_SIZE_16X16_CTX);
}

#[test]
fn coeff_base_eob_ctx_16x16_band_breaks_are_32_and_64() {
    let geom = TxGeom::TX_16X16;
    assert_eq!(coeff_base_eob_ctx_geom(0, geom), 0);
    assert_eq!(coeff_base_eob_ctx_geom(1, geom), 1);
    assert_eq!(coeff_base_eob_ctx_geom(32, geom), 1);
    assert_eq!(coeff_base_eob_ctx_geom(33, geom), 2);
    assert_eq!(coeff_base_eob_ctx_geom(64, geom), 2);
    assert_eq!(coeff_base_eob_ctx_geom(65, geom), 3);
}

#[test]
fn all_zero_16x16_block_emits_single_all_zero_token() {
    let quant = [0i32; COEFF_COUNT];
    let trace = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(trace.len(), 1);
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c) if c.symbol() == 1
            && matches!(c.syntax(), CoefficientTokenSyntax::AllZero)
            && matches!(
                c.selector(),
                CoefficientCdfRowSelector::TxbSkip { tx_size: TX_SIZE_16X16_CTX, .. }
            )
    ));
    let recovered = recover_quant_from_tokens_geom(&trace, TxGeom::TX_16X16, Q_CTX).unwrap();
    assert_eq!(recovered, vec![0i32; COEFF_COUNT]);
}

#[test]
fn dc_only_16x16_uses_eob_pt_256_and_tx_16x16_selectors() {
    let quant = block_from(&[(0, 6)]);
    let trace = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(trace.len(), 7);
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c) if matches!(
            c.selector(),
            CoefficientCdfRowSelector::TxbSkip { tx_size: TX_SIZE_16X16_CTX, .. }
        )
    ));
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c) if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256)
            && c.symbol() == 0
    ));
    assert!(matches!(
        trace[2],
        BlockSymbolToken::Coeff(c) if matches!(
            c.selector(),
            CoefficientCdfRowSelector::CoeffBaseLfEob { tx_size: TX_SIZE_16X16_CTX, ctx: 0, .. }
        )
    ));
    assert_16x16_roundtrips(&quant);
}

#[test]
fn two_coeff_16x16_dc_plus_one_ac_roundtrips() {
    let quant = block_from(&[(0, 3), (1, 5)]);
    let trace = assert_16x16_roundtrips(&quant);
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c) if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256)
            && c.symbol() == 1
    ));
    assert!(!trace.iter().any(|t| matches!(t, BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra))));
}

#[test]
fn lf_region_16x16_block_roundtrips() {
    let scan = scan_16x16();
    for (c, &raster_u16) in scan.iter().enumerate().take(6) {
        let raster = raster_u16 as usize;
        let row = raster >> 4;
        let col = raster - (row << 4);
        assert!(
            row + col < 4,
            "scan index {c} (raster {raster}, row {row}, col {col}) must be LF"
        );
    }
    let quant = block_from(&[(0, 4), (1, 1), (2, 6), (3, 1), (4, 4), (5, 2)]);
    assert_16x16_roundtrips(&quant);
}

#[test]
fn hf_coefficient_16x16_at_scan_index_ge_10_roundtrips() {
    let scan = scan_16x16();
    let hf_index = 12usize;
    let raster = scan[hf_index] as usize;
    let row = raster >> 4;
    let col = raster - (row << 4);
    assert!(
        row + col >= 4,
        "scan index {hf_index} (raster {raster}) must be HF"
    );
    let quant = block_from(&[(0, 2), (hf_index, 3)]);
    let trace = assert_16x16_roundtrips(&quant);
    let eob_base = trace
        .iter()
        .find(|t| {
            matches!(t, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBaseEob))
        })
        .copied()
        .unwrap();
    assert!(matches!(
        eob_base,
        BlockSymbolToken::Coeff(c) if matches!(
            c.selector(),
            CoefficientCdfRowSelector::CoeffBaseEob { tx_size: TX_SIZE_16X16_CTX, .. }
        )
    ));
}

#[test]
fn eob_near_32_16x16_block_roundtrips_with_eob_extra() {
    let pairs = [(0usize, 4u32), (5, 1), (17, 2), (31, 3)];
    let quant = block_from(&pairs);
    let trace = assert_16x16_roundtrips(&quant);

    let eob_pt = trace
        .iter()
        .find(|t| {
            matches!(t, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256))
        })
        .copied()
        .unwrap();
    assert!(matches!(
        eob_pt,
        BlockSymbolToken::Coeff(c) if c.symbol() == 5
    ));
    let eob_extra = trace
        .iter()
        .find(|t| {
            matches!(t, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra))
        })
        .copied()
        .unwrap();
    assert!(matches!(
        eob_extra,
        BlockSymbolToken::Coeff(c) if c.symbol() == 1
    ));
    let bypasses: Vec<_> = trace
        .iter()
        .take_while(|t| {
            !matches!(t, BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBaseEob))
        })
        .filter_map(|t| match t {
            BlockSymbolToken::Bypass { width: 1, value } => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(
        bypasses,
        vec![1, 1, 1],
        "3 eob_extra_bit literals MSB-first"
    );
}

#[test]
fn rejects_eob_above_32() {
    let quant = block_from(&[(0, 1), (32, 1)]);
    let err = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(
        matches!(err, Error::CoefficientTokenizationUnsupportedEob { .. }),
        "eob 33 must be rejected as out of the base-pass window; got {err:?}"
    );
}

#[test]
fn sign_swap_negative_test_16x16() {
    let quant = block_from(&[(0, 4), (1, 5)]);
    assert_eq!(
        quant[scan_16x16()[0] as usize],
        -4,
        "DC negative magnitude 4"
    );
    assert_eq!(
        quant[scan_16x16()[1] as usize],
        5,
        "AC positive magnitude 5"
    );
    assert_16x16_roundtrips(&quant);
}
