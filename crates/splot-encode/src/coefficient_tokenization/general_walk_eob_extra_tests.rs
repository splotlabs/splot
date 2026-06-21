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
        // The EOB coefficient (scan index eob-1) must be nonzero.
        for &eob_mag in &NONZERO_TIERS {
            // Lower positions (scan 0..eob-1) range over the full tier set (incl. 0).
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
    // eob 3: 4 eob-mags * 5^2 lower = 100; eob 4: 4 * 5^3 = 500. Total 600.
    assert_eq!(covered, 600, "expected 600 enumerated in-scope blocks");
}

#[test]
fn general_lf_eob6_eobpt4_exact_eob_signaling() {
    // eob == 6 → eobPt 4 (base 5). offset = 6 - 5 = 1; width = eobPt - 3 = 1.
    // eob_extra = (1 >> 1) & 1 = 0; eob_extra_bits = 1 & 1 = 1 → one `eob_extra_bit`
    // bypass = (1 >> 0) & 1 = 1.
    let quant = scan_block(6, &[1, 1, 1, 1, 1, 1]);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // 4 header (all_zero, eob_pt_16, eob_extra, 1 eob_extra_bit) + 6 base + 6 sign +
    // 2 chroma = 18 tokens.
    assert_eq!(trace.len(), 18);

    // all_zero == 0.
    assert!(matches!(
        trace[0],
        BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::AllZero) && c.symbol() == 0
    ));
    // eob_pt_16 symbol 3 (eobPt 4).
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 3),
        "eob_pt_16 symbol 3 (eobPt 4)"
    );
    // eob_extra flag 0 (the HIGH refinement bit of offset 1, width 1).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0"
    );
    // One `eob_extra_bit` bypass literal = 1 (the LOW bit of offset 1).
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
    // eob == 10 → eobPt 5 (base 9). offset = 10 - 9 = 1; width = eobPt - 3 = 2.
    // eob_extra = (1 >> 2) & 1 = 0; eob_extra_bits = 1 & 3 = 1 (binary 01) → two
    // `eob_extra_bit` bypass literals emitted MSB-first: bit1 = (1 >> 1) & 1 = 0,
    // then bit0 = (1 >> 0) & 1 = 1.
    let quant = scan_block(10, &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    // 5 header (all_zero, eob_pt_16, eob_extra, 2 eob_extra_bit) + 10 base + 10 sign +
    // 2 chroma = 27 tokens.
    assert_eq!(trace.len(), 27);

    // eob_pt_16 symbol 4 (eobPt 5).
    assert!(
        matches!(trace[1], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16) && c.symbol() == 4),
        "eob_pt_16 symbol 4 (eobPt 5)"
    );
    // eob_extra flag 0 (the HIGH refinement bit of offset 1, width 2).
    assert!(
        matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra) && c.symbol() == 0),
        "eob_extra flag 0"
    );
    // The two `eob_extra_bit` bypass literals, MSB-first: 0 then 1.
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
    // Drive every refined eob 3..=10 with all-magnitude-1 coefficients and assert the
    // emitted (eob_pt_16, eob_extra, eob_extra_bit*) header matches the decoder
    // mapping, and the block roundtrips + recovers. The (eob_pt symbol, eob_extra
    // flag, MSB-first eob_extra_bit literals) per eob (base 3/5/9 for eobPt 3/4/5):
    //   eob 3  eobPt3 off0 w0 -> sym2 extra0 []
    //   eob 4  eobPt3 off1 w0 -> sym2 extra1 []
    //   eob 5  eobPt4 off0 w1 -> sym3 extra0 [0]
    //   eob 6  eobPt4 off1 w1 -> sym3 extra0 [1]
    //   eob 7  eobPt4 off2 w1 -> sym3 extra1 [0]
    //   eob 8  eobPt4 off3 w1 -> sym3 extra1 [1]
    //   eob 9  eobPt5 off0 w2 -> sym4 extra0 [0,0]
    //   eob 10 eobPt5 off1 w2 -> sym4 extra0 [0,1]
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

        // trace[1] = eob_pt_16.
        assert!(
            matches!(trace[1], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobPt16)
                    && c.symbol() == eob_pt_sym),
            "eob {eob}: eob_pt_16 symbol {eob_pt_sym}"
        );
        // trace[2] = eob_extra flag.
        assert!(
            matches!(trace[2], BlockSymbolToken::Coeff(c)
                if matches!(c.syntax(), CoefficientTokenSyntax::EobExtra)
                    && c.symbol() == u8::from(eob_extra)),
            "eob {eob}: eob_extra flag {eob_extra}"
        );
        // trace[3..3+width] = the MSB-first eob_extra_bit bypass literals.
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
        // The base pass must start right after the header.
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
    // A malformed trace whose `eob_pt_16` symbol selects an eobPt far beyond the
    // supported range (the symbol is a `u8`) must return a typed error, NOT panic on
    // the `1 << (eobPt - 3)` shift in `read_eob_from_tokens`.
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
