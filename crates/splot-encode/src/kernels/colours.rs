// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The distinct sample values and the range of a block, the statistics that
//! tell screen content from camera content.

use std::simd::Simd;
use std::simd::cmp::{SimdOrd as _, SimdPartialEq as _};
use std::simd::num::SimdUint as _;

/// The most distinct values [`colours_simd`] tracks.
pub(crate) const MAX_COLOURS: usize = 16;
const LANES: usize = 8;

/// `(min(distinct values, limit + 1), max - min)` of `block`, with `limit`
/// clamped to [`MAX_COLOURS`] - 1; `(0, 0)` for an empty block.
#[cfg(test)]
pub(crate) fn colours_scalar(block: &[u16], limit: usize) -> (usize, u16) {
    let limit = limit.min(MAX_COLOURS - 1);
    let mut seen = Vec::new();
    for &sample in block {
        if seen.len() <= limit && !seen.contains(&sample) {
            seen.push(sample);
        }
    }
    let range = match (block.iter().min(), block.iter().max()) {
        (Some(&low), Some(&high)) => high - low,
        _ => 0,
    };
    (seen.len(), range)
}

/// [`colours_scalar`] with the range in eight lanes and the distinct values
/// in one vector compare per sample.
pub(crate) fn colours_simd(block: &[u16], limit: usize) -> (usize, u16) {
    let Some(&first) = block.first() else {
        return (0, 0);
    };
    let limit = limit.min(MAX_COLOURS - 1);
    let (mut low, mut high) = (Simd::<u16, LANES>::splat(first), Simd::splat(first));
    let chunks = block.chunks_exact(LANES);
    let tail = chunks.remainder();
    for chunk in chunks {
        let v = Simd::from_slice(chunk);
        low = low.simd_min(v);
        high = high.simd_max(v);
    }
    let (mut low, mut high) = (low.reduce_min(), high.reduce_max());
    for &sample in tail {
        low = low.min(sample);
        high = high.max(sample);
    }
    let mut palette = Simd::<u16, MAX_COLOURS>::splat(first);
    let mut count = 1;
    for &sample in &block[1..] {
        if count > limit {
            break;
        }
        if !palette.simd_eq(Simd::splat(sample)).any() {
            palette[count] = sample;
            count += 1;
        }
    }
    (count, high - low)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// Blocks of few or many values, skewed toward repeats, of odd
        /// lengths, for every limit.
        #[test]
        fn simd_matches_scalar(
            block in prop::collection::vec(prop_oneof![3 => 40u16..46, 1 => 0u16..1024], 0..80),
            limit in 0..2 * MAX_COLOURS,
        ) {
            prop_assert_eq!(colours_scalar(&block, limit), colours_simd(&block, limit));
        }
    }

    #[test]
    fn counts_two_colours_of_a_text_edge() {
        let block: Vec<u16> = (0..64).map(|i| if i % 8 < 3 { 16 } else { 235 }).collect();
        assert_eq!(colours_simd(&block, 8), (2, 219));
        let ramp: Vec<u16> = (0..64).collect();
        assert_eq!(colours_simd(&ramp, 8), (9, 63));
    }
}
