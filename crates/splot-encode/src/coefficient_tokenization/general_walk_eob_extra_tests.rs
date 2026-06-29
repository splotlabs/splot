// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Refined-eob (eob 3..=10) general LF coefficient-walk tests: the
//! `eob_extra` CDF flag, the `eob_extra_bit` bypass literals, the reverse-scan
//! base/`coeff_br` pass over the low-frequency scan prefix, and the exhaustive
//! context-routing fuzz. Split from `general_walk_tests.rs` to keep each test
//! file under the 1000-line source budget (eob `<=` 2 base/br behaviour stays
//! there).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{CoefficientCdfRowSelector, CoefficientTokenSyntax};

/// The coefficient CDF q-context the minimal general walk uses (q-ctx 0).
const Q_CTX: usize = 0;

/// The 4x4 2D scan raster positions for scan indices 0..=9
/// (the low-frequency prefix of `[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, ...]`): every
/// one has `row + col < 4` (the LF region), so eob `1..=10` is entirely
/// low-frequency. Scan index 10 (raster 13) is the first high-frequency coefficient.
const SCAN_RASTER_0_9: [usize; 10] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3];

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
        let raster = SCAN_RASTER_0_9[c];
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
    let quant = scan_block(3, &[1, 1, 1]);
    assert_eq!(quant[0], -1);
    assert_eq!(quant[4], 1);
    assert_eq!(quant[1], -1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 11);

    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 2),
        "eob_pt_16 symbol 2 (eobPt 3)"
    );
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0 (eob 3)"
    );
    assert!(
        matches!(trace[3], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseLfEob { ctx: 1, .. })
                && c.symbol() == 0),
        "scan-2 coeff_base_eob at ctx 1 (coeff_base_eob_ctx(c=2)), symbol 0"
    );

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
    let quant = scan_block(4, &[1, 1, 1, 1]);
    assert_eq!(quant[0], -1);
    assert_eq!(quant[4], 1);
    assert_eq!(quant[1], -1);
    assert_eq!(quant[8], 1);

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 13);

    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 2),
        "eob_pt_16 symbol 2 (eobPt 3)"
    );
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 1),
        "eob_extra flag 1 (eob 4)"
    );
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

/// The magnitude tiers exercised by the routing fuzz: 0 (zero non-EOB), 1 (base
/// low), 4 (base max, no `coeff_br`), 5 (first `coeff_br`), 7 (`coeff_br` max).
const TIERS: [u32; 5] = [0, 1, 4, 5, 7];
/// The nonzero tiers an EOB coefficient may take (it never takes 0).
const NONZERO_TIERS: [u32; 4] = [1, 4, 5, 7];

/// Tokenizes one `scan_block`, proves it through `roundtrip_block_symbol_trace` (so
/// every reached `coeff_base`/`coeff_base_eob`/`coeff_br`/`eob_extra`/`eob_extra_bit`
/// context or literal is routed — an unrouted context surfaces here as
/// `BlockSymbolTraceUnsupportedSelector`, not a wrong hash), and asserts
/// `recover_quant_from_tokens` reproduces the input exactly.
fn assert_block_roundtrips(eob: usize, mags: &[u32]) {
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
}

/// EXHAUSTIVE routing fuzz for the smaller eobs (3..=4): enumerate every in-scope
/// block over the magnitude tier set ({0,1,4,5,7}), with the eob-1 (EOB) position
/// forced nonzero and all lower positions free (including 0). Every such block MUST
/// tokenize, roundtrip, and recover exactly — discovering the complete reachable
/// context set for these eobs.
#[test]
fn general_lf_eob3_4_exhaustive_routing_fuzz() {
    let mut covered = 0usize;
    for eob in 3..=4usize {
        for &eob_mag in &NONZERO_TIERS {
            let lower = eob - 1;
            let combos = TIERS.len().pow(lower as u32);
            for combo in 0..combos {
                let mut mags = [0u32; 10];
                let mut rem = combo;
                for slot in mags.iter_mut().take(lower) {
                    *slot = TIERS[rem % TIERS.len()];
                    rem /= TIERS.len();
                }
                mags[eob - 1] = eob_mag;
                assert_block_roundtrips(eob, &mags);
                covered += 1;
            }
        }
    }
    assert_eq!(covered, 600, "expected 600 enumerated in-scope blocks");
}

