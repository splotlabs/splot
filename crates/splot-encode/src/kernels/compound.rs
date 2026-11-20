// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound type costs from the two single-reference predictions `p0`, `p1`
//! of a block and its source `s`. With `d = s - p1` and `e = p0 - p1`, a
//! blend that gives `p0` the weight `m / 64` leaves the residual
//! `d - m·e / 64`, so its squared error is
//! `Σd² - 2·Σm·d·e / 64 + Σm²·e² / 4096`: every weight, wedge and difference
//! mask is scored from the per-sample terms `d·e` and `e²` without blending.

use fearless_simd::{
    Level, Simd, SimdBase, SimdIntWiden as _, SimdNarrow as _, SimdWiden as _, dispatch, i32x8,
    i64x4, u16x8,
};

use super::load_u16_as_i32;

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
    dispatch!(Level::baseline(), simd => terms_kernel(simd, s, (p0, p1), de, ee))
}

#[inline]
fn terms_kernel<S: Simd>(
    simd: S,
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
    let mut acc = [i64x4::splat(simd, 0); 3];
    let samples = s[..len].as_chunks::<LANES>().0.iter();
    let preds = p0[..len]
        .as_chunks::<LANES>()
        .0
        .iter()
        .zip(p1[..len].as_chunks::<LANES>().0);
    let terms = de[..len].as_chunks_mut::<LANES>().0.iter_mut();
    let terms = terms.zip(ee[..len].as_chunks_mut::<LANES>().0);
    for ((s, (p0, p1)), (de, ee)) in samples.zip(preds).zip(terms) {
        let pred1 = load_u16_as_i32(simd, p1);
        let diff = load_u16_as_i32(simd, s) - pred1;
        let gap = load_u16_as_i32(simd, p0) - pred1;
        let (dd, dev, eev) = (diff * diff, diff * gap, gap * gap);
        dev.store_slice(de);
        eev.store_slice(ee);
        for (acc, term) in acc.iter_mut().zip([dd, dev, eev]) {
            let (lo, hi) = term.widen();
            *acc += lo + hi;
        }
    }
    let start = len - len % LANES;
    let tail = terms_tail(
        &s[start..len],
        (&p0[start..len], &p1[start..len]),
        &mut de[start..len],
        &mut ee[start..len],
    );
    let sums = acc.map(SimdBase::reduce_sum);
    [sums[0] + tail[0], sums[1] + tail[1], sums[2] + tail[2]]
}

/// `[Σm·de, Σm·ee, Σm²·ee]` of the mask `mask` over the terms of
/// [`terms_scalar`].
#[cfg(test)]
pub(crate) fn mask_moments_scalar(mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    mask_moments_tail(mask, de, ee)
}

/// [`mask_moments_scalar`] in eight lanes. Every product is of two values
/// that fit `i32` (`m·m` is below `2^16`), so it is one widening multiply.
pub(crate) fn mask_moments_simd(mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    dispatch!(Level::baseline(), simd => mask_moments_kernel(simd, mask, de, ee))
}

#[inline]
fn mask_moments_kernel<S: Simd>(simd: S, mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
    let len = mask.len().min(de.len()).min(ee.len());
    let mut acc = [i64x4::splat(simd, 0); 3];
    let mut i = 0;
    while i + LANES <= len {
        let m = i32x8::from_fn(simd, |k| i32::from(mask[i + k]));
        let d = i32x8::from_slice(simd, &de[i..i + LANES]);
        let e = i32x8::from_slice(simd, &ee[i..i + LANES]);
        for (acc, (lo, hi)) in acc.iter_mut().zip([
            m.widening_mul(d),
            m.widening_mul(e),
            (m * m).widening_mul(e),
        ]) {
            *acc += lo + hi;
        }
        i += LANES;
    }
    let tail = mask_moments_tail(&mask[i..len], &de[i..len], &ee[i..len]);
    let sums = acc.map(SimdBase::reduce_sum);
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
    dispatch!(Level::baseline(), simd => diff_mask_kernel(simd, p0, p1, shift, out));
}

#[inline]
fn diff_mask_kernel<S: Simd>(simd: S, p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]) {
    let len = p0.len().min(p1.len()).min(out.len());
    let shift = shift + 4;
    let mut i = 0;
    while i + LANES <= len {
        let a = u16x8::from_slice(simd, &p0[i..i + LANES]);
        let b = u16x8::from_slice(simd, &p1[i..i + LANES]);
        let diff = a.max(b) - a.min(b);
        let weight = ((diff >> shift) + DIFF_BASE).min(MASK_ONE);
        out[i..i + LANES].copy_from_slice(&weight.narrow(weight).as_slice()[..LANES]);
        i += LANES;
    }
    for j in i..len {
        let diff = p0[j].abs_diff(p1[j]);
        out[j] = (DIFF_BASE + (diff >> shift)).min(MASK_ONE) as u8;
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

        /// Mask moments of any `u8` mask over any `i32` terms, of different
        /// lengths, need the full widening products.
        #[test]
        fn mask_moments_simd_matches_scalar_over_every_value(
            mask in prop::collection::vec(any::<u8>(), 0..80),
            de in prop::collection::vec(any::<i32>(), 0..80),
            ee in prop::collection::vec(any::<i32>(), 0..80),
        ) {
            prop_assert_eq!(mask_moments_scalar(&mask, &de, &ee), mask_moments_simd(&mask, &de, &ee));
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
