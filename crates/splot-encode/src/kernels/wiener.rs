// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Normal equations of the Wiener-NS filter (§ 7.20.3).
//!
//! The filter adds `sum(coeff[k] * f[k]) / 2^7` to the centre `m`, where
//! `f[k]` is the tap difference of coefficient `k`. A least-squares fit of
//! `source - m` needs `A = sum(f f^T)` and `b = sum(f (source - m))`. The SIMD
//! form takes eight samples at a time in `i32` lanes, carries them across the
//! rows of a unit, and moves them to `i64` sums before they can overflow.

use std::simd::Simd;
use std::simd::num::SimdInt as _;

const LANES: usize = 8;
type V = Simd<i32, LANES>;

/// The normal-equation sums of a set of samples over `n` coefficients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WienerStats {
    /// `sum(f[i] * f[j])` for `i <= j`, row by row.
    a: Vec<i64>,
    /// `sum(f[i] * (source - m))`.
    pub(crate) b: Vec<i64>,
    /// Samples.
    pub(crate) samples: u64,
}

impl WienerStats {
    /// Empty sums over `n` coefficients.
    pub(crate) fn new(n: usize) -> Self {
        Self {
            a: vec![0; n * (n + 1) / 2],
            b: vec![0; n],
            samples: 0,
        }
    }

    /// The coefficient count.
    pub(crate) fn len(&self) -> usize {
        self.b.len()
    }

    /// `A[i][j]`.
    pub(crate) fn a(&self, i: usize, j: usize) -> i64 {
        let (i, j) = (i.min(j), i.max(j));
        self.a[i * self.len() - i * i.saturating_sub(1) / 2 + j - i]
    }

    /// Adds the samples of `other`, which has the same coefficients.
    pub(crate) fn add(&mut self, other: &Self) {
        for (sum, value) in self.a.iter_mut().zip(&other.a) {
            *sum += value;
        }
        for (sum, value) in self.b.iter_mut().zip(&other.b) {
            *sum += value;
        }
        self.samples += other.samples;
    }
}

/// Accumulates samples `range` of the rows one at a time.
fn accumulate_samples(
    features: &[&[i32]],
    target: &[i32],
    range: core::ops::Range<usize>,
    stats: &mut WienerStats,
) {
    stats.samples += range.len() as u64;
    for x in range {
        let mut pair = 0;
        for (i, fi) in features.iter().enumerate() {
            stats.b[i] += i64::from(fi[x] * target[x]);
            for fj in &features[i..] {
                stats.a[pair] += i64::from(fi[x] * fj[x]);
                pair += 1;
            }
        }
    }
}

/// The scalar reference of [`WienerAccumulator`].
#[cfg(test)]
pub(crate) fn accumulate_scalar(features: &[&[i32]], target: &[i32], stats: &mut WienerStats) {
    accumulate_samples(features, target, 0..target.len(), stats);
}

/// The SIMD accumulation: `i32` lane sums carried across rows, moved to `i64`
/// before a lane can overflow.
pub(crate) struct WienerAccumulator {
    pairs: Vec<V>,
    cross: Vec<V>,
    chunks: usize,
    budget: usize,
}

impl WienerAccumulator {
    /// An accumulator over `n` coefficients whose feature and target products
    /// are at most `max_product` in magnitude.
    pub(crate) fn new(n: usize, max_product: u64) -> Self {
        Self {
            pairs: vec![V::splat(0); n * (n + 1) / 2],
            cross: vec![V::splat(0); n],
            chunks: 0,
            budget: (i32::MAX as u64 / max_product.max(1)).max(1) as usize,
        }
    }

    /// Accumulates the rows `features[k]` and `target`, which have the same
    /// length, into the lanes and, for their tail, into `stats`.
    pub(crate) fn add(&mut self, features: &[&[i32]], target: &[i32], stats: &mut WienerStats) {
        let len = target.len();
        let mut f = vec![V::splat(0); features.len()];
        let mut x = 0;
        stats.samples += (len - len % LANES) as u64;
        while x + LANES <= len {
            if self.chunks == self.budget {
                self.flush(stats);
            }
            for (lanes, row) in f.iter_mut().zip(features) {
                *lanes = V::from_slice(&row[x..x + LANES]);
            }
            let t = V::from_slice(&target[x..x + LANES]);
            let mut pair = 0;
            for (i, &fi) in f.iter().enumerate() {
                self.cross[i] += fi * t;
                for &fj in &f[i..] {
                    self.pairs[pair] += fi * fj;
                    pair += 1;
                }
            }
            x += LANES;
            self.chunks += 1;
        }
        accumulate_samples(features, target, x..len, stats);
    }

    /// Moves the lane sums into `stats`.
    pub(crate) fn flush(&mut self, stats: &mut WienerStats) {
        for (sum, lanes) in stats.a.iter_mut().zip(self.pairs.iter_mut()) {
            *sum += lanes.cast::<i64>().reduce_sum();
            *lanes = V::splat(0);
        }
        for (sum, lanes) in stats.b.iter_mut().zip(self.cross.iter_mut()) {
            *sum += lanes.cast::<i64>().reduce_sum();
            *lanes = V::splat(0);
        }
        self.chunks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// `(coefficients, bound, rows of (features, target))`.
    type Case = (usize, i32, Vec<(Vec<Vec<i32>>, Vec<i32>)>);

    fn case() -> impl Strategy<Value = Case> {
        (
            8u32..=12,
            0usize..300,
            1usize..4,
            prop_oneof![Just(16usize), Just(18)],
        )
            .prop_flat_map(|(bits, width, rows, n)| {
                let max = (1i32 << bits) - 1;
                let row = (
                    proptest::collection::vec(
                        proptest::collection::vec(-2 * max..=2 * max, width),
                        n,
                    ),
                    proptest::collection::vec(-max..=max, width),
                );
                (Just(n), Just(max), proptest::collection::vec(row, rows))
            })
    }

    fn run(n: usize, max: i32, rows: &[(Vec<Vec<i32>>, Vec<i32>)]) -> (WienerStats, WienerStats) {
        let (mut scalar, mut simd) = (WienerStats::new(n), WienerStats::new(n));
        let mut lanes = WienerAccumulator::new(n, 4 * (max as u64).pow(2));
        for (features, target) in rows {
            let features: Vec<&[i32]> = features.iter().map(Vec::as_slice).collect();
            accumulate_scalar(&features, target, &mut scalar);
            lanes.add(&features, target, &mut simd);
        }
        lanes.flush(&mut simd);
        (scalar, simd)
    }

    proptest! {
        #[test]
        fn simd_matches_scalar((n, max, rows) in case()) {
            let (scalar, simd) = run(n, max, &rows);
            prop_assert_eq!(scalar, simd);
        }
    }

    /// Full-range 12-bit features of one sign over long rows reach the flush
    /// bound without overflow.
    #[test]
    fn extreme_rows_do_not_overflow() {
        let width = 8 * 40 + 5;
        let rows = vec![(vec![vec![8190; width]; 16], vec![-4095; width]); 3];
        let (scalar, simd) = run(16, 4095, &rows);
        assert_eq!(scalar, simd);
        assert_eq!(scalar.a(0, 15), 8190i64.pow(2) * 3 * width as i64);
    }
}
