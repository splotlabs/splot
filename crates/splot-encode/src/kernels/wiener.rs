// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Normal equations of the luma Wiener-NS filter (§ 7.20.3).
//!
//! The filter adds `sum(coeff[k] * f[k]) / 2^7` to the centre `m`, where
//! `f[k]` is the sum of the differences of the two taps of pair `k` from `m`.
//! A least-squares fit of `source - m` needs `A = sum(f f^T)` and
//! `b = sum(f (source - m))`. The SIMD form takes eight samples at a time in
//! `i32` lanes, carries them across the rows of a unit, and moves them to
//! `i64` sums before they can overflow.

use std::simd::Simd;
use std::simd::num::{SimdInt as _, SimdUint as _};

use splot_decode::encoder_syntax::WIENER_NS_CONFIG_Y_PAIRS;

/// Luma coefficients (`WIENER_NS_LUMA_COEFFS`).
pub(crate) const COEFFS: usize = 16;
/// Entries of the upper triangle of `A`, row by row.
pub(crate) const PAIRS: usize = COEFFS * (COEFFS + 1) / 2;
/// Rows above and below the centre the taps read.
pub(crate) const RADIUS: usize = 4;

const LANES: usize = 8;

/// The normal-equation sums of a set of samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WienerStats {
    /// `sum(f[i] * f[j])` for `i <= j`, row by row.
    pub(crate) a: [i64; PAIRS],
    /// `sum(f[i] * (source - m))`.
    pub(crate) b: [i64; COEFFS],
    /// Samples.
    pub(crate) samples: u64,
}

impl Default for WienerStats {
    fn default() -> Self {
        Self {
            a: [0; PAIRS],
            b: [0; COEFFS],
            samples: 0,
        }
    }
}

impl WienerStats {
    /// Adds the samples of `other`.
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

/// One row of `width` samples: the reconstruction rows `y - 4 ..= y + 4`, each
/// starting four samples left of the first sample and four past the last, and
/// the source row.
#[derive(Clone, Copy)]
pub(crate) struct WienerRow<'a> {
    pub(crate) rows: [&'a [u16]; 2 * RADIUS + 1],
    pub(crate) source: &'a [u16],
}

impl WienerRow<'_> {
    fn len(&self) -> usize {
        self.rows.iter().fold(self.source.len(), |len, row| {
            len.min(row.len().saturating_sub(2 * RADIUS))
        })
    }

    fn sample(&self, dy: isize, x: isize) -> i32 {
        i32::from(self.rows[(RADIUS as isize + dy) as usize][(RADIUS as isize + x) as usize])
    }
}

/// Accumulates samples `range` of `row` one at a time.
fn accumulate_samples(row: WienerRow<'_>, range: core::ops::Range<usize>, stats: &mut WienerStats) {
    stats.samples += range.len() as u64;
    for x in range {
        let x = x as isize;
        let m = row.sample(0, x);
        let f: [i32; COEFFS] = core::array::from_fn(|k| {
            let (dy, dx, _) = WIENER_NS_CONFIG_Y_PAIRS[k];
            row.sample(dy, x + dx) + row.sample(-dy, x - dx) - 2 * m
        });
        let t = i32::from(row.source[x as usize]) - m;
        let mut pair = 0;
        for i in 0..COEFFS {
            stats.b[i] += i64::from(f[i] * t);
            for j in i..COEFFS {
                stats.a[pair] += i64::from(f[i] * f[j]);
                pair += 1;
            }
        }
    }
}

/// The scalar reference of [`WienerAccumulator`].
#[cfg(test)]
pub(crate) fn accumulate_scalar(row: WienerRow<'_>, stats: &mut WienerStats) {
    accumulate_samples(row, 0..row.len(), stats);
}

type V = Simd<i32, LANES>;

/// The SIMD accumulation: `i32` lane sums carried across rows, moved to `i64`
/// before a lane can overflow.
pub(crate) struct WienerAccumulator {
    pairs: [V; PAIRS],
    cross: [V; COEFFS],
    chunks: usize,
    budget: usize,
}

