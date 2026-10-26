// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The amount each lookahead block of a frame passes to its references in
//! the temporal model.

use std::simd::Simd;
use std::simd::cmp::SimdOrd as _;
use std::simd::num::SimdUint as _;

const LANES: usize = 4;

/// `out[i] = (intra[i] + received[i]) * kept / max(intra[i], 1)` with
/// `kept = intra[i] - min(intra[i], best[i] + noise)`: the share of a block's
/// cost and of what it received that its best inter prediction, with `noise`
/// added for a coded reference, saves over intra. Sums and products wrap.
#[cfg(test)]
pub(crate) fn propagate_scalar(
    intra: &[u32],
    best: &[u32],
    received: &[u64],
    noise: u32,
    out: &mut [u64],
) {
    for (((out, &intra), &best), &received) in out.iter_mut().zip(intra).zip(best).zip(received) {
        let (intra, best) = (u64::from(intra), u64::from(best));
        let kept = intra - intra.min(best + u64::from(noise));
        *out = intra.wrapping_add(received).wrapping_mul(kept) / intra.max(1);
    }
}

/// [`propagate_scalar`] in four `u64` lanes.
pub(crate) fn propagate_simd(
    intra: &[u32],
    best: &[u32],
    received: &[u64],
    noise: u32,
    out: &mut [u64],
) {
    let len = out.len().min(intra.len()).min(best.len()).min(received.len());
    let (noise, one) = (Simd::splat(u64::from(noise)), Simd::splat(1));
    let mut index = 0;
    while index + LANES <= len {
        let intra_v = Simd::<u32, LANES>::from_slice(&intra[index..]).cast::<u64>();
        let best_v = Simd::<u32, LANES>::from_slice(&best[index..]).cast::<u64>();
        let received_v = Simd::<u64, LANES>::from_slice(&received[index..]);
        let kept = intra_v - intra_v.simd_min(best_v + noise);
        ((intra_v + received_v) * kept / intra_v.simd_max(one))
            .copy_to_slice(&mut out[index..index + LANES]);
        index += LANES;
    }
    for i in index..len {
        let (intra, best) = (u64::from(intra[i]), u64::from(best[i]));
        let kept = intra - intra.min(best + noise[0]);
        out[i] = intra.wrapping_add(received[i]).wrapping_mul(kept) / intra.max(1);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// Odd lengths, inter costs above and below intra, zero intra costs,
        /// and received amounts large enough to wrap the product.
        #[test]
        fn simd_matches_scalar(
            blocks in prop::collection::vec(
                (prop_oneof![1 => Just(0u32), 4 => 0u32..200_000], 0u32..300_000,
                 prop_oneof![2 => 0u64..1 << 20, 1 => any::<u64>()]),
                0..41,
            ),
            noise in prop_oneof![Just(0u32), 0u32..5_000],
        ) {
            let intra: Vec<u32> = blocks.iter().map(|b| b.0).collect();
            let best: Vec<u32> = blocks.iter().map(|b| b.1.min(b.0 + 7)).collect();
            let received: Vec<u64> = blocks.iter().map(|b| b.2).collect();
            let (mut scalar, mut simd) = (vec![0; blocks.len()], vec![0; blocks.len()]);
            propagate_scalar(&intra, &best, &received, noise, &mut scalar);
            propagate_simd(&intra, &best, &received, noise, &mut simd);
            prop_assert_eq!(scalar, simd);
        }
    }

    #[test]
    fn kept_share_scales_cost_and_received() {
        let mut out = [0; 3];
        propagate_scalar(&[100, 100, 0], &[40, 100, 0], &[300, 9, 5], 10, &mut out);
        assert_eq!(out, [(100 + 300) * 50 / 100, 0, 0]);
    }
}
