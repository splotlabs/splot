// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound type costs from the two single-reference predictions `p0`, `p1`
//! of a block and its source `s`. With `d = s - p1` and `e = p0 - p1`, a
//! blend that gives `p0` the weight `m / 64` leaves the residual
//! `d - m·e / 64`, so its squared error is
//! `Σd² - 2·Σm·d·e / 64 + Σm²·e² / 4096`: every weight, wedge and difference
//! mask is scored from the per-sample terms `d·e` and `e²` without blending.

use std::simd::cmp::SimdOrd as _;
use std::simd::num::{SimdInt, SimdUint as _};
use std::simd::{Simd, u16x8};

const LANES: usize = 8;
/// `38` of the § 7.13.3.28 difference weight.
const DIFF_BASE: u16 = 38;
/// The full weight of a mask sample.
pub(crate) const MASK_ONE: u16 = 64;

/// `[Σd², Σd·e, Σe²]` over the shorter of `s`, `p0`, `p1`, with `d·e` and
/// `e²` of each sample into `de` and `ee`.
#[cfg(test)]
pub(crate) fn terms_scalar(
    s: &[u16],
    (p0, p1): (&[u16], &[u16]),
    de: &mut [i32],
    ee: &mut [i32],
) -> [i64; 3] {
    terms_tail(s, (p0, p1), de, ee)
}

/// [`terms_scalar`] one sample at a time; the tail of [`terms_simd`].
fn terms_tail(s: &[u16], (p0, p1): (&[u16], &[u16]), de: &mut [i32], ee: &mut [i32]) -> [i64; 3] {
    let mut sums = [0i64; 3];
    let samples = s
        .iter()
        .zip(p0)
        .zip(p1)
        .zip(de.iter_mut().zip(ee.iter_mut()));
    for (((&s, &a), &b), (de, ee)) in samples {
        let d = i32::from(s) - i32::from(b);
        let e = i32::from(a) - i32::from(b);
        (*de, *ee) = (d * e, e * e);
        sums[0] += i64::from(d * d);
        sums[1] += i64::from(d * e);
        sums[2] += i64::from(e * e);
    }
    sums
}

/// [`terms_scalar`] in eight lanes.
pub(crate) fn terms_simd(
    s: &[u16],
    (p0, p1): (&[u16], &[u16]),
    de: &mut [i32],
    ee: &mut [i32],
) -> [i64; 3] {
    let len = s
        .len()
        .min(p0.len())
        .min(p1.len())
        .min(de.len())
        .min(ee.len());
    let mut acc = [Simd::<i64, LANES>::splat(0); 3];
    let mut start = 0;
    while start + LANES <= len {
        let src: Simd<i32, LANES> = u16x8::from_slice(&s[start..]).cast();
        let pred1: Simd<i32, LANES> = u16x8::from_slice(&p1[start..]).cast();
        let diff = src - pred1;
        let gap = u16x8::from_slice(&p0[start..]).cast::<i32>() - pred1;
        let (dd, dev, eev) = (diff * diff, diff * gap, gap * gap);
        dev.copy_to_slice(&mut de[start..start + LANES]);
        eev.copy_to_slice(&mut ee[start..start + LANES]);
        acc[0] += dd.cast();
        acc[1] += dev.cast();
        acc[2] += eev.cast();
        start += LANES;
    }
    let tail = terms_tail(
        &s[start..len],
        (&p0[start..len], &p1[start..len]),
        &mut de[start..len],
        &mut ee[start..len],
    );
    let sums = acc.map(SimdInt::reduce_sum);
    [sums[0] + tail[0], sums[1] + tail[1], sums[2] + tail[2]]
}

/// `[Σm·de, Σm·ee, Σm²·ee]` of the mask `mask` over the terms of
/// [`terms_scalar`].
#[cfg(test)]
pub(crate) fn mask_moments_scalar(mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    mask_moments_tail(mask, de, ee)
}

/// [`mask_moments_scalar`] in eight lanes.
pub(crate) fn mask_moments_simd(mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    let len = mask.len().min(de.len()).min(ee.len());
    let mut acc = [Simd::<i64, LANES>::splat(0); 3];
    let mut i = 0;
    while i + LANES <= len {
        let m: Simd<i64, LANES> = Simd::<u8, LANES>::from_slice(&mask[i..]).cast();
        let d: Simd<i64, LANES> = Simd::<i32, LANES>::from_slice(&de[i..]).cast();
        let e: Simd<i64, LANES> = Simd::<i32, LANES>::from_slice(&ee[i..]).cast();
        let me = m * e;
        acc[0] += m * d;
        acc[1] += me;
        acc[2] += m * me;
        i += LANES;
    }
    let tail = mask_moments_tail(&mask[i..len], &de[i..len], &ee[i..len]);
    let sums = acc.map(SimdInt::reduce_sum);
    [sums[0] + tail[0], sums[1] + tail[1], sums[2] + tail[2]]
}

