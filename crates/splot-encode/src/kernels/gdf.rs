// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Squared error of a GDF row at every `GdfPixScale` (§ 7.20.5).
//!
//! A GDF sample is `Clip1(lr + Round2Signed(err * scale, shift))`, and `err`
//! does not depend on the scale, so one `err` row gives all four. Every sum
//! is exact: `|err * 4| < 2^15`, and a squared `u16` difference fits `u32`
//! before it widens into `u64` lanes.

use std::simd::Simd;
use std::simd::cmp::SimdOrd as _;
use std::simd::num::{SimdInt as _, SimdUint as _};

const LANES: usize = 8;

/// `Round2Signed(err * scale, shift)` (§ 4.7).
fn residual(err: i32, scale: i32, shift: u32) -> i32 {
    let value = err * scale;
    if shift == 0 {
        value
    } else if value >= 0 {
        (value + (1 << (shift - 1))) >> shift
    } else {
        -((-value + (1 << (shift - 1))) >> shift)
    }
}

/// Adds `(sample - src)^2` of `lr` (index 0) and of GDF at scales 1 to 4
/// (indices 1 to 4) over `range`.
fn accumulate_samples(
    (lr, err, src): (&[u16], &[i16], &[u16]),
    range: core::ops::Range<usize>,
    (shift, max): (u32, u16),
    sums: &mut [u64; 5],
) {
    for x in range {
        let (base, target) = (i32::from(lr[x]), i64::from(src[x]));
        let square = |sample: i32| (i64::from(sample) - target).pow(2) as u64;
        sums[0] += square(base);
        for scale in 1..=4 {
            let sample =
                (base + residual(i32::from(err[x]), scale, shift)).clamp(0, i32::from(max));
            sums[scale as usize] += square(sample);
        }
    }
}

/// The scalar reference of [`scale_sse_simd`].
#[cfg(test)]
pub(crate) fn scale_sse_scalar(
    rows: (&[u16], &[i16], &[u16]),
    (shift, max): (u32, u16),
    sums: &mut [u64; 5],
) {
    let len = rows.0.len().min(rows.1.len()).min(rows.2.len());
    accumulate_samples(rows, 0..len, (shift, max), sums);
}

/// Adds to `sums` the squared error against `src` of the samples `lr` and of
/// `lr` with GDF `err` at `GdfPixScale` 1 to 4, over the shortest row;
/// `shift` is `12 - BitDepth` and `max` the largest sample.
pub(crate) fn scale_sse_simd(
    (lr, err, src): (&[u16], &[i16], &[u16]),
    (shift, max): (u32, u16),
    sums: &mut [u64; 5],
) {
    let len = lr.len().min(err.len()).min(src.len());
    let half = if shift == 0 { 0 } else { 1 << (shift - 1) };
    let (half, top) = (Simd::<i32, LANES>::splat(half), Simd::splat(i32::from(max)));
    let mut acc = [Simd::<u64, LANES>::splat(0); 5];
    let mut x = 0;
    while x + LANES <= len {
        let base = Simd::<u16, LANES>::from_slice(&lr[x..x + LANES]).cast::<i32>();
        let target = Simd::<u16, LANES>::from_slice(&src[x..x + LANES]).cast::<i32>();
        let e = Simd::<i16, LANES>::from_slice(&err[x..x + LANES]).cast::<i32>();
        let square = |sample: Simd<i32, LANES>| {
            let d = (sample - target).abs().cast::<u32>();
            (d * d).cast::<u64>()
        };
        acc[0] += square(base);
        for scale in 1..=4 {
            let value = e * Simd::splat(scale);
            let rounded = if shift == 0 {
                value
            } else {
                let magnitude = (value.abs() + half) >> Simd::splat(shift as i32);
                value.signum() * magnitude
            };
            let sample = (base + rounded).simd_clamp(Simd::splat(0), top);
            acc[scale as usize] += square(sample);
        }
        x += LANES;
    }
    for (sum, acc) in sums.iter_mut().zip(acc) {
        *sum += acc.reduce_sum();
    }
    accumulate_samples((lr, err, src), x..len, (shift, max), sums);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Rows of different lengths below `2^bits` and `err` in the table range.
    fn rows() -> impl Strategy<Value = (Vec<u16>, Vec<i16>, Vec<u16>, u32, u16)> {
        prop_oneof![Just(8u32), Just(10), Just(12)].prop_flat_map(|bits| {
            let max = ((1u32 << bits) - 1) as u16;
            (
                proptest::collection::vec(0..=max, 0..40),
                proptest::collection::vec(-60i16..=60, 0..40),
                proptest::collection::vec(0..=max, 0..40),
                Just(12 - bits),
                Just(max),
            )
        })
    }

    proptest! {
        #[test]
        fn simd_matches_scalar((lr, err, src, shift, max) in rows()) {
            let (mut simd, mut scalar) = ([3u64; 5], [3u64; 5]);
            scale_sse_simd((&lr, &err, &src), (shift, max), &mut simd);
            scale_sse_scalar((&lr, &err, &src), (shift, max), &mut scalar);
            prop_assert_eq!(simd, scalar);
        }
    }

    /// `Round2Signed` rounds half away from zero; the GDF samples clip.
    #[test]
    fn scales_round_and_clip_like_the_decoder() {
        assert_eq!(residual(3, 4, 4), 1);
        assert_eq!(residual(-3, 4, 4), -1);
        assert_eq!(residual(2, 4, 4), 1);
        assert_eq!(residual(-2, 4, 4), -1);
        assert_eq!(residual(1, 4, 4), 0);
        let mut sums = [0; 5];
        scale_sse_simd((&[250; 9], &[40; 9], &[255; 9]), (4, 255), &mut sums);
        assert_eq!(sums, [9 * 25, 9 * 4, 0, 0, 0]);
    }
}
