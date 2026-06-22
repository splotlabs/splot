// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! HIGH-frequency (eob 11) general coefficient-walk tests
//! (`ENC-COEFF-GENERAL-WALK-HF-EOB11`): the FIRST high-frequency coefficient (scan
//! index 10, raster 13 = row 3, col 1, diagonal 4) as the eob-11 EOB coefficient,
//! coded with the 4-symbol HF `coeff_base_eob` (`DEFAULT_COEFF_BASE_EOB_CDF`) and —
//! when its magnitude exceeds the base tier — the HF `coeff_br` (`DEFAULT_COEFF_BR_CDF`,
//! constant context 0, NO `+7` offset). Split from `general_walk_eob_extra_tests.rs`
//! to keep each test file under the 1000-line source budget; the LF eob 3..=10
//! behaviour stays there.
//!
//! HONESTY: the `roundtrip_block_symbol_trace` / `roundtrip_entropy_tokens` proofs are
//! AV2 § 8.2 SELF-CONSISTENCY — the same code authored the emission and its inverse,
//! so they prove the encoder's emitted (level, sign, position) triples are internally
//! reversible and that every reached HF context routes to a real generated default
//! row. They do NOT validate the § 8.3.2 CDF contexts against a real decoder; context
//! conformance is deferred to the splot-decode cross-check brick.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{
    CoefficientCdfRowSelector, CoefficientTokenSyntax, coeff_base_hf_eob_token, coeff_br_hf_token,
    roundtrip_entropy_tokens,
};

/// The coefficient CDF q-context the minimal general walk uses (q-ctx 0).
const Q_CTX: usize = 0;

/// The 4x4 2D scan raster positions for scan indices 0..=10
/// (`[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13]`): indices 0..=9 are low-frequency
/// (`row + col < 4`) and index 10 (raster 13 = row 3, col 1, diagonal 4) is the FIRST
/// high-frequency coefficient.
const SCAN_RASTER_0_10: [usize; 11] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13];

/// Builds a signed raster `[i32; 16]` from `eob` magnitudes assigned to scan
/// positions 0..eob, with a deterministic asymmetric sign pattern (scan-even
/// negative, scan-odd positive) so a swapped sign order cannot masquerade as a
/// match (the decode-verify-asymmetric-values lesson). `mags[c]` is the unsigned
/// magnitude at scan index `c`.
fn scan_block(eob: usize, mags: &[u32]) -> [i32; TX_4X4_COEFF_COUNT] {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    for (c, &mag) in mags.iter().enumerate().take(eob) {
        if mag == 0 {
            continue;
        }
        let raster = SCAN_RASTER_0_10[c];
        let value = if c % 2 == 0 {
            -(mag as i32)
        } else {
            mag as i32
        };
        quant[raster] = value;
    }
    quant
}

/// `coeff_base_eob_ctx(10)` for a 16-coefficient block is `3`: `c = 10` is neither
/// `0`, nor `<= numCoeffs/8 (2)`, nor `<= numCoeffs/4 (4)`, so it lands in the final
/// band `3`. This is the SHARED (LF/HF-independent) scan-band context the HF EOB
/// coefficient reads.
#[test]
fn coeff_base_eob_ctx_for_scan_index_10_is_3() {
    assert_eq!(coeff_base_eob_ctx(10), 3);
}

/// Raster 13 (scan index 10) is high-frequency; rasters 0..=9's scan prefix are all
/// low-frequency. `is_lf_position` mirrors the decoder `get_lf_limits` for
/// `TX_CLASS_2D` luma (`row + col < 4`).
#[test]
fn raster_13_is_high_frequency_lf_prefix_is_low_frequency() {
    // Raster 13 = row 3, col 1 → diagonal 4 → HF.
    assert!(!is_lf_position(13));
    // Every scan-0..=9 raster is LF.
    for &raster in &SCAN_RASTER_0_10[..10] {
        assert!(is_lf_position(raster), "raster {raster} should be LF");
    }
}