/// [`mask_moments_scalar`] one sample at a time; the tail of
/// [`mask_moments_simd`].
fn mask_moments_tail(mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    mask.iter()
        .zip(de)
        .zip(ee)
        .fold([0; 3], |[a, b, c], ((&m, &de), &ee)| {
            let m = i64::from(m);
            let (de, ee) = (i64::from(de), i64::from(ee));
            [a + m * de, b + m * ee, c + m * m * ee]
        })
}

/// The § 7.13.3.28 difference weight of `p0` over `p1` at sample precision:
/// `min(38 + (|p0 - p1| >> shift) / 16, 64)`, where `shift` is the bit
/// depth above 8.
#[cfg(test)]
pub(crate) fn diff_mask_scalar(p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]) {
    for ((o, &a), &b) in out.iter_mut().zip(p0).zip(p1) {
        *o = (DIFF_BASE + ((a.abs_diff(b) >> shift) >> 4)).min(MASK_ONE) as u8;
    }
}

/// [`diff_mask_scalar`] in eight lanes.
pub(crate) fn diff_mask_simd(p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]) {
    let len = p0.len().min(p1.len()).min(out.len());
    let shift = u16x8::splat((shift + 4) as u16);
    let mut i = 0;
    while i + LANES <= len {
        let a = u16x8::from_slice(&p0[i..]);
        let b = u16x8::from_slice(&p1[i..]);
        let diff = a.simd_max(b) - a.simd_min(b);
        let weight = (u16x8::splat(DIFF_BASE) + (diff >> shift)).simd_min(u16x8::splat(MASK_ONE));
        let weight: Simd<u8, LANES> = weight.cast();
        weight.copy_to_slice(&mut out[i..i + LANES]);
        i += LANES;
    }
    for j in i..len {
        let diff = p0[j].abs_diff(p1[j]);
        out[j] = (DIFF_BASE + (diff >> shift[0])).min(MASK_ONE) as u8;
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// Source and predictions of different 10-bit ranges, masks of every
        /// weight, odd lengths and both bit-depth shifts.
        #[test]
        fn simd_matches_scalar(
            s in prop::collection::vec(0u16..1024, 0..80),
            p0 in prop::collection::vec(100u16..1024, 80),
            p1 in prop::collection::vec(0u16..700, 80),
            mask in prop::collection::vec(0u8..=64, 80),
            shift in 0u32..3,
        ) {
            let len = s.len();
            let (mut de, mut ee) = (vec![0; len], vec![0; len]);
            let (mut de2, mut ee2) = (vec![0; len], vec![0; len]);
            let scalar = terms_scalar(&s, (&p0, &p1), &mut de, &mut ee);
            let simd = terms_simd(&s, (&p0, &p1), &mut de2, &mut ee2);
            prop_assert_eq!(scalar, simd);
            prop_assert_eq!((&de, &ee), (&de2, &ee2));
            prop_assert_eq!(
                mask_moments_scalar(&mask[..len], &de, &ee),
                mask_moments_simd(&mask[..len], &de, &ee)
            );
            let (mut a, mut b) = (vec![0u8; len], vec![0u8; len]);
            diff_mask_scalar(&p0[..len], &p1[..len], shift, &mut a);
            diff_mask_simd(&p0[..len], &p1[..len], shift, &mut b);
            prop_assert_eq!(a, b);
        }

        /// The squared error of an explicit `m / 64` blend equals the moment
        /// form of the module docs, scaled by 4096.
        #[test]
        fn moments_score_the_blend(
            s in prop::collection::vec(0u16..256, 1..40),
            p0 in prop::collection::vec(30u16..256, 40),
            p1 in prop::collection::vec(0u16..200, 40),
            mask in prop::collection::vec(0u8..=64, 40),
        ) {
            let len = s.len();
            let (mut de, mut ee) = (vec![0; len], vec![0; len]);
            let [dd, _, _] = terms_simd(&s, (&p0, &p1), &mut de, &mut ee);
            let [mde, _, mmee] = mask_moments_simd(&mask[..len], &de, &ee);
            let explicit: i64 = (0..len)
                .map(|i| {
                    let (m, a, b) = (i64::from(mask[i]), i64::from(p0[i]), i64::from(p1[i]));
                    let r = 64 * i64::from(s[i]) - (m * a + (64 - m) * b);
                    r * r
                })
                .sum();
            prop_assert_eq!(explicit, 4096 * dd - 128 * mde + mmee);
        }
    }
}
