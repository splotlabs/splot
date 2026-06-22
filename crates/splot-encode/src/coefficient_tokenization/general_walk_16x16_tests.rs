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
use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientTokenSyntax, TX_SIZE_16X16_CTX,
    recover_quant_from_tokens_geom,
};
use crate::error::Error;
use splot_recon::{TransformClass, coefficient_scan_order};

/// The coefficient CDF q-context the general walk uses in these tests (q-ctx 0).
const Q_CTX: usize = 0;
/// 16x16 DCT_DCT coefficient count (`Quant[256]`).
const COEFF_COUNT: usize = 256;

/// Builds the 16x16 2D scan order (`scan[c]` = raster position of scan index `c`),
/// using the SAME `coefficient_scan_order` the tokenizer uses — never a hard-coded
/// raster table.
fn scan_16x16() -> Vec<u16> {
    let mut scan = vec![0u16; COEFF_COUNT];
    coefficient_scan_order(16, 16, TransformClass::TwoD, &mut scan).unwrap();
    scan
}

/// Builds a signed raster `[i32; 256]` from a list of `(scan_index, magnitude)` pairs,
/// with an ASYMMETRIC, mixed-sign pattern: an even scan index is negative, an odd one
/// positive (so a swapped sign order cannot masquerade as a match — the
/// decode-verify-asymmetric-values lesson). A magnitude of 0 leaves the position zero.
fn block_from(pairs: &[(usize, u32)]) -> [i32; COEFF_COUNT] {
    let scan = scan_16x16();
    let mut quant = [0i32; COEFF_COUNT];
    for &(c, mag) in pairs {
        if mag == 0 {
            continue;
        }
        let raster = scan[c] as usize;
        let value = if c % 2 == 0 {
            -(mag as i32)
        } else {
            mag as i32
        };
        quant[raster] = value;
    }
    quant
}

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
    // numCoeffs = Tx_Height << Tx_Width_Log2 = 16 << 4 = 256; band breaks at 32 & 64.
    assert_eq!(geom.num_coeffs, 256);
    assert_eq!(geom.tx_size_ctx, TX_SIZE_16X16_CTX);
}

#[test]
fn coeff_base_eob_ctx_16x16_band_breaks_are_32_and_64() {
    // The § 8.3.2 `coeff_base_eob_ctx` band breaks for a 256-coeff block are
    // numCoeffs/8 = 32 and numCoeffs/4 = 64 (vs 2 & 4 for 4x4). Mirror the decoder.
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
    // The all-zero `txb_skip` (symbol 1) MUST carry the TX_16X16 txSzCtx, not TX_4X4 — the
    // CDF row is keyed by tx_size, so a 4x4-context all-zero token would desync a real
    // decoder (a mismatch the §8.2 self-consistency roundtrip cannot catch).
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
    // A single DC coefficient (scan index 0, magnitude 6, negative). The txb_skip and
    // coeff_base_eob MUST carry the TX_16X16 txSzCtx and the EOB symbol the eob_pt_256
    // size class.
    let quant = block_from(&[(0, 6)]);
    let trace = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap();
    // all_zero(0), eob_pt_256(0), coeff_base_eob, coeff_br, dc_sign, U, V = 7.
    assert_eq!(trace.len(), 7);
    // txb_skip at TX_16X16.
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c) if matches!(
            c.selector(),
            CoefficientCdfRowSelector::TxbSkip { tx_size: TX_SIZE_16X16_CTX, .. }
        )
    ));
    // EOB symbol is eob_pt_256 (size class), symbol 0 (eobPt 1 → eob 1).
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c) if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256)
            && c.symbol() == 0
    ));
    // coeff_base_eob at TX_16X16, LF DC ctx 0.
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
    // DC (scan 0, mag 3, negative) + one AC (scan 1, mag 5, positive — an asymmetric,
    // mixed-sign LF pair; mag 5 exercises the LF `coeff_br`).
    let quant = block_from(&[(0, 3), (1, 5)]);
    let trace = assert_16x16_roundtrips(&quant);
    // eob 2 → eob_pt_256 symbol 1, no eob_extra.
    assert!(matches!(
        trace[1],
        BlockSymbolToken::Coeff(c) if matches!(c.syntax(), CoefficientTokenSyntax::EobPt256)
            && c.symbol() == 1
    ));
    // No eob_extra token for eobPt < 3.
    assert!(!trace.iter().any(|t| matches!(t, BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra))));
}

