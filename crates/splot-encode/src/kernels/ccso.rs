// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CCSO statistics: each sample takes the § 7.19.1 class of its deblocked luma
//! centre and taps with `ccso_edge_clf = 0` and a power-of-two band count
//! (eight for the edge classes, 64 for band offsets only), and the class
//! accumulates `source - filtered` and a sample count. Coarser band and edge
//! classes are sums of these classes.
//!
//! The SIMD form classifies 16 samples at a time in `u16` lanes, as the
//! decoder does, and adds the lanes to their classes in lane order.

use fearless_simd::{Level, Select as _, Simd, SimdBase as _, SimdInt as _, dispatch, u16x16};

/// Edge classes of one tap (`CCSO_INPUT_INTERVAL`).
pub(crate) const EDGE_CLASSES: usize = 3;
/// Bands of the statistics: `ccso_max_band_log2 = 3`.
pub(crate) const BANDS: usize = 8;
/// Classes of a table: `(cls0 * EDGE_CLASSES + cls1) * BANDS + band`.
pub(crate) const CLASSES: usize = EDGE_CLASSES * EDGE_CLASSES * BANDS;
/// Bands of the band-offset statistics: `ccso_max_band_log2 = 6`
/// (`CCSO_BAND_NUM`).
pub(crate) const BO_BANDS: usize = 64;
/// Classes of a band-offset table, laid out as [`CLASSES`].
pub(crate) const BO_CLASSES: usize = EDGE_CLASSES * EDGE_CLASSES * BO_BANDS;

const LANES: usize = 16;

/// Per-class sums of `source - filtered` and sample counts over `N` classes:
/// `N / (EDGE_CLASSES * EDGE_CLASSES)` bands, a power of two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CcsoStats<const N: usize = CLASSES> {
    pub(crate) sum: [i64; N],
    pub(crate) count: [u32; N],
}

impl<const N: usize> Default for CcsoStats<N> {
    fn default() -> Self {
        Self {
            sum: [0; N],
            count: [0; N],
        }
    }
}

/// The band count of a table of `classes` classes.
const fn bands(classes: usize) -> usize {
    classes / (EDGE_CLASSES * EDGE_CLASSES)
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
fn class_of<const N: usize>(
    center: u16,
    taps: [u16; 2],
    quant_step: u16,
    band_shift: u32,
) -> usize {
    let score = |tap: u16| {
        let diff = i32::from(tap) - i32::from(center);
        if diff > i32::from(quant_step) {
            2
        } else {
            usize::from(diff >= -i32::from(quant_step))
        }
    };
    (score(taps[0]) * EDGE_CLASSES + score(taps[1])) * bands(N)
        + usize::from(center >> band_shift) % bands(N)
}

/// Accumulates samples `range` of `row` one at a time.
fn accumulate_samples<const N: usize>(
    row: CcsoRow<'_>,
    range: core::ops::Range<usize>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats<N>,
) {
    for x in range {
        let class = class_of::<N>(
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
pub(crate) fn accumulate_scalar<const N: usize>(
    row: CcsoRow<'_>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats<N>,
) {
    accumulate_samples(row, 0..row.len(), quant_step, band_shift, stats);
}

/// Accumulates `row` into `stats`; `band_shift` is `BitDepth` minus the
/// base 2 logarithm of the band count of `N`.
pub(crate) fn accumulate_simd<const N: usize>(
    row: CcsoRow<'_>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats<N>,
) {
    dispatch!(Level::baseline(), simd => {
        accumulate_kernel(simd, row, quant_step, band_shift, stats);
    });
}

#[inline]
fn accumulate_kernel<S: Simd, const N: usize>(
    simd: S,
    row: CcsoRow<'_>,
    quant_step: u16,
    band_shift: u32,
    stats: &mut CcsoStats<N>,
) {
    let len = row.len();
    let q = u16x16::splat(simd, quant_step);
    let (one, zero) = (u16x16::splat(simd, 1), u16x16::splat(simd, 0));
    let classify = |tap: u16x16<S>, center: u16x16<S>| {
        tap.saturating_add(q).simd_ge(center).select(one, zero)
            + tap.simd_gt(center.saturating_add(q)).select(one, zero)
    };
    let load = |samples: &[u16], x: usize| u16x16::from_slice(simd, &samples[x..x + LANES]);
    let mut x = 0;
    while x + LANES <= len {
        let center = load(row.center, x);
        let band = (center >> band_shift) & (bands(N) as u16 - 1);
        let class = (classify(load(row.tap0, x), center) * EDGE_CLASSES as u16
            + classify(load(row.tap1, x), center))
            * bands(N) as u16
            + band;
        let (source, filtered) = (load(row.source, x), load(row.filtered, x));
        for ((&class, &source), &filtered) in class.iter().zip(source.iter()).zip(filtered.iter()) {
            stats.sum[usize::from(class)] += i64::from(source) - i64::from(filtered);
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

    /// The SIMD and scalar forms agree at `N` classes, `bits`-bit samples.
    fn check<const N: usize>(
        row: CcsoRow<'_>,
        quant_step: u16,
        bits: u32,
    ) -> Result<(), TestCaseError> {
        let band_shift = bits - bands(N).trailing_zeros();
        let (mut scalar, mut simd) = (CcsoStats::<N>::default(), CcsoStats::<N>::default());
        accumulate_scalar(row, quant_step, band_shift, &mut scalar);
        accumulate_simd(row, quant_step, band_shift, &mut simd);
        prop_assert_eq!(&scalar, &simd);
        prop_assert_eq!(scalar.count.iter().sum::<u32>() as usize, row.len());
        Ok(())
    }

    proptest! {
        /// Every band count of § 5.18.7.12, 1 to 64.
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
            let bits = band_shift + 3;
            check::<9>(row, quant_step, bits)?;
            check::<18>(row, quant_step, bits)?;
            check::<36>(row, quant_step, bits)?;
            check::<CLASSES>(row, quant_step, bits)?;
            check::<144>(row, quant_step, bits)?;
            check::<288>(row, quant_step, bits)?;
            check::<BO_CLASSES>(row, quant_step, bits)?;
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
        let mut stats = CcsoStats::<CLASSES>::default();
        accumulate_simd(row, 16, 7, &mut stats);
        let class = |cls0: usize, cls1: usize| (cls0 * EDGE_CLASSES + cls1) * BANDS + 4;
        assert_eq!(stats.sum[class(2, 1)], 6);
        assert_eq!(stats.sum[class(1, 0)], -5);
        assert_eq!(stats.count[class(0, 2)], 1);
    }
}
