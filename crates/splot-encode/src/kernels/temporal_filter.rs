// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal filter kernels: the weights of the sixteen 16x16 quadrants of a
//! motion-compensated neighbour in one 64x64 block, the weighted sum of its
//! samples, and the normalized result.

use std::simd::Simd;
use std::simd::cmp::SimdOrd as _;
use std::simd::num::SimdUint as _;

/// The 16x16 quadrants of a 64x64 block.
pub(crate) const QUADRANTS: usize = 16;
/// The weight of a prediction equal to the centre frame.
pub(crate) const MAX_WEIGHT: u32 = 1024;
/// Exponent steps of 1/8 in [`WEIGHTS`]; a larger index takes the last.
pub(crate) const STEPS: u64 = 64;
/// `MAX_WEIGHT * exp(-i / 8)`, rounded.
const WEIGHTS: [u32; STEPS as usize + 1] = weights_table();
const LANES: usize = 8;

const fn weights_table() -> [u32; STEPS as usize + 1] {
    /// `exp(-1/8)` in Q30.
    const STEP_Q30: u64 = 947_581_580;
    let mut out = [0; STEPS as usize + 1];
    let mut value: u64 = 1 << 30;
    let mut i = 0;
    while i < out.len() {
        out[i] = ((value * MAX_WEIGHT as u64 + (1 << 29)) >> 30) as u32;
        value = (value * STEP_Q30 + (1 << 29)) >> 30;
        i += 1;
    }
    out
}

/// `out[q] = WEIGHTS[min(errors[q] * distances[q] / max(decay, 1), STEPS)]`.
#[cfg(test)]
pub(crate) fn weights_scalar(
    errors: &[u32; QUADRANTS],
    distances: &[u32; QUADRANTS],
    decay: u32,
    out: &mut [u32; QUADRANTS],
) {
    for ((out, &error), &distance) in out.iter_mut().zip(errors).zip(distances) {
        let index = (u64::from(error) * u64::from(distance) / u64::from(decay.max(1))).min(STEPS);
        *out = WEIGHTS[index as usize];
    }
}

/// [`weights_scalar`] in sixteen `u64` lanes and a table gather.
pub(crate) fn weights_simd(
    errors: &[u32; QUADRANTS],
    distances: &[u32; QUADRANTS],
    decay: u32,
    out: &mut [u32; QUADRANTS],
) {
    let product = Simd::from_array(*errors).cast::<u64>() * Simd::from_array(*distances).cast();
    let index = (product / Simd::splat(u64::from(decay.max(1)))).simd_min(Simd::splat(STEPS));
    *out = Simd::gather_or_default(&WEIGHTS, index.cast::<usize>()).to_array();
}

/// `accum[i] += weight * pred[i]`, wrapping, over the shorter slice.
#[cfg(test)]
pub(crate) fn accumulate_scalar(pred: &[u16], weight: u32, accum: &mut [u32]) {
    for (accum, &sample) in accum.iter_mut().zip(pred) {
        *accum = accum.wrapping_add(weight.wrapping_mul(u32::from(sample)));
    }
}

/// [`accumulate_scalar`] in eight `u32` lanes.
pub(crate) fn accumulate_simd(pred: &[u16], weight: u32, accum: &mut [u32]) {
    let len = pred.len().min(accum.len());
    let (pred, accum) = (&pred[..len], &mut accum[..len]);
    let weight_v = Simd::<u32, LANES>::splat(weight);
    let mut chunks = accum.chunks_exact_mut(LANES);
    for (acc, samples) in (&mut chunks).zip(pred.chunks_exact(LANES)) {
        let sum = Simd::<u32, LANES>::from_slice(acc)
            + weight_v * Simd::<u16, LANES>::from_slice(samples).cast();
        sum.copy_to_slice(acc);
    }
    let done = len - chunks.into_remainder().len();
    for (accum, &sample) in accum[done..].iter_mut().zip(&pred[done..]) {
        *accum = accum.wrapping_add(weight.wrapping_mul(u32::from(sample)));
    }
}

