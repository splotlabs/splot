// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use splot_recon::{BitDepth, dequant_coefficient, max_quantizer_index, quantizer_value};

use super::super::{EncoderKernels, ScalarKernels, SimdKernels};
use super::{MAX_LEVEL, Quantizer};

fn bit_depth(ten: bool) -> BitDepth {
    if ten { BitDepth::Ten } else { BitDepth::Eight }
}

/// A quantizer from § 7.14.2 table steps: `(quantizer, dc_q2, ac_q2, dq_denom, bit_depth)`.
fn quantizer(
    rounding: impl Strategy<Value = u32>,
) -> impl Strategy<Value = (Quantizer, u32, u32, u32, BitDepth)> {
    (any::<bool>(), 0u32..=1000, -64i32..=64, 0u32..=2, rounding).prop_map(
        |(ten, qindex, delta, denom_log2, rounding)| {
            let bd = bit_depth(ten);
            let qindex = qindex % (max_quantizer_index(bd) + 1);
            let (dc, ac) = (
                quantizer_value(qindex, delta, bd),
                quantizer_value(qindex, 0, bd),
            );
            let denom = 1 << denom_log2;
            (
                Quantizer::new(dc, ac, denom, bd, rounding).unwrap(),
                dc,
                ac,
                denom,
                bd,
            )
        },
    )
}

fn coefficient() -> impl Strategy<Value = i32> {
    prop_oneof![
        8 => -4096i32..=4096,
        4 => -(1i32 << 18)..=(1 << 18),
        1 => any::<i32>(),
    ]
}

fn coefficients() -> impl Strategy<Value = Vec<i32>> {
    prop_oneof![
        proptest::collection::vec(coefficient(), 16),
        proptest::collection::vec(coefficient(), 1..80),
        proptest::collection::vec(coefficient(), 1024),
    ]
}

fn run(k: &dyn EncoderKernels, q: &Quantizer, coeffs: &[i32]) -> (Vec<i32>, Vec<i32>, usize) {
    let mut levels = vec![0; coeffs.len()];
    let mut dequant = vec![0; coeffs.len()];
    let nonzero = k.quantize(q, coeffs, &mut levels, &mut dequant).unwrap();
    (levels, dequant, nonzero)
}

proptest! {
    #[test]
    fn simd_matches_scalar((q, ..) in quantizer(0u32..=128), coeffs in coefficients()) {
        prop_assert_eq!(run(&SimdKernels, &q, &coeffs), run(&ScalarKernels, &q, &coeffs));
    }

    #[test]
    fn dequant_is_the_decoder_value_and_levels_are_in_range(
        (q, dc, ac, denom, bd) in quantizer(0u32..=128),
        coeffs in coefficients(),
    ) {
        let (levels, dequant, nonzero) = run(&SimdKernels, &q, &coeffs);
        prop_assert_eq!(nonzero, levels.iter().filter(|&&l| l != 0).count());
        for (i, (&level, &dq)) in levels.iter().zip(&dequant).enumerate() {
            let q2 = if i == 0 { dc } else { ac };
            prop_assert!(level.unsigned_abs() <= MAX_LEVEL);
            prop_assert!(u64::from(level.unsigned_abs()) * u64::from(q2) < 1 << 24);
            prop_assert_eq!(dq, dequant_coefficient(level, q2, denom, bd));
        }
    }

    #[test]
    fn rounding_one_half_picks_the_nearest_level(
        (q, dc, ac, denom, bd) in quantizer(Just(128u32)),
        coeffs in coefficients(),
    ) {
        let limit = (1i64 << (7 + bd.bits())) - 1;
        let (levels, dequant, _) = run(&SimdKernels, &q, &coeffs);
        for (i, ((&c, &level), &dq)) in coeffs.iter().zip(&levels).zip(&dequant).enumerate() {
            let (q2, max_level) = if i == 0 { (dc, q.dc.max_level) } else { (ac, q.ac.max_level) };
            if i64::from(c).abs() > limit {
                continue;
            }
            let distance = (i64::from(c) - i64::from(dq)).abs();
            for neighbour in [level - 1, level + 1] {
                if neighbour.unsigned_abs() <= max_level {
                    let other = dequant_coefficient(neighbour, q2, denom, bd);
                    prop_assert!(distance <= (i64::from(c) - i64::from(other)).abs(), "c={} level={} neighbour={}", c, level, neighbour);
                }
            }
        }
    }

    #[test]
    fn rounding_zero_never_overshoots((q, ..) in quantizer(Just(0u32)), coeffs in coefficients()) {
        let (_, dequant, _) = run(&ScalarKernels, &q, &coeffs);
        for (&c, &dq) in coeffs.iter().zip(&dequant).filter(|&(&c, _)| c.unsigned_abs() < (1 << 15) - 1) {
            prop_assert!(i64::from(dq).abs() <= i64::from(c).abs());
        }
    }
}

#[test]
fn quantizes_a_known_block() {
    let q = Quantizer::new(64, 40, 1, BitDepth::Eight, 128).unwrap();
    let coeffs = [24, 5, -5, 2, -3, 0, 100, -100];
    let (levels, dequant, nonzero) = run(&SimdKernels, &q, &coeffs);
    assert_eq!(levels, [3, 1, -1, 0, -1, 0, 20, -20]);
    assert_eq!(dequant, [24, 5, -5, 0, -5, 0, 100, -100]);
    assert_eq!(nonzero, 6);
}

#[test]
fn rejects_invalid_parameters() {
    assert!(Quantizer::new(31, 40, 4, BitDepth::Eight, 128).is_err());
    assert!(Quantizer::new(64, 64, 3, BitDepth::Eight, 128).is_err());
    assert!(Quantizer::new(64, 64, 1, BitDepth::Eight, 129).is_err());
    assert!(Quantizer::new(64, 1 << 24, 1, BitDepth::Eight, 0).is_err());
    let q = Quantizer::new(64, 64, 1, BitDepth::Eight, 64).unwrap();
    assert!(
        SimdKernels
            .quantize(&q, &[0; 16], &mut [0; 15], &mut [0; 16])
            .is_err()
    );
}

#[test]
fn denominator_follows_the_transform_area() {
    assert_eq!(Quantizer::dq_denom_without_tcq(4, 4), 1);
    assert_eq!(Quantizer::dq_denom_without_tcq(5, 4), 2);
    assert_eq!(Quantizer::dq_denom_without_tcq(5, 5), 2);
    assert_eq!(Quantizer::dq_denom_without_tcq(6, 5), 4);
    assert_eq!(Quantizer::dq_denom_without_tcq(2, 6), 1);
}

/// Every coefficient magnitude up to past the `Clip3` bound, for the smallest,
/// a mid, and the largest table steps.
#[test]
fn simd_matches_scalar_exhaustively() {
    let cases = [
        (BitDepth::Eight, 40),
        (BitDepth::Eight, 41),
        (BitDepth::Eight, 1600),
        (BitDepth::Ten, 57),
        (BitDepth::Ten, 245_760),
    ];
    for (bd, q2) in cases {
        for denom in [1, 2, 4] {
            let q = Quantizer::new(q2, q2, denom, bd, 128).unwrap();
            let bound = 1i32 << (7 + bd.bits());
            let coeffs: Vec<i32> = (-bound - 8..=bound + 8).collect();
            for chunk in coeffs.chunks(1024) {
                assert_eq!(
                    run(&SimdKernels, &q, chunk),
                    run(&ScalarKernels, &q, chunk),
                    "q2={q2} denom={denom}"
                );
            }
        }
    }
}
