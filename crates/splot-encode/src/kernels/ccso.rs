// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CCSO statistics: each sample takes the § 7.19.1 class of its deblocked luma
//! centre and taps with `ccso_edge_clf = 0` and eight bands, and the class
//! accumulates `source - filtered` and a sample count. Coarser band and edge
//! classes are sums of these classes.
//!
//! The SIMD form classifies 16 samples at a time in `u16` lanes, as the
//! decoder does, and adds the lanes to their classes in lane order.

use std::simd::cmp::SimdPartialOrd as _;
use std::simd::num::SimdUint as _;
use std::simd::{Select as _, Simd};

/// Edge classes of one tap (`CCSO_INPUT_INTERVAL`).
pub(crate) const EDGE_CLASSES: usize = 3;
/// Bands of the statistics: `ccso_max_band_log2 = 3`.
pub(crate) const BANDS: usize = 8;
/// Classes of a table: `(cls0 * EDGE_CLASSES + cls1) * BANDS + band`.
pub(crate) const CLASSES: usize = EDGE_CLASSES * EDGE_CLASSES * BANDS;

const LANES: usize = 16;

/// Per-class sums of `source - filtered` and sample counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CcsoStats {
    pub(crate) sum: [i64; CLASSES],
    pub(crate) count: [u32; CLASSES],
}

impl Default for CcsoStats {
    fn default() -> Self {
        Self {
            sum: [0; CLASSES],
            count: [0; CLASSES],
        }
    }
}

/// One row of samples: the deblocked luma centre and taps of each sample, and
/// the source and filtered samples. The shortest slice sets the length.
#[derive(Clone, Copy)]
pub(crate) struct CcsoRow<'a> {
    pub(crate) center: &'a [u16],
    pub(crate) tap0: &'a [u16],
    pub(crate) tap1: &'a [u16],
    pub(crate) source: &'a [u16],
    pub(crate) filtered: &'a [u16],
}

impl CcsoRow<'_> {
    fn len(&self) -> usize {
        [self.tap0, self.tap1, self.source, self.filtered]
            .iter()
            .fold(self.center.len(), |len, row| len.min(row.len()))
    }
}

/// The class of `center` and its taps (§ 7.19.1 `ccso_score` with
/// `edgeClassifier = 0`).
fn class_of(center: u16, taps: [u16; 2], quant_step: u16, band_shift: u32) -> usize {
    let score = |tap: u16| {
        let diff = i32::from(tap) - i32::from(center);
        if diff > i32::from(quant_step) {
            2
        } else {
            usize::from(diff >= -i32::from(quant_step))
        }
    };
    (score(taps[0]) * EDGE_CLASSES + score(taps[1])) * BANDS
        + usize::from(center >> band_shift) % BANDS
}

/// Accumulates samples `range` of `row` one at a time.
fn accumulate_samples(
    row: CcsoRow<'_>,
    range: core::ops::Range<usize>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats,
) {
    for x in range {
        let class = class_of(
            row.center[x],
            [row.tap0[x], row.tap1[x]],
            quant_step,
            band_shift,
        );
        stats.sum[class] += i64::from(row.source[x]) - i64::from(row.filtered[x]);
        stats.count[class] += 1;
    }
}

/// The scalar reference of [`accumulate_simd`].
#[cfg(test)]
pub(crate) fn accumulate_scalar(
    row: CcsoRow<'_>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats,
) {
    accumulate_samples(row, 0..row.len(), quant_step, band_shift, stats);
}

/// Accumulates `row` into `stats`; `band_shift` is `BitDepth - 3`.
pub(crate) fn accumulate_simd(
    row: CcsoRow<'_>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats,
) {
    let len = row.len();
    let q = Simd::<u16, LANES>::splat(quant_step);
    let (one, zero) = (Simd::<u16, LANES>::splat(1), Simd::splat(0));
    let classify = |tap: Simd<u16, LANES>, center: Simd<u16, LANES>| {
        tap.saturating_add(q).simd_ge(center).select(one, zero)
            + tap.simd_gt(center.saturating_add(q)).select(one, zero)
    };
    let load = |samples: &[u16], x: usize| Simd::<u16, LANES>::from_slice(&samples[x..x + LANES]);
    let mut x = 0;
    while x + LANES <= len {
        let center = load(row.center, x);
        let band = (center >> band_shift as u16) & Simd::splat(BANDS as u16 - 1);
        let class = (classify(load(row.tap0, x), center) * Simd::splat(EDGE_CLASSES as u16)
            + classify(load(row.tap1, x), center))
            * Simd::splat(BANDS as u16)
            + band;
        let diff = load(row.source, x).cast::<i32>() - load(row.filtered, x).cast::<i32>();
        for (&class, &diff) in class.as_array().iter().zip(diff.as_array()) {
            stats.sum[usize::from(class)] += i64::from(diff);
            stats.count[usize::from(class)] += 1;
        }
        x += LANES;
    }
    accumulate_samples(row, x..len, quant_step, band_shift, stats);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn case() -> impl Strategy<Value = (Vec<[u16; 5]>, u16, u32)> {
        (8u32..=12).prop_flat_map(|bits| {
            let max = (1u16 << bits) - 1;
            (
                proptest::collection::vec(
                    (0..=max, 0..=max, 0..=max, 0..=max, 0..=max)
                        .prop_map(|(a, b, c, d, e)| [a, b, c, d, e]),
                    0..80,
                ),
                prop_oneof![Just(0u16), Just(8), Just(16), Just(48), Just(256)],
                Just(bits - 3),
            )
        })
    }

    proptest! {
        #[test]
        fn simd_matches_scalar((samples, quant_step, band_shift) in case()) {
            let column = |i: usize| samples.iter().map(|s| s[i]).collect::<Vec<_>>();
            let planes: Vec<Vec<u16>> = (0..5).map(column).collect();
            let row = CcsoRow {
                center: &planes[0],
                tap0: &planes[1],
                tap1: &planes[2],
                source: &planes[3],
                filtered: &planes[4],
            };
            let (mut scalar, mut simd) = (CcsoStats::default(), CcsoStats::default());
            accumulate_scalar(row, quant_step, band_shift, &mut scalar);
            accumulate_simd(row, quant_step, band_shift, &mut simd);
            prop_assert_eq!(&scalar, &simd);
            prop_assert_eq!(scalar.count.iter().sum::<u32>() as usize, samples.len());
        }
    }

    /// Taps above, at and below the quantization window land in classes 2, 1
    /// and 0, and the band is the top three bits of the centre.
    #[test]
    fn classes_follow_the_decoder_score() {
        let center = [512u16; 3];
        let row = CcsoRow {
            center: &center,
            tap0: &[530, 512, 490],
            tap1: &[512, 400, 600],
            source: &[10, 20, 30],
            filtered: &[4, 25, 30],
        };
        let mut stats = CcsoStats::default();
        accumulate_simd(row, 16, 7, &mut stats);
        let class = |cls0: usize, cls1: usize| (cls0 * EDGE_CLASSES + cls1) * BANDS + 4;
        assert_eq!(stats.sum[class(2, 1)], 6);
        assert_eq!(stats.sum[class(1, 0)], -5);
        assert_eq!(stats.count[class(0, 2)], 1);
    }
}