/// `out[i] = (accum[i] + count / 2) / count`, with `count` at least 1,
/// saturated to `u16`, over the shorter slice.
#[cfg(test)]
pub(crate) fn normalize_scalar(accum: &[u32], count: u32, out: &mut [u16]) {
    let count = count.max(1);
    for (out, &sum) in out.iter_mut().zip(accum) {
        let value = (u64::from(sum) + u64::from(count / 2)) / u64::from(count);
        *out = u16::try_from(value).unwrap_or(u16::MAX);
    }
}

/// [`normalize_scalar`] in eight `u64` lanes.
pub(crate) fn normalize_simd(accum: &[u32], count: u32, out: &mut [u16]) {
    let len = accum.len().min(out.len());
    let (accum, out) = (&accum[..len], &mut out[..len]);
    let count = count.max(1);
    let (half, divisor) = (
        Simd::<u64, LANES>::splat(u64::from(count / 2)),
        Simd::<u64, LANES>::splat(u64::from(count)),
    );
    let mut chunks = out.chunks_exact_mut(LANES);
    for (out, sums) in (&mut chunks).zip(accum.chunks_exact(LANES)) {
        let value = (Simd::<u32, LANES>::from_slice(sums).cast::<u64>() + half) / divisor;
        value
            .simd_min(Simd::splat(u64::from(u16::MAX)))
            .cast::<u16>()
            .copy_to_slice(out);
    }
    let done = len - chunks.into_remainder().len();
    for (out, &sum) in out[done..].iter_mut().zip(&accum[done..]) {
        let value = (u64::from(sum) + half[0]) / u64::from(count);
        *out = u16::try_from(value).unwrap_or(u16::MAX);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn weights_fall_from_the_maximum_to_zero() {
        assert_eq!(WEIGHTS[0], MAX_WEIGHT);
        assert_eq!(WEIGHTS[8], 377, "exp(-1) * 1024");
        assert!(WEIGHTS.windows(2).all(|pair| pair[1] <= pair[0]));
        assert_eq!(WEIGHTS[STEPS as usize], 0);
    }

    proptest! {
        /// Errors, distances and decays of every size, zero decays and
        /// products beyond `u32`.
        #[test]
        fn weights_match_scalar(
            errors in prop::array::uniform16(prop_oneof![3 => 0u32..5_000, 1 => any::<u32>()]),
            distances in prop::array::uniform16(prop_oneof![3 => 16u32..200, 1 => any::<u32>()]),
            decay in prop_oneof![Just(0u32), 1u32..100_000, Just(u32::MAX)],
        ) {
            let (mut scalar, mut simd) = ([0; QUADRANTS], [7; QUADRANTS]);
            weights_scalar(&errors, &distances, decay, &mut scalar);
            weights_simd(&errors, &distances, decay, &mut simd);
            prop_assert_eq!(scalar, simd);
        }

        /// Odd lengths, a longer prediction than accumulator and sums that wrap.
        #[test]
        fn accumulate_matches_scalar(
            pred in prop::collection::vec(0u16..=1023, 0..41),
            start in prop::collection::vec(prop_oneof![3 => 0u32..1 << 20, 1 => any::<u32>()], 0..37),
            weight in prop_oneof![Just(0u32), 0..=MAX_WEIGHT, Just(u32::MAX)],
        ) {
            let (mut scalar, mut simd) = (start.clone(), start);
            accumulate_scalar(&pred, weight, &mut scalar);
            accumulate_simd(&pred, weight, &mut simd);
            prop_assert_eq!(scalar, simd);
        }

        /// Zero and one counts, sums that round both ways and quotients
        /// beyond `u16`.
        #[test]
        fn normalize_matches_scalar(
            accum in prop::collection::vec(prop_oneof![3 => 0u32..1 << 24, 1 => any::<u32>()], 0..43),
            count in prop_oneof![Just(0u32), Just(1u32), 1u32..20_000, Just(u32::MAX)],
            len in 0usize..45,
        ) {
            let (mut scalar, mut simd) = (vec![1; len], vec![2; len]);
            normalize_scalar(&accum, count, &mut scalar);
            normalize_simd(&accum, count, &mut simd);
            let shared = len.min(accum.len());
            prop_assert_eq!(&scalar[..shared], &simd[..shared]);
        }
    }
}
