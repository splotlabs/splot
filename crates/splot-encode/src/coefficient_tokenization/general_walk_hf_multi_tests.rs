// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! NON-EOB high-frequency (eob 12..=16) general coefficient-walk tests
//! (`ENC-COEFF-GENERAL-WALK-HF-MULTI`): the full 4x4 high-frequency tail (scan
//! indices 10..=15, rasters 13, 10, 7, 14, 11, 15; diagonals 4..=6). At eob 12..=16 a
//! block has NON-EOB high-frequency coefficients — the EOB coefficient at scan
//! 11..=15 plus one or more high-frequency coefficients at scan 10..eob-2. A non-EOB
//! high-frequency coefficient is coded with the 4-symbol HF `coeff_base`
//! (`DEFAULT_COEFF_BASE_CDF`, the `coeff_base_hf_luma_context` band — NO near-DC
//! `magLimit = 5` carve-out, NO DC band) and, when refined, the HF `coeff_br`
//! (`DEFAULT_COEFF_BR_CDF`, the no-`+7` branch). Split from `general_walk_hf_tests.rs`
//! (the eob-11 EOB-only HF brick) to keep each test file under the 1000-line source
//! budget.
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
    COEFF_BASE_CTX_COUNT, CoefficientCdfRowSelector, CoefficientTokenSyntax,
    coeff_base_hf_luma_context, coeff_base_hf_token, roundtrip_entropy_tokens,
};

/// The coefficient CDF q-context the minimal general walk uses (q-ctx 0).
const Q_CTX: usize = 0;

/// The full 4x4 2D scan raster positions for scan indices 0..=15
/// (`[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]`): indices 0..=9 are
/// low-frequency (`row + col < 4`), indices 10..=15 (rasters 13, 10, 7, 14, 11, 15;
/// diagonals 4, 4, 4, 5, 5, 6) are high-frequency.
const SCAN_RASTER_0_15: [usize; 16] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15];

/// Builds a signed raster `[i32; 16]` from `eob` magnitudes assigned to scan
/// positions 0..eob, with a deterministic asymmetric sign pattern (scan-even
/// negative, scan-odd positive) so a swapped sign order cannot masquerade as a match
/// (the decode-verify-asymmetric-values lesson). `mags[c]` is the unsigned magnitude
/// at scan index `c`.
fn scan_block(eob: usize, mags: &[u32]) -> [i32; TX_4X4_COEFF_COUNT] {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    for (c, &mag) in mags.iter().enumerate().take(eob) {
        if mag == 0 {
            continue;
        }
        let raster = SCAN_RASTER_0_15[c];
        let value = if c % 2 == 0 {
            -(mag as i32)
        } else {
            mag as i32
        };
        quant[raster] = value;
    }
    quant
}

/// Scan indices 10..=15 are all high-frequency; scan indices 0..=9 are all
/// low-frequency. `is_lf_position` mirrors the decoder `get_lf_limits` for
/// `TX_CLASS_2D` luma (`row + col < 4`).
#[test]
fn scan_10_through_15_are_high_frequency() {
    for &raster in &SCAN_RASTER_0_15[10..] {
        assert!(
            !is_lf_position(raster),
            "raster {raster} (scan 10..=15) should be HF"
        );
    }
    for &raster in &SCAN_RASTER_0_15[..10] {
        assert!(
            is_lf_position(raster),
            "raster {raster} (scan 0..=9) should be LF"
        );
    }
}

/// The eob 12..=16 → eobPt 5 mapping (base 9): eob_extra 0 spans eob 9..=12, eob_extra
/// 1 spans eob 13..=16. Confirms the `eob_pt_16` symbol stays 4 (eobPt 5) across the
/// whole new window.
#[test]
fn eob_12_through_16_all_use_eob_pt_5() {
    for eob in 12..=16 {
        let mags = vec![1u32; eob];
        let quant = scan_block(eob, &mags);
        let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
        assert!(
            matches!(trace[1], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 4),
            "eob {eob} should use eob_pt_16 symbol 4 (eobPt 5); got {:?}",
            trace[1]
        );
    }
}

