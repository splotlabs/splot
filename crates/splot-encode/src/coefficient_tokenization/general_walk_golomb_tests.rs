// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General-walk golomb-tail tests (`ENC-COEFF-GENERAL-WALK-GOLOMB`, sub-brick 5e, plus
//! `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI`, sub-brick 5e-ii): a 4x4 luma block with one
//! or more coefficients whose magnitude reaches its position `maxLevel` (LF `8`, HF
//! `6`) and therefore carries the AV2 § 5.20.7.28 `read_quant` golomb tail. With a
//! SINGLE golomb coefficient `hrLevelAvg == 0` when its `read_quant` fires, so the
//! golomb parameter `m = Clip3(1, 6, GetMsb(0)) = 1` — the same case the DC golomb
//! composers (`crates/splot-encode/src/block_symbol_trace/golomb.rs`) implement. With
//! MULTIPLE golomb coefficients the running `hrLevelAvg` predictor is threaded across
//! them in reverse scan (`c = eob-1 .. 0`), so the later golomb coefficients' `m`
//! rises above `1` (their `coeff_rem` widens to `L(m)`).
//!
//! These tests assert the exact § 8.2.5 bypass-bit sequence of the golomb tail
//! (mirrored from the decoder `read_quant` and the spec § 5.20.7.28) — including the
//! m>1 second-coefficient stream — the § 8.2 roundtrip, and that
//! `recover_quant_from_tokens` reproduces the input exactly.
//!
//! HONESTY: the `roundtrip_block_symbol_trace` / `recover_quant_from_tokens` proofs
//! are AV2 § 8.2 SELF-CONSISTENCY — the same code authored the emission and its
//! inverse. They do NOT validate the § 8.3.2 CDF contexts (or the golomb bit values)
//! against a real decoder; the golomb tail's bit VALUES are checked here by exact
//! bypass-stream assertions mirrored from the decoder `read_quant`, and context
//! conformance is deferred to the splot-decode cross-check brick.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::block_symbol_trace::roundtrip_block_symbol_trace;
use crate::coefficient_tokenization::{CoefficientCdfRowSelector, CoefficientTokenSyntax};

/// The coefficient CDF q-context the minimal general walk uses (q-ctx 0).
const Q_CTX: usize = 0;

/// The 4x4 2D scan raster positions for scan indices 0..=15
/// (`[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]`): indices 0..=9 are
/// low-frequency (`row + col < 4`), index 10 (raster 13) is the FIRST high-frequency
/// position.
const SCAN_RASTER_0_15: [usize; 16] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15];

/// A single-coefficient block with `value` at the DC (raster 0), all else zero.
fn dc_block(value: i32) -> [i32; TX_4X4_COEFF_COUNT] {
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];
    quant[0] = value;
    quant
}

/// Builds a signed raster `[i32; 16]` from `eob` magnitudes assigned to scan
/// positions 0..eob, with a deterministic asymmetric sign pattern (scan-even
/// negative, scan-odd positive) so a swapped sign order cannot masquerade as a match
/// (the decode-verify-asymmetric-values lesson).
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

/// Extracts the trailing § 8.2.5 bypass literals of a single-DC-coefficient golomb
/// trace: everything between the `dc_sign` CDF token and the chroma U/V tail. For an
/// eob-1 DC block the trace is `all_zero(0), eob_pt_16(0), coeff_base_eob,
/// coeff_br, dc_sign, <golomb tail bypass bits...>, U, V`.
fn dc_golomb_tail(trace: &[BlockSymbolToken]) -> Vec<BlockSymbolToken> {
    let chroma_start = trace.len() - 2;
    trace[..chroma_start]
        .iter()
        .copied()
        .filter(|t| matches!(t, BlockSymbolToken::Bypass { .. }))
        .collect()
}