#[test]
fn general_hf_eob11_all_mag1_exact_stream() {
    // eob == 11, all eleven coefficients magnitude 1. The EOB coefficient is the HF
    // coefficient at scan index 10 (raster 13).
    let quant = scan_block(11, &[1; 11]);
    // Sanity: raster 13 carries the EOB coefficient (scan index 10, even → negative).
    assert_eq!(quant[13], -1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // 5 header (all_zero, eob_pt_16, eob_extra, 2 eob_extra_bit) + 11 base + 11 sign +
    // 2 chroma = 29 tokens.
    assert_eq!(trace.len(), 29);

    // eob_pt_16 symbol 4 (eobPt 5; eob 11 in 9..=12).
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 4),
        "eob_pt_16 symbol 4 (eobPt 5)"
    );
    // eob_extra flag 0 (offset 2, width 2 → high bit (2 >> 2) & 1 = 0).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0"
    );
    // Two eob_extra_bit bypass literals MSB-first: offset low bits = 2 = binary 10 →
    // bit1 = 1, bit0 = 0.
    assert_eq!(trace[3], BlockSymbolToken::Bypass { width: 1, value: 1 });
    assert_eq!(trace[4], BlockSymbolToken::Bypass { width: 1, value: 0 });

    // trace[5] is the HF EOB coeff_base: it MUST be the HF `CoeffBaseEob` selector (NOT
    // the LF `CoeffBaseLfEob`), at the shared scan-band ctx 3, level 1 → symbol 0.
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(
                c.selector(),
                CoefficientCdfRowSelector::CoeffBaseEob { ctx: 3, tx_size: 0, .. }
            ) && c.symbol() == 0),
        "scan-10 HF coeff_base_eob at ctx 3, symbol 0; got {:?}",
        trace[5]
    );
    // And it must NOT be the LF EOB selector.
    assert!(
        !matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { .. })),
        "the HF EOB coeff must NOT use the LF CoeffBaseLfEob selector"
    );

    // The ten non-EOB coefficients (scan 0..=9) MUST stay on the LF `coeff_base` path:
    // they occupy trace[6..16] (after the HF EOB coeff_base at trace[5]).
    for (i, token) in trace[6..16].iter().enumerate() {
        assert!(
            matches!(token, BlockSymbolToken::Coeff(c)
                if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLf { .. })),
            "non-EOB base token {i} must be an LF CoeffBaseLf selector; got {token:?}"
        );
    }

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_hf_eob11_hf_eob_coeff_br_at_ctx0() {
    // eob == 11, the HF EOB coefficient has magnitude 3 (> NUM_BASE_LEVELS = 2), so it
    // emits an interleaved HF `coeff_br` at the constant empty-`Level[]` context 0 (the
    // non-DC HF `else { mag }` branch with mag 0, NO `+7`). The HF base level saturates
    // at `NUM_BASE_LEVELS + 1 = 3` (a 3-symbol CDF), so level 3 → symbol 2 and the
    // br_symbol is `3 - (NUM_BASE_LEVELS + 1) = 0`. The LF coefficients are magnitude 1.
    let mut mags = [1u32; 11];
    mags[10] = 3;
    let quant = scan_block(11, &mags);
    assert_eq!(quant[13], -3); // scan index 10 is even → negative.

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // 5 header + 11 base + 1 HF coeff_br + 11 sign + 2 chroma = 30 tokens.
    assert_eq!(trace.len(), 30);

    // trace[5] = HF coeff_base_eob, level min(3, NUM_BASE_LEVELS+1=3) = 3 → symbol 2.
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(
                c.selector(),
                CoefficientCdfRowSelector::CoeffBaseEob { ctx: 3, tx_size: 0, .. }
            ) && c.symbol() == 2),
        "HF coeff_base_eob level 3 → symbol 2; got {:?}",
        trace[5]
    );
    // trace[6] = the interleaved HF coeff_br: it MUST be the HF `CoeffBr` selector at
    // the constant ctx 0 (NOT the LF `CoeffBrLf`), symbol 3 - (2+1) = 0.
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBr { ctx: 0, .. })
                && c.symbol() == 0),
        "HF coeff_br at ctx 0, symbol 0; got {:?}",
        trace[6]
    );
    // And it must NOT be the LF `CoeffBrLf` selector (the no-`+7` confirmation: an LF
    // non-DC EOB coeff_br would be ctx 7 via `CoeffBrLf`).
    assert!(
        !matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBrLf { .. })),
        "the HF coeff_br must NOT use the LF CoeffBrLf selector"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_hf_eob11_hf_eob_coeff_br_max_magnitude() {
    // The HF EOB coefficient at magnitude 5 (the HF base-range cap
    // `NUM_BASE_LEVELS + COEFF_BASE_RANGE = 5`, NOT the LF cap 7) drives the HF
    // `coeff_br` to its max symbol `5 - (NUM_BASE_LEVELS + 1) = 2`. Magnitude > 5 at an
    // HF position needs the `read_quant` golomb tail (a later sub-brick) and is
    // rejected (see `general_hf_rejects_magnitude_above_hf_cap`).
    let mut mags = [1u32; 11];
    mags[10] = 5;
    let quant = scan_block(11, &mags);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // HF coeff_base_eob saturates at level 3 → symbol 2.
    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseEob { ctx: 3, .. })
                && c.symbol() == 2),
        "HF coeff_base_eob level 3 (saturated) → symbol 2; got {:?}",
        trace[5]
    );
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBr { ctx: 0, .. })
                && c.symbol() == 2),
        "HF coeff_br at ctx 0 should be symbol 2 for magnitude 5; got {:?}",
        trace[6]
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_hf_magnitude_at_hf_maxlevel_is_golomb() {
    // An HF EOB coefficient at magnitude 6 (the HF `maxLevel`) is now the FIRST HF
    // § 5.20.7.28 `read_quant` golomb coefficient (the `ENC-COEFF-GENERAL-WALK-GOLOMB`
    // tier): it saturates its base+`coeff_br` level to `maxLevel` and carries the
    // golomb tail. It tokenizes and roundtrips (a single golomb coefficient, `m = 1`).
    let mut mags = [1u32; 11];
    mags[10] = 6;
    let quant = scan_block(11, &mags);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);

    // A magnitude above the HF golomb cap 523 at an HF position is still rejected.
    let mut mags = [1u32; 11];
    mags[10] = 526;
    let quant = scan_block(11, &mags);
    let err = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude {
                magnitude: 526,
                max_magnitude: 523,
                coefficient_index: 13,
                ..
            }
        ),
        "HF magnitude 526 (above the golomb cap) must be rejected; got {err:?}"
    );
}

