// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The distinct sample values and the range of a block, the statistics that
//! tell screen content from camera content, and the nearest palette color of
//! each sample.

use fearless_simd::{
    Level, Select as _, Simd, SimdBase as _, SimdMask as _, SimdNarrow as _, SimdSplit as _,
    dispatch, u16x8, u16x16,
};

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
    dispatch!(Level::baseline(), simd => colours_kernel(simd, block, limit))
}

#[inline]
fn colours_kernel<S: Simd>(simd: S, block: &[u16], limit: usize) -> (usize, u16) {
    let Some(&first) = block.first() else {
        return (0, 0);
    };
    let limit = limit.min(MAX_COLOURS - 1);
    let (mut low, mut high) = (u16x8::splat(simd, first), u16x8::splat(simd, first));
    let (chunks, tail) = block.as_chunks::<LANES>();
    for chunk in chunks {
        let v = u16x8::load_array_ref(simd, chunk);
        low = low.min(v);
        high = high.max(v);
    }
    let (mut low, mut high) = (low.reduce_min(), high.reduce_max());
    for &sample in tail {
        low = low.min(sample);
        high = high.max(sample);
    }
    let mut palette = u16x16::splat(simd, first);
    let mut count = 1;
    for &sample in &block[1..] {
        if count > limit {
            break;
        }
        if !palette.simd_eq(sample).any_true() {
            palette[count] = sample;
            count += 1;
        }
    }
    (count, high - low)
}

/// Writes to `out[i]` the index of the color of `colors` nearest
/// `samples[i]`, the lower index on a tie; `0` without colors.
#[cfg(test)]
pub(crate) fn nearest_scalar(samples: &[u16], colors: &[u16], out: &mut [u8]) {
    for (&sample, out) in samples.iter().zip(out) {
        let mut best = (u16::MAX, 0);
        for (index, &color) in colors.iter().enumerate() {
            let distance = sample.abs_diff(color);
            if index == 0 || distance < best.0 {
                best = (distance, index as u8);
            }
        }
        *out = best.1;
    }
}

/// [`nearest_scalar`] over sixteen samples per step.
pub(crate) fn nearest_simd(samples: &[u16], colors: &[u16], out: &mut [u8]) {
    dispatch!(Level::baseline(), simd => nearest_kernel(simd, samples, colors, out));
}

#[inline]
fn nearest_kernel<S: Simd>(simd: S, samples: &[u16], colors: &[u16], out: &mut [u8]) {
    const WIDE: usize = 16;
    let Some((&first, rest)) = colors.split_first() else {
        out.iter_mut()
            .take(samples.len())
            .for_each(|index| *index = 0);
        return;
    };
    let chunks = samples.chunks_exact(WIDE);
    let tail = chunks.remainder();
    let mut outs = out.chunks_exact_mut(WIDE);
    for (chunk, out) in chunks.zip(&mut outs) {
        let v = u16x16::from_slice(simd, chunk);
        let distance = |color: u16| v.max(color) - v.min(color);
        let mut best = distance(first);
        let mut index = u16x16::splat(simd, 0);
        for (i, &color) in rest.iter().enumerate() {
            let d = distance(color);
            let closer = d.simd_lt(best);
            best = closer.select(d, best);
            index = closer.select(u16x16::splat(simd, u16::from(i as u8 + 1)), index);
        }
        let (lo, hi) = index.split();
        lo.narrow(hi).store_slice(out);
    }
    let done = samples.len() - tail.len();
    for (&sample, out) in tail.iter().zip(&mut out[done..]) {
        let mut best = (sample.abs_diff(first), 0);
        for (i, &color) in rest.iter().enumerate() {
            let d = sample.abs_diff(color);
            if d < best.0 {
                best = (d, i as u8 + 1);
            }
        }
        *out = best.1;
    }
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

        /// Samples near and between up to eight ascending colors, with ties,
        /// of odd lengths.
        #[test]
        fn nearest_simd_matches_scalar(
            samples in prop::collection::vec(prop_oneof![3 => 90u16..140, 1 => 0u16..1024], 0..80),
            mut colors in prop::collection::vec(prop_oneof![90u16..140, 0u16..1024], 0..9),
        ) {
            colors.sort_unstable();
            colors.dedup();
            let (mut scalar, mut simd) = (vec![9; samples.len()], vec![9; samples.len()]);
            nearest_scalar(&samples, &colors, &mut scalar);
            nearest_simd(&samples, &colors, &mut simd);
            prop_assert_eq!(scalar, simd);
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