impl WienerAccumulator {
    /// An accumulator for samples up to `max_sample`: each lane product is at
    /// most `(2 * max_sample)^2`, so a lane takes that many chunks of `i32::MAX`.
    pub(crate) fn new(max_sample: u16) -> Self {
        let product = 4 * u64::from(max_sample).pow(2);
        Self {
            pairs: [V::splat(0); PAIRS],
            cross: [V::splat(0); COEFFS],
            chunks: 0,
            budget: (i32::MAX as u64 / product.max(1)).max(1) as usize,
        }
    }

    /// Accumulates `row` into the lanes and, for its tail, into `stats`.
    pub(crate) fn add_row(&mut self, row: WienerRow<'_>, stats: &mut WienerStats) {
        let len = row.len();
        let load = |dy: isize, x: isize| {
            let start = (RADIUS as isize + x) as usize;
            Simd::<u16, LANES>::from_slice(
                &row.rows[(RADIUS as isize + dy) as usize][start..start + LANES],
            )
            .cast::<i32>()
        };
        let mut x = 0;
        stats.samples += (len - len % LANES) as u64;
        while x + LANES <= len {
            if self.chunks == self.budget {
                self.flush(stats);
            }
            let xi = x as isize;
            let centre = load(0, xi);
            let f: [V; COEFFS] = core::array::from_fn(|k| {
                let (dy, dx, _) = WIENER_NS_CONFIG_Y_PAIRS[k];
                load(dy, xi + dx) + load(-dy, xi - dx) - centre - centre
            });
            let t =
                Simd::<u16, LANES>::from_slice(&row.source[x..x + LANES]).cast::<i32>() - centre;
            let mut pair = 0;
            for i in 0..COEFFS {
                self.cross[i] += f[i] * t;
                for j in i..COEFFS {
                    self.pairs[pair] += f[i] * f[j];
                    pair += 1;
                }
            }
            x += LANES;
            self.chunks += 1;
        }
        accumulate_samples(row, x..len, stats);
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

    /// `(max sample, rows of (9 tap rows, source))`.
    type Case = (u16, Vec<(Vec<Vec<u16>>, Vec<u16>)>);

    fn case() -> impl Strategy<Value = Case> {
        (8u32..=12, 0usize..300, 1usize..4).prop_flat_map(|(bits, width, rows)| {
            let max = (1u16 << bits) - 1;
            let row = (
                proptest::collection::vec(
                    proptest::collection::vec(0..=max, width + 2 * RADIUS),
                    2 * RADIUS + 1,
                ),
                proptest::collection::vec(0..=max, width),
            );
            (Just(max), proptest::collection::vec(row, rows))
        })
    }

    fn run(max: u16, rows: &[(Vec<Vec<u16>>, Vec<u16>)]) -> (WienerStats, WienerStats) {
        let (mut scalar, mut simd) = (WienerStats::default(), WienerStats::default());
        let mut lanes = WienerAccumulator::new(max);
        for (taps, source) in rows {
            let row = WienerRow {
                rows: core::array::from_fn(|i| taps[i].as_slice()),
                source,
            };
            accumulate_scalar(row, &mut scalar);
            lanes.add_row(row, &mut simd);
        }
        lanes.flush(&mut simd);
        (scalar, simd)
    }

    proptest! {
        #[test]
        fn simd_matches_scalar((max, rows) in case()) {
            let (scalar, simd) = run(max, &rows);
            prop_assert_eq!(scalar, simd);
        }
    }

    /// Full-range 12-bit taps of opposite sign over long rows reach the flush
    /// bound without overflow.
    #[test]
    fn extreme_rows_do_not_overflow() {
        let width = 8 * 40 + 5;
        let high = vec![4095u16; width + 2 * RADIUS];
        let low = vec![0u16; width + 2 * RADIUS];
        let taps: Vec<Vec<u16>> = (0..=2 * RADIUS)
            .map(|i| {
                if i == RADIUS {
                    low.clone()
                } else {
                    high.clone()
                }
            })
            .collect();
        let rows = vec![(taps, low[..width].to_vec()); 3];
        let (scalar, simd) = run(4095, &rows);
        assert_eq!(scalar, simd);
        assert_eq!(scalar.a[0], (2 * 4095i64).pow(2) * 3 * width as i64);
    }
}