/// The magnitude tiers the LF positions sweep: 0 (zero non-EOB), 1 (base low), 4
/// (LF base max, no `coeff_br`), 5 (first LF `coeff_br`), 7 (LF `coeff_br` max).
const LF_TIERS: [u32; 5] = [0, 1, 4, 5, 7];
/// The nonzero tiers the HF EOB coefficient may take (it is never 0; the HF base-range
/// cap is `NUM_BASE_LEVELS + COEFF_BASE_RANGE = 5`, so 7 is out of HF scope): 1 (base
/// low), 3 (HF base max `NUM_BASE_LEVELS + 1`, first `coeff_br`), 4 (mid `coeff_br`), 5
/// (HF `coeff_br` max).
const HF_EOB_TIERS: [u32; 4] = [1, 3, 4, 5];

/// Tokenizes one eob-11 `scan_block`, proves it through `roundtrip_block_symbol_trace`
/// (so every reached HF/LF context routes — an unrouted context surfaces here as
/// `BlockSymbolTraceUnsupportedSelector`, not a wrong hash), and asserts
/// `recover_quant_from_tokens` reproduces the input exactly.
fn assert_eob11_roundtrips(mags: &[u32]) {
    let quant = scan_block(11, mags);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX);
    assert!(trace.is_ok(), "tokenize failed mags {mags:?}: {trace:?}");
    let trace = trace.unwrap();
    let proof = roundtrip_block_symbol_trace(&trace);
    assert!(
        proof.is_ok(),
        "roundtrip failed (unrouted HF ctx?) mags {mags:?}: {proof:?}"
    );
    assert!(
        !proof.unwrap().bytes().is_empty(),
        "empty proof mags {mags:?}"
    );
    let recovered = recover_quant_from_tokens(&trace, Q_CTX);
    assert!(
        recovered.is_ok(),
        "recover failed mags {mags:?}: {recovered:?}"
    );
    assert_eq!(recovered.unwrap(), quant, "recover != input mags {mags:?}");
}