#[test]
fn lf_golomb_dc_finite_q_magnitude_10_exact_stream() {
    let quant = dc_block(10);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert!(matches!(trace[2], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBaseEob) && c.symbol() == 4));
    assert!(matches!(trace[3], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::CoeffBr) && c.symbol() == 3));
    assert!(matches!(trace[4], BlockSymbolToken::Coeff(c)
            if matches!(c.syntax(), CoefficientTokenSyntax::DcSign) && c.symbol() == 0));

    assert_eq!(
        dc_golomb_tail(&trace),
        vec![
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(1, 0),
        ]
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn lf_golomb_dc_finite_q_boundary_magnitude_17_exact_stream() {
    let quant = dc_block(-17);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(
        dc_golomb_tail(&trace),
        vec![
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(1, 1),
        ]
    );
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn lf_golomb_dc_prefix_magnitude_18_exact_stream() {
    let quant = dc_block(18);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(
        dc_golomb_tail(&trace),
        vec![
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(2, 0),
        ]
    );
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn lf_golomb_dc_prefix_magnitude_50_exact_stream() {
    let quant = dc_block(50);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    assert_eq!(
        dc_golomb_tail(&trace),
        vec![
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(5, 4),
        ]
    );
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn lf_golomb_dc_prefix_magnitude_525_exact_stream() {
    let quant = dc_block(525);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let mut expected = Vec::new();
    for _ in 0..5 {
        expected.push(BlockSymbolToken::bypass(1, 0)); // cMax q-zeros
    }
    for _ in 0..6 {
        expected.push(BlockSymbolToken::bypass(1, 0)); // golomb-zeros
    }
    expected.push(BlockSymbolToken::bypass(1, 1)); // terminator
    expected.push(BlockSymbolToken::bypass(8, 255)); // coeff_rem L(8)
    assert_eq!(dc_golomb_tail(&trace), expected);

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn hf_golomb_eob_coeff_magnitude_8_exact_stream() {
    let mut mags = [1u32; 11];
    mags[10] = 8;
    let quant = scan_block(11, &mags);
    assert_eq!(quant[13], -8); // scan index 10 is even → negative.
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    assert!(matches!(trace[5], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBaseEob { .. })
                && c.symbol() == 2));
    assert!(matches!(trace[6], BlockSymbolToken::Coeff(c)
            if matches!(c.selector(), CoefficientCdfRowSelector::CoeffBr { ctx: 0, .. })
                && c.symbol() == 3));

    let bypasses: Vec<_> = trace
        .iter()
        .copied()
        .filter(|t| matches!(t, BlockSymbolToken::Bypass { .. }))
        .collect();
    assert!(
        bypasses.windows(3).any(|w| w
            == [
                BlockSymbolToken::bypass(1, 0),
                BlockSymbolToken::bypass(1, 1),
                BlockSymbolToken::bypass(1, 0),
            ]),
        "expected the finite-q golomb tail [0,1,0]; bypasses = {bypasses:?}"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

/// The LF golomb magnitudes the fuzz sweeps (finite-q `8..=17` and golomb-prefix
/// `18..=525`): maxLevel 8, base/boundary/prefix samples.
const LF_GOLOMB_MAGS: [u32; 8] = [8, 10, 17, 18, 19, 50, 200, 525];
/// The HF golomb magnitudes the fuzz sweeps (HF maxLevel 6): the same finite-q /
/// golomb-prefix structure shifted down by the maxLevel difference.
const HF_GOLOMB_MAGS: [u32; 8] = [6, 8, 15, 16, 17, 48, 198, 523];

#[test]
fn lf_golomb_dc_bounded_fuzz_roundtrips() {
    let mut covered = 0usize;
    for &mag in &LF_GOLOMB_MAGS {
        for sign in [1i32, -1] {
            let quant = dc_block(sign * mag as i32);
            let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
            let proof = roundtrip_block_symbol_trace(&trace).unwrap();
            assert!(
                !proof.bytes().is_empty(),
                "empty proof mag {mag} sign {sign}"
            );
            assert_eq!(
                recover_quant_from_tokens(&trace, Q_CTX).unwrap(),
                quant,
                "recover != input mag {mag} sign {sign}"
            );
            covered += 1;
        }
    }
    assert_eq!(covered, LF_GOLOMB_MAGS.len() * 2);
}

#[test]
fn golomb_coefficient_at_each_position_roundtrips() {
    let mut covered = 0usize;
    for golomb_scan in 0..16 {
        let eob = golomb_scan + 1; // make the golomb coefficient the EOB at least once
        let raster = SCAN_RASTER_0_15[golomb_scan];
        let is_lf = is_lf_position(raster);
        let golomb_mags = if is_lf {
            LF_GOLOMB_MAGS
        } else {
            HF_GOLOMB_MAGS
        };
        for &gmag in &golomb_mags {
            let mut mags = vec![1u32; eob];
            mags[golomb_scan] = gmag;
            let quant = scan_block(eob, &mags);
            let trace = tokenize_general_lf_luma_block(&quant, Q_CTX);
            assert!(
                trace.is_ok(),
                "tokenize failed scan {golomb_scan} mag {gmag}: {trace:?}"
            );
            let trace = trace.unwrap();
            let proof = roundtrip_block_symbol_trace(&trace);
            assert!(
                proof.is_ok(),
                "roundtrip failed scan {golomb_scan} mag {gmag}: {proof:?}"
            );
            assert!(!proof.unwrap().bytes().is_empty());
            let recovered = recover_quant_from_tokens(&trace, Q_CTX);
            assert!(
                recovered.is_ok(),
                "recover failed scan {golomb_scan} mag {gmag}: {recovered:?}"
            );
            assert_eq!(
                recovered.unwrap(),
                quant,
                "recover != input scan {golomb_scan} mag {gmag}"
            );
            covered += 1;
        }
    }
    assert_eq!(covered, 16 * LF_GOLOMB_MAGS.len());
}

#[test]
fn rejects_magnitude_above_golomb_cap() {
    let quant = dc_block(526);
    let err = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CoefficientTokenizationUnsupportedMagnitude {
                magnitude: 526,
                max_magnitude: 525,
                ..
            }
        ),
        "magnitude 526 must be rejected; got {err:?}"
    );
}

#[test]
fn two_lf_golomb_coefficients_drive_m_above_one_exact_stream() {
    let quant = scan_block(2, &[20, 16]);
    assert_eq!(quant[0], -20); // DC
    assert_eq!(quant[4], 16); // AC

    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();

    let bypasses: Vec<_> = trace
        .iter()
        .copied()
        .filter(|t| matches!(t, BlockSymbolToken::Bypass { .. }))
        .collect();
    assert_eq!(
        bypasses,
        vec![
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 0),
            BlockSymbolToken::bypass(1, 1),
            BlockSymbolToken::bypass(2, 0),
        ],
        "two-golomb m=1-then-m=2 exact bypass stream; bypasses = {bypasses:?}"
    );

    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}

#[test]
fn multiple_golomb_coefficients_bounded_fuzz_roundtrips() {
    let cases: &[(usize, &[(usize, u32)])] = &[
        (2, &[(0, 10), (1, 12)]),
        (3, &[(0, 30), (2, 60)]),
        (4, &[(0, 50), (3, 200)]),
        (5, &[(0, 18), (2, 40), (4, 120)]),
        (11, &[(0, 25), (10, 16)]),
        (12, &[(10, 20), (11, 48)]),
        (13, &[(0, 40), (10, 18), (12, 100)]),
    ];

    let mut covered = 0usize;
    for &(eob, golomb) in cases {
        let mut mags = vec![1u32; eob];
        for &(scan, mag) in golomb {
            mags[scan] = mag;
        }
        let quant = scan_block(eob, &mags);
        let trace = tokenize_general_lf_luma_block(&quant, Q_CTX);
        assert!(
            trace.is_ok(),
            "tokenize failed eob {eob} golomb {golomb:?}: {trace:?}"
        );
        let trace = trace.unwrap();
        let proof = roundtrip_block_symbol_trace(&trace);
        assert!(
            proof.is_ok(),
            "roundtrip failed eob {eob} golomb {golomb:?}: {proof:?}"
        );
        assert!(!proof.unwrap().bytes().is_empty());
        let recovered = recover_quant_from_tokens(&trace, Q_CTX);
        assert!(
            recovered.is_ok(),
            "recover failed eob {eob} golomb {golomb:?}: {recovered:?}"
        );
        assert_eq!(
            recovered.unwrap(),
            quant,
            "recover != input eob {eob} golomb {golomb:?}"
        );
        covered += 1;
    }
    assert_eq!(covered, cases.len());
}

#[test]
fn two_golomb_prefix_coefficients_with_high_m_roundtrips() {
    let quant = scan_block(2, &[300, 525]);
    let trace = tokenize_general_lf_luma_block(&quant, Q_CTX).unwrap();
    let proof = roundtrip_block_symbol_trace(&trace).unwrap();
    assert!(!proof.bytes().is_empty());
    assert_eq!(recover_quant_from_tokens(&trace, Q_CTX).unwrap(), quant);
}