#[test]
fn lf_region_16x16_block_roundtrips() {
    // An LF-region block: several nonzeros at scan indices whose rasters are all in the
    // low-frequency diagonal (row + col < 4), with mixed magnitudes and signs. The
    // scan indices 0..=5 of the 16x16 2D scan are all low-frequency (the same way 4x4
    // scan 0..=9 are). Magnitudes mix the base tier (1), the LF base max (4), and a
    // `coeff_br` level (6).
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
    // A block with a nonzero at a HIGH-frequency scan index (>= 10): the 16x16 2D scan
    // reaches the first HF coefficient (row + col >= 4) before scan index 10, so a
    // nonzero placed at scan index 12 is firmly high-frequency. Confirm it is HF, then
    // roundtrip a block whose EOB coefficient is that HF coefficient (plus an LF DC).
    let scan = scan_16x16();
    let hf_index = 12usize;
    let raster = scan[hf_index] as usize;
    let row = raster >> 4;
    let col = raster - (row << 4);
    assert!(
        row + col >= 4,
        "scan index {hf_index} (raster {raster}) must be HF"
    );
    // DC (scan 0, mag 2, negative) + a HF EOB coefficient at scan 12 (mag 3, negative —
    // mag 3 == NUM_BASE_LEVELS + 1, so it exercises the HF base max + HF coeff_br).
    let quant = block_from(&[(0, 2), (hf_index, 3)]);
    let trace = assert_16x16_roundtrips(&quant);
    // The EOB coefficient (the first base-pass token after the header) MUST be the HF
    // `CoeffBaseEob` selector at TX_16X16 (NOT the LF `CoeffBaseLfEob`).
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
    // A block reaching eob 32 (eobPt 6): nonzeros up to scan index 31, asymmetric/mixed
    // signs. eobPt 6 carries the `eob_extra` CDF flag and `eobPt - 3 = 3`
    // `eob_extra_bit` literals. eob 32 → base 17, offset 15 = 0b1111 → eob_extra (high
    // bit) 1, the low 3 bits 0b111. The block tokenizes, routes, and recovers exactly.
    // Nonzeros at scan indices 0, 5, 17, and 31 (the EOB) with mixed magnitudes; scan
    // index 31 → eob 32.
    let pairs = [(0usize, 4u32), (5, 1), (17, 2), (31, 3)];
    let quant = block_from(&pairs);
    let trace = assert_16x16_roundtrips(&quant);

    // eob_pt_256 symbol 5 (eobPt 6).
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
    // An eob_extra CDF flag follows (the HIGH refinement bit = 1 for offset 15).
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
    // Exactly 3 `eob_extra_bit` bypass literals (width eobPt - 3 = 3), MSB-first: the
    // low 3 bits of offset 15 are 0b111.
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
    // A nonzero at scan index 32 → eob 33 → eobPt 7, beyond the base-pass window. It is
    // rejected with a typed `CoefficientTokenizationUnsupportedEob` error (the
    // `eob_pt_256` higher-eobPt signaling is the next brick), NOT a panic.
    let quant = block_from(&[(0, 1), (32, 1)]);
    let err = tokenize_general_16x16_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(
        matches!(err, Error::CoefficientTokenizationUnsupportedEob { .. }),
        "eob 33 must be rejected as out of the base-pass window; got {err:?}"
    );
}

#[test]
fn sign_swap_negative_test_16x16() {
    // Two LF coefficients with DIFFERENT magnitudes and OPPOSITE signs: DC (scan 0)
    // negative magnitude 4, AC (scan 1) positive magnitude 5. Recovering must restore
    // BOTH the magnitudes AND the (distinct) signs at the (distinct) positions — a
    // swapped sign order would surface because the magnitudes differ (exit_symbol's
    // bit-count-only check cannot catch a value/position transposition).
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