#[test]
fn general_hf_eob12_non_eob_hf_coeff_uses_coeff_base() {
    let quant = scan_block(12, &[1; 12]);
    assert_eq!(quant[10], 1);
    assert_eq!(quant[13], -1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 31);

    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0; got {:?}",
        trace[2]
    );
    assert_eq!(trace[3], BlockSymbolToken::Bypass { width: 1, value: 1 });
    assert_eq!(trace[4], BlockSymbolToken::Bypass { width: 1, value: 1 });

    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(
                c.selector(),
                CoefficientCdfRowSelector::CoeffBaseEob { ctx: 3, tx_size: 0, .. }
            ) && c.symbol() == 0),
        "scan-11 HF coeff_base_eob at ctx 3, symbol 0; got {:?}",
        trace[5]
    );

    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(
                c.selector(),
                CoefficientCdfRowSelector::CoeffBase { ctx: 0, tx_size: 0, .. }
            ) && c.symbol() == 1),
        "scan-10 NON-EOB HF coeff_base at ctx 0, symbol 1; got {:?}",
        trace[6]
    );
    assert!(
        !matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLf { .. })),
        "the non-EOB HF coeff must NOT use the LF CoeffBaseLf selector; got {:?}",
        trace[6]
    );

    for (i, token) in trace[7..17].iter().enumerate() {
        assert!(
            matches!(token, BlockSymbolToken::Coeff(c)
                if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLf { .. })),
            "non-EOB LF base token {i} must be a CoeffBaseLf selector; got {token:?}"
        );
    }

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_hf_eob13_non_eob_hf_coeff_br_and_context_band() {
    let mut mags = [1u32; 13];
    mags[11] = 4;
    let quant = scan_block(13, &mags);
    assert_eq!(quant[10], 4);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert!(
        matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseEob { ctx: 3, .. })),
        "scan-12 HF coeff_base_eob; got {:?}",
        trace[5]
    );
    assert!(
        matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBase { tx_size: 0, .. })
                && c.symbol() == 3),
        "scan-11 NON-EOB HF coeff_base saturated at level 3 → symbol 3; got {:?}",
        trace[6]
    );
    assert!(
        matches!(trace[7], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBr { .. })
                && c.symbol() == 1),
        "scan-11 NON-EOB HF coeff_br symbol 1; got {:?}",
        trace[7]
    );
    assert!(
        !matches!(trace[7], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBrLf { .. })),
        "the non-EOB HF coeff_br must NOT use the LF CoeffBrLf selector; got {:?}",
        trace[7]
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_hf_eob16_full_scan_max_hf_magnitude() {
    let mut mags = [1u32; 16];
    mags[13] = 5;
    let quant = scan_block(16, &mags);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let mut idx = 5usize;
    for offset in 0..16 {
        let c = 15 - offset;
        let token = trace[idx];
        idx += 1;
        let is_hf_eob = matches!(token, BlockSymbolToken::Coeff(coeff)
            if matches!(coeff.selector(), CoefficientCdfRowSelector::CoeffBaseEob { .. }));
        let is_hf_base = matches!(token, BlockSymbolToken::Coeff(coeff)
            if matches!(coeff.selector(), CoefficientCdfRowSelector::CoeffBase { .. }));
        let is_lf_base = matches!(token, BlockSymbolToken::Coeff(coeff)
            if matches!(coeff.selector(), CoefficientCdfRowSelector::CoeffBaseLf { .. }));
        if c >= 10 {
            if offset == 0 {
                assert!(
                    is_hf_eob,
                    "scan {c} EOB should be HF CoeffBaseEob; got {token:?}"
                );
            } else {
                assert!(
                    is_hf_base,
                    "scan {c} non-EOB should be HF CoeffBase; got {token:?}"
                );
            }
        } else {
            assert!(
                is_lf_base,
                "scan {c} non-EOB should be LF CoeffBaseLf; got {token:?}"
            );
        }
        let next_is_coeff_br = matches!(trace[idx], BlockSymbolToken::Coeff(coeff)
            if matches!(coeff.syntax(), CoefficientTokenSyntax::CoeffBr));
        if next_is_coeff_br {
            idx += 1;
        }
    }

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

/// The magnitude tiers the LF positions sweep: 0 (zero non-EOB), 1 (base low), 4 (LF
/// base max, no `coeff_br`), 5 (first LF `coeff_br`), 7 (LF `coeff_br` max).
const LF_TIERS: [u32; 5] = [0, 1, 4, 5, 7];
/// The magnitude tiers a NON-EOB high-frequency position may take (the HF base-range
/// cap is `NUM_BASE_LEVELS + COEFF_BASE_RANGE = 5`): 0 (zero), 1 (base low), 3 (HF
/// base max `NUM_BASE_LEVELS + 1`, first `coeff_br`), 4 (mid `coeff_br`), 5 (HF
/// `coeff_br` max).
const HF_NON_EOB_TIERS: [u32; 5] = [0, 1, 3, 4, 5];
/// The nonzero tiers the HF EOB coefficient may take (it is never 0): 1, 3, 4, 5.
const HF_EOB_TIERS: [u32; 4] = [1, 3, 4, 5];

/// Tokenizes one `scan_block`, proves it through `roundtrip_block_symbol_trace` (so
/// every reached HF/LF context routes — an unrouted context surfaces here as
/// `BlockSymbolTraceUnsupportedSelector`, not a wrong hash), and asserts
/// `recover_quant_from_tokens` reproduces the input exactly.
fn assert_roundtrips(eob: usize, mags: &[u32]) {
    let quant = scan_block(eob, mags);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX);
    assert!(
        trace.is_ok(),
        "tokenize failed eob {eob} mags {mags:?}: {trace:?}"
    );
    let trace = trace.unwrap();
    let proof = roundtrip_block_symbol_trace(&trace);
    assert!(
        proof.is_ok(),
        "roundtrip failed (unrouted HF ctx?) eob {eob} mags {mags:?}: {proof:?}"
    );
    assert!(
        !proof.unwrap().bytes().is_empty(),
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
}

/// BOUNDED routing fuzz over eob 12..=16 blocks. A full sweep of all positions over
/// all tiers would explode, so this (a) drives the EOB coefficient over its nonzero
/// tiers with all earlier positions at base-low 1, (b) sweeps each NON-EOB
/// high-frequency position across its tiers (the rest at 1), and (c) sweeps each LF
/// position across its tiers (the rest at 1). Every block MUST tokenize, roundtrip
/// (no unrouted context), and recover exactly.
#[test]
fn general_hf_multi_bounded_routing_fuzz() {
    let mut covered = 0usize;
    for eob in 12..=16usize {
        let eob_scan = eob - 1; // the EOB coefficient's scan index (always HF here).
        for &eob_mag in &HF_EOB_TIERS {
            let mut mags = vec![1u32; eob];
            mags[eob_scan] = eob_mag;
            assert_roundtrips(eob, &mags);
            covered += 1;
        }
        for non_eob_scan in 10..eob_scan {
            for &tier in &HF_NON_EOB_TIERS {
                let mut mags = vec![1u32; eob];
                mags[non_eob_scan] = tier;
                assert_roundtrips(eob, &mags);
                covered += 1;
            }
        }
        for lf_scan in 0..10usize {
            for &tier in &LF_TIERS {
                let mut mags = vec![1u32; eob];
                mags[lf_scan] = tier;
                assert_roundtrips(eob, &mags);
                covered += 1;
            }
        }
    }
    assert_eq!(covered, 59 + 64 + 69 + 74 + 79);
}

/// Dual-router coverage: route a `coeff_base_hf_token` (the NEW non-EOB HF
/// `coeff_base`) through the OTHER § 8.2 proof (`roundtrip_entropy_tokens`, the
/// q-generic `CoefficientTokenCdfRows` router), proving BOTH routers carry the new HF
/// non-EOB base bank (a missing arm fails only the path that exercises that router).
#[test]
fn non_eob_hf_coeff_base_routes_through_entropy_token_proof() {
    let tokens = [
        coeff_base_hf_token(Q_CTX, 0, 0, 3),
        coeff_base_hf_token(Q_CTX, 7, 0, 2),
    ];
    let proof = roundtrip_entropy_tokens(&tokens).unwrap();
    assert_eq!(proof.decoded_symbols(), &[3, 2]);
    assert!(!proof.bytes().is_empty());
}

/// Hole-free HF `coeff_base` context sweep: every § 8.3.2 HF `coeff_base` context
/// `0..COEFF_BASE_CTX_COUNT` (20) must route to a real generated default row through
/// the entropy-token proof (the same hole-free banking the LF tier uses). An unrouted
/// context would surface as a `CoefficientTokenizationUnsupportedCdfSelector`, not a
/// wrong hash.
#[test]
fn non_eob_hf_coeff_base_context_sweep_is_hole_free() {
    for ctx in 0..COEFF_BASE_CTX_COUNT {
        let token = coeff_base_hf_token(Q_CTX, ctx, 0, 1);
        let proof = roundtrip_entropy_tokens(&[token]);
        assert!(
            proof.is_ok(),
            "HF coeff_base ctx {ctx} failed to route: {proof:?}"
        );
        assert_eq!(proof.unwrap().decoded_symbols(), &[1]);
    }
}

/// The `coeff_base_hf_luma_context` 2D band map (mirroring the decoder
/// `CoeffBaseContext::select` HF branch): with an empty `Level[]` (`mag = 0`, `ctx2 =
/// 0`) the context is purely the raster-diagonal band — `row + col < 6 -> 0`,
/// `< 8 -> 5`, else `10`. Scan 10..=14 (rasters 13, 10, 7, 14, 11; diagonals 4, 4, 4,
/// 5, 5) all land in band 0; scan 15 (raster 15, diagonal 6) lands in band 5.
#[test]
fn coeff_base_hf_context_band_map_empty_levels() {
    let empty = [0u32; TX_4X4_COEFF_COUNT];
    for &raster in &SCAN_RASTER_0_15[10..15] {
        assert_eq!(
            coeff_base_hf_luma_context(raster, 2, 4, 4, 0, &empty),
            0,
            "raster {raster} (diagonal < 6) should be HF band 0"
        );
    }
    assert_eq!(coeff_base_hf_luma_context(15, 2, 4, 4, 0, &empty), 5);
}

/// The HF `magLimit = 3` divergence from the LF near-DC `magLimit = 5`: a single
/// HIGH-magnitude neighbour clamps to 3 in the HF context, raising `ctx = (mag + 1) >>
/// 1` to at most `(3 + 1) >> 1 = 2` per neighbour. For raster 14 (row 3, col 2,
/// diagonal 5, band 0) with a level-7 neighbour at raster 15 (offset [0,1] = (3,3))
/// the HF clamp pins the contribution to 3, so `ctx2 = (3 + 1) >> 1 = 2` and the HF
/// context is band-0 `ctx2 = 2` — NOT the `(7.min(5) + 1) >> 1 = 3` the LF near-DC
/// `magLimit = 5` would have produced.
#[test]
fn coeff_base_hf_context_clamps_neighbour_to_three() {
    let mut levels = [0u32; TX_4X4_COEFF_COUNT];
    levels[15] = 7; // the [0,1] neighbour of raster 14.
    let ctx = coeff_base_hf_luma_context(14, 2, 4, 4, 0, &levels);
    assert_eq!(
        ctx, 2,
        "HF magLimit 3 should cap the contribution → ctx2 = 2"
    );
}

#[test]
fn general_hf_rejects_eob17_impossible_for_4x4() {
    assert_eq!(MAX_GENERAL_SCAN_INDEX, 15);
    let quant = scan_block(16, &[1; 16]);
    assert!(
        tokenize_general_lf_luma_block(&quant, Q_CTX).is_ok(),
        "the full eob-16 block must be accepted"
    );
}

#[test]
fn general_hf_non_eob_magnitude_at_hf_maxlevel_is_golomb() {
    let mut mags = [1u32; 13];
    mags[10] = 6;
    let quant = scan_block(13, &mags);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);

    let mut mags = [1u32; 13];
    mags[10] = 526;
    let quant = scan_block(13, &mags);
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
        "non-EOB HF magnitude 526 (above the golomb cap) must be rejected; got {err:?}"
    );

    let mut mags = [1u32; 13];
    mags[0] = 6;
    let quant = scan_block(13, &mags);
    assert!(
        tokenize_general_lf_luma_block(&quant, Q_CTX).is_ok(),
        "LF magnitude 6 (within the LF base-range cap 7) must be accepted"
    );
}