/// BOUNDED routing fuzz over eob-11 blocks: the HF EOB coefficient over every nonzero
/// tier, and the ten LF positions exercised over the magnitude tiers. A full
/// `5^10`-combination sweep would explode, so this (a) drives all ten LF positions at
/// one shared tier and (b) walks each LF position across every tier with the others
/// fixed at base-low 1. Every block MUST tokenize, roundtrip (no unrouted context),
/// and recover exactly.
#[test]
fn general_hf_eob11_bounded_routing_fuzz() {
    let mut covered = 0usize;
    for &hf_mag in &HF_EOB_TIERS {
        // (a) all ten LF positions at one shared tier (including 0).
        for &tier in &LF_TIERS {
            let mut mags = [tier; 11];
            mags[10] = hf_mag;
            assert_eob11_roundtrips(&mags);
            covered += 1;
        }
        // (b) one LF position swept across every tier, the rest fixed at 1.
        for pos in 0..10 {
            for &tier in &LF_TIERS {
                let mut mags = [1u32; 11];
                mags[pos] = tier;
                mags[10] = hf_mag;
                assert_eob11_roundtrips(&mags);
                covered += 1;
            }
        }
    }
    // 4 hf tiers * (5 shared + 10 positions * 5 tiers) = 4 * 55 = 220.
    assert_eq!(covered, 220, "expected 220 enumerated eob-11 blocks");
}

/// Dual-router coverage: route a `coeff_base_hf_eob_token` and a `coeff_br_hf_token`
/// through the OTHER § 8.2 proof (`roundtrip_entropy_tokens`, the q-generic
/// `CoefficientTokenCdfRows` router), proving BOTH routers carry the HF banks (a
/// missing arm fails only the path that exercises that router).
#[test]
fn hf_tokens_route_through_entropy_token_proof() {
    let tokens = [
        // HF EOB coeff_base at the eob-11 shared ctx 3, level 3 → symbol 2 (the HF
        // `coeff_base_eob` CDF is 3-symbol, so 2 is its max value).
        coeff_base_hf_eob_token(Q_CTX, 3, 3),
        // HF coeff_br at the constant ctx 0, symbol 2 (magnitude 5: 5 - 3 = 2).
        coeff_br_hf_token(Q_CTX, 0, 2),
    ];
    let proof = roundtrip_entropy_tokens(&tokens).unwrap();
    assert_eq!(proof.decoded_symbols(), &[2, 2]);
    assert!(!proof.bytes().is_empty());
}

#[test]
fn general_hf_eob12_scan_index_11_now_in_scope() {
    // A nonzero at scan index 11 (raster 10 in the order [..,13,10,..], row 2 + col 2 =
    // diagonal 4) is the SECOND high-frequency coefficient, eob 12. The
    // `ENC-COEFF-GENERAL-WALK-HF-MULTI` sub-brick lifted the gate to eob 16, so it is
    // now in scope (its detailed non-EOB HF behaviour is covered in
    // `general_walk_hf_multi_tests`). The boundary is the last 4x4 scan index 15.
    assert_eq!(MAX_GENERAL_SCAN_INDEX, 15);
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[10] = 1; // raster 10 is scan index 11 → eob 12.
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);

    // An eob-11 block whose HF EOB coefficient additionally has a nonzero at scan index
    // 11 (eob 12) is likewise in scope and roundtrips.
    let mut quant = scan_block(11, &[1; 11]);
    quant[10] = 1; // adds scan index 11 → eob 12.
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}
