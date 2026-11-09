// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One chroma row of luma down-sampled by a § 7.13.5 CfL filter.

use fearless_simd::{
    Bytes as _, Level, Simd, SimdBase as _, SimdCombine as _, SimdWiden as _, dispatch, i32x8,
    u16x8,
};

const LANES: usize = 8;

/// `Σ weights[r][k] · rows[r][2i + k − 1]` with each column clamped into its row.
fn tap(weights: &[[i32; 3]; 3], rows: [&[u16]; 3], i: usize) -> i32 {
    let mut sum = 0;
    for (row, taps) in rows.iter().zip(weights) {
        let last = row.len().saturating_sub(1);
        for (k, &weight) in taps.iter().enumerate() {
            let x = (2 * i + k).saturating_sub(1).min(last);
            sum += weight * i32::from(row.get(x).copied().unwrap_or(0));
        }
    }
    sum
}

/// Each `out[i]` is [`tap`] `i`: rows `0..3` are the luma rows above, at and
/// below the chroma row, and `weights[r][k]` weighs column `2i + k − 1`.
#[cfg(test)]
pub(crate) fn downsample_scalar(weights: &[[i32; 3]; 3], rows: [&[u16]; 3], out: &mut [i32]) {
    for (i, sample) in out.iter_mut().enumerate() {
        *sample = tap(weights, rows, i);
    }
}

/// [`downsample_scalar`] in eight lanes where no column clamps.
pub(crate) fn downsample_simd(weights: &[[i32; 3]; 3], rows: [&[u16]; 3], out: &mut [i32]) {
    dispatch!(Level::baseline(), simd => downsample_kernel(simd, weights, rows, out));
}

#[inline]
fn downsample_kernel<S: Simd>(
    simd: S,
    weights: &[[i32; 3]; 3],
    rows: [&[u16]; 3],
    out: &mut [i32],
) {
    let len = rows.iter().map(|row| row.len()).min().unwrap_or(0);
    let mut i = 0;
    while i < out.len() {
        if i == 0 || i + LANES > out.len() || 2 * (i + LANES) > len {
            out[i] = tap(weights, rows, i);
            i += 1;
            continue;
        }
        let mut sum = i32x8::splat(simd, 0);
        for (row, &[left, centre, right]) in rows.iter().zip(weights) {
            let load = |at: usize| {
                let lo = u16x8::from_slice(simd, &row[at..at + LANES]);
                let hi = u16x8::from_slice(simd, &row[at + LANES..at + 2 * LANES]);
                let wide = |v: u16x8<S>| {
                    let (lo, hi) = v.widen();
                    lo.combine(hi).bitcast::<i32x8<S>>()
                };
                let (even, odd) = lo.deinterleave(hi);
                (wide(even), wide(odd))
            };
            let (prev, _) = load(2 * i - 1);
            let (even, odd) = load(2 * i);
            sum += prev * left + even * centre + odd * right;
        }
        sum.store_slice(&mut out[i..i + LANES]);
        i += LANES;
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use splot_decode::encoder_syntax::{CFL_FILTERS_420, CFL_FILTERS_422};

    use super::*;

    proptest! {
        /// Rows of different asymmetric ranges and lengths, odd and short too,
        /// and every 4:2:0 and 4:2:2 filter.
        #[test]
        fn simd_matches_scalar(
            above in prop::collection::vec(0u16..1024, 0..80),
            at in prop::collection::vec(200u16..900, 0..80),
            below in prop::collection::vec(0u16..300, 0..80),
            filter in 0..6usize,
            width in 0..40usize,
        ) {
            let weights = match filter {
                0..3 => CFL_FILTERS_420[filter],
                _ => [[0; 3], CFL_FILTERS_422[filter - 3], [0; 3]],
            };
            let rows = [above.as_slice(), at.as_slice(), below.as_slice()];
            let (mut scalar, mut simd) = (vec![0; width], vec![0; width]);
            downsample_scalar(&weights, rows, &mut scalar);
            downsample_simd(&weights, rows, &mut simd);
            prop_assert_eq!(scalar, simd);
        }
    }

    /// The 4:2:0 filter `1` weighs `1 2 1` around each even column of the
    /// two rows at and below, clamped at the left edge.
    #[test]
    fn taps_columns_around_the_even_sample() {
        let at = [10u16, 20, 30, 40];
        let below = [1u16, 2, 3, 4];
        let mut out = [0; 2];
        downsample_simd(&CFL_FILTERS_420[1], [&[], &at, &below], &mut out);
        assert_eq!(out, [10 + 20 + 20 + 1 + 2 + 2, 20 + 60 + 40 + 2 + 6 + 4]);
    }
}