#[test]
fn general_lf_eob6_eobpt4_exact_eob_signaling() {
    let quant = scan_block(6, &[1, 1, 1, 1, 1, 1]);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 18);

    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 0
    ));
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 3),
        "eob_pt_16 symbol 3 (eobPt 4)"
    );
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0"
    );
    assert_eq!(
        trace[3],
        BlockSymbolToken::Bypass { width: 1, value: 1 },
        "the single eob_extra_bit bypass literal is 1"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob10_eobpt5_exact_eob_signaling() {
    let quant = scan_block(10, &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert_eq!(trace.len(), 27);

    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 4),
        "eob_pt_16 symbol 4 (eobPt 5)"
    );
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0"
    );
    assert_eq!(
        trace[3],
        BlockSymbolToken::Bypass { width: 1, value: 0 },
        "the first (MSB) eob_extra_bit bypass literal is 0"
    );
    assert_eq!(
        trace[4],
        BlockSymbolToken::Bypass { width: 1, value: 1 },
        "the second (LSB) eob_extra_bit bypass literal is 1"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    let recovered = recover_quant_from_tokens(&trace, Q_CTX).unwrap();
    assert_eq!(recovered, quant);
}

#[test]
fn general_lf_eob_extra_bits_cover_every_refined_eob() {
    let expected: [(usize, u8, bool, &[u32]); 8] = [
        (3, 2, false, &[]),
        (4, 2, true, &[]),
        (5, 3, false, &[0]),
        (6, 3, false, &[1]),
        (7, 3, true, &[0]),
        (8, 3, true, &[1]),
        (9, 4, false, &[0, 0]),
        (10, 4, false, &[0, 1]),
    ];
    for (eob, eob_pt_sym, eob_extra, extra_bits) in expected {
        let mags = vec![1u32; eob];
        let quant = scan_block(eob, &mags);
        let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

        assert!(
            matches!(trace[1], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16)
                    && c.symbol() == eob_pt_sym),
            "eob {eob}: eob_pt_16 symbol {eob_pt_sym}"
        );
        assert!(
            matches!(trace[2], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra)
                    && c.symbol() == u8::from(eob_extra)),
            "eob {eob}: eob_extra flag {eob_extra}"
        );
        for (offset, &bit) in extra_bits.iter().enumerate() {
            assert_eq!(
                trace[3 + offset],
                BlockSymbolToken::Bypass {
                    width: 1,
                    value: bit
                },
                "eob {eob}: eob_extra_bit[{offset}] == {bit}"
            );
        }
        let header_len = 3 + extra_bits.len();
        assert!(
            matches!(trace[header_len], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBaseEob)),
            "eob {eob}: the base pass starts after the {header_len}-token header"
        );

        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert!(!proof.bytes().is_empty(), "eob {eob}");
        assert_eq!(
            recover_quant_from_tokens(&trace, Q_CTX).unwrap(),
            quant,
            "eob {eob} recover"
        );
    }
}

/// BOUNDED routing fuzz for the larger eobs (5..=10): the eob 3..=4 exhaustive sweep
/// would explode (5^9 lower combinations for eob 10), so this exercises every refined
/// eob and every reachable context with a bounded enumeration. For each eob and each
/// nonzero EOB tier it (a) sweeps all lower positions set to the SAME tier (a cheap
/// way to drive every base/`coeff_br` context band at every LF position) and (b)
/// individually walks each lower position across every tier with the others fixed at
/// a base-low `1` (so each position's context derivation is exercised independently).
/// Every block MUST tokenize, roundtrip (no unrouted context), and recover exactly.
#[test]
fn general_lf_eob5_10_bounded_routing_fuzz() {
    let mut covered = 0usize;
    for eob in 5..=10usize {
        let lower = eob - 1;
        for &eob_mag in &NONZERO_TIERS {
            // (a) all lower positions at one shared tier (including 0).
            for &tier in &TIERS {
                let mut mags = [0u32; 10];
                for slot in mags.iter_mut().take(lower) {
                    *slot = tier;
                }
                mags[eob - 1] = eob_mag;
                assert_block_roundtrips(eob, &mags);
                covered += 1;
            }
            // (b) one lower position swept across every tier, the rest fixed at 1.
            for pos in 0..lower {
                for &tier in &TIERS {
                    let mut mags = [1u32; 10];
                    mags[pos] = tier;
                    mags[eob - 1] = eob_mag;
                    assert_block_roundtrips(eob, &mags);
                    covered += 1;
                }
            }
        }
    }
    assert!(
        covered > 0,
        "the bounded fuzz must cover at least one block"
    );
}

#[test]
fn recover_rejects_out_of_range_eob_pt_without_panicking() {
    let trace = vec![
        BlockSymbolToken::Coeff(coded_luma_all_zero_token(Q_CTX)),
        BlockSymbolToken::Coeff(eob_pt_16_token(Q_CTX, EOB_CTX_LUMA_INTRA, 200)),
    ];
    let err = recover_quant_from_tokens(&trace, Q_CTX).unwrap_err();
    assert!(matches!(
        err,
        Error::CoefficientTokenizationMalformedTokenTrace { .. }
    ));
}
