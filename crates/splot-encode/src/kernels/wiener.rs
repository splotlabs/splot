// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Normal equations of the Wiener-NS filter (§ 7.20.3).
//!
//! The filter adds `sum(coeff[k] * f[k]) / 2^7` to the centre `m`, where
//! `f[k]` is the tap difference of coefficient `k`. A least-squares fit of
//! `source - m` needs `A = sum(f f^T)` and `b = sum(f (source - m))`. The SIMD
//! form takes eight samples at a time in `i32` lanes, carries them across the
//! rows of a unit, and moves them to `i64` sums before they can overflow.

use fearless_simd::{Level, Simd, SimdBase, dispatch, i32x4, i32x8};

use super::load_u16_as_i32;

const LANES: usize = 8;

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

/// Adds `f[i] * t` and `f[i] * f[j]` (`i <= j`) of the `L` samples at `x`
/// to the lane sums.
#[inline]
fn add_products<S, V, const L: usize, const N: usize>(
    simd: S,
    (pairs, cross): (&mut [[[i32; L]; N]; N], &mut [[i32; L]; N]),
    features: &[&[i32]; N],
    target: &[i32],
    x: usize,
) where
    S: Simd,
    V: SimdBase<S, Element = i32, Array = [i32; L]>,
{
    let add = |lanes: &mut [i32; L], product: V| {
        (V::load_array_ref(simd, lanes) + product).store_slice(lanes);
    };
    let f: [V; N] = core::array::from_fn(|k| V::from_slice(simd, &features[k][x..x + L]));
    let t = V::from_slice(simd, &target[x..x + L]);
    for i in 0..N {
        add(&mut cross[i], f[i] * t);
        for j in i..N {
            add(&mut pairs[i][j], f[i] * f[j]);
        }
    }
}

/// The `i64` sum of `lanes`, which it zeroes.
fn drain<const L: usize>(lanes: &mut [i32; L]) -> i64 {
    core::mem::replace(lanes, [0; L])
        .iter()
        .map(|&lane| i64::from(lane))
        .sum()
}

/// The SIMD accumulation over `N` coefficients: `i32` lane sums carried
/// across rows, moved to `i64` before a lane can overflow.
pub(crate) struct WienerAccumulator<const N: usize> {
    pairs: [[[i32; LANES]; N]; N],
    cross: [[i32; LANES]; N],
    chunks: usize,
    budget: usize,
}

impl<const N: usize> WienerAccumulator<N> {
    /// An accumulator whose feature and target products are at most
    /// `max_product` in magnitude.
    pub(crate) fn new(max_product: u64) -> Self {
        Self {
            pairs: [[[0; LANES]; N]; N],
            cross: [[0; LANES]; N],
            chunks: 0,
            budget: (i32::MAX as u64 / max_product.max(1)).max(1) as usize,
        }
    }

    /// Accumulates the rows `features[k]` and `target`, which have the same
    /// length, into the lanes and, for their tail, into `stats`.
    pub(crate) fn add(&mut self, features: &[&[i32]; N], target: &[i32], stats: &mut WienerStats) {
        let len = target.len();
        let mut x = 0;
        stats.samples += (len - len % LANES) as u64;
        dispatch!(Level::baseline(), simd => while x + LANES <= len {
            if self.chunks == self.budget {
                self.flush(stats);
            }
            let lanes = (&mut self.pairs, &mut self.cross);
            add_products::<_, i32x8<_>, LANES, N>(simd, lanes, features, target, x);
            x += LANES;
            self.chunks += 1;
        });
        accumulate_samples(features, target, x..len, stats);
    }

    /// Moves the lane sums into `stats`.
    pub(crate) fn flush(&mut self, stats: &mut WienerStats) {
        let mut pair = 0;
        for i in 0..N {
            stats.b[i] += drain(&mut self.cross[i]);
            for j in i..N {
                stats.a[pair] += drain(&mut self.pairs[i][j]);
                pair += 1;
            }
        }
        self.chunks = 0;
    }
}

/// Samples of a row that share a § 7.20.4 class: one 4x4 cell.
const CELL: usize = 4;

/// The SIMD accumulation of samples into per-class normal equations: each
/// run of four samples of one cell adds to its class's `i32` lanes, which
/// move to `i64` before they can overflow.
pub(crate) struct WienerClassAccumulator<const N: usize> {
    pairs: Vec<[[[i32; CELL]; N]; N]>,
    cross: Vec<[[i32; CELL]; N]>,
    chunks: Vec<usize>,
    touched: Vec<usize>,
    budget: usize,
}

impl<const N: usize> WienerClassAccumulator<N> {
    /// An accumulator of `classes` classes whose feature and target products
    /// are at most `max_product` in magnitude.
    pub(crate) fn new(classes: usize, max_product: u64) -> Self {
        Self {
            pairs: vec![[[[0; CELL]; N]; N]; classes],
            cross: vec![[[0; CELL]; N]; classes],
            chunks: vec![0; classes],
            touched: Vec::new(),
            budget: (i32::MAX as u64 / max_product.max(1)).max(1) as usize,
        }
    }

    /// Accumulates the rows `features[k]` and `target`, whose samples
    /// `4 * i .. 4 * i + 4` have class `cells[i]`, into the lanes and, for the
    /// tail, into `stats`.
    pub(crate) fn add(
        &mut self,
        features: &[&[i32]; N],
        target: &[i32],
        cells: &[u8],
        stats: &mut [WienerStats],
    ) {
        let len = target.len();
        dispatch!(Level::baseline(), simd => {
            for (i, &class) in cells.iter().enumerate().take(len / CELL) {
                let class = usize::from(class);
                if self.chunks[class] == self.budget {
                    self.flush_class(class, stats);
                }
                if self.chunks[class] == 0 {
                    self.touched.push(class);
                }
                let lanes = (&mut self.pairs[class], &mut self.cross[class]);
                add_products::<_, i32x4<_>, CELL, N>(simd, lanes, features, target, i * CELL);
                self.chunks[class] += 1;
                stats[class].samples += CELL as u64;
            }
        });
        let tail = len - len % CELL;
        if let Some(&class) = cells.get(tail / CELL).filter(|_| tail < len) {
            accumulate_samples(features, target, tail..len, &mut stats[usize::from(class)]);
        }
    }

    /// Moves the lane sums of every class into `stats` and returns the classes
    /// with samples since the last call, sorted.
    pub(crate) fn flush(&mut self, stats: &mut [WienerStats]) -> Vec<usize> {
        let mut touched = core::mem::take(&mut self.touched);
        for &class in &touched {
            self.flush_class(class, stats);
        }
        touched.sort_unstable();
        touched.dedup();
        touched
    }

    fn flush_class(&mut self, class: usize, stats: &mut [WienerStats]) {
        let (pairs, cross, stats) = (
            &mut self.pairs[class],
            &mut self.cross[class],
            &mut stats[class],
        );
        let mut pair = 0;
        for i in 0..N {
            stats.b[i] += drain(&mut cross[i]);
            for lanes in &mut pairs[i][i..] {
                stats.a[pair] += drain(lanes);
                pair += 1;
            }
        }
        self.chunks[class] = 0;
    }
}

/// `plus + minus - 2 * centre` per sample into `out`, the length of the
/// shortest input: the symmetric tap pair feature of the Wiener-NS fit.
pub(crate) fn pair_features(plus: &[u16], minus: &[u16], centre: &[u16], out: &mut Vec<i32>) {
    let n = plus.len().min(minus.len()).min(centre.len());
    out.clear();
    out.reserve(n);
    let (p, p_tail) = plus[..n].as_chunks::<16>();
    let (m, m_tail) = minus[..n].as_chunks::<16>();
    let (c, c_tail) = centre[..n].as_chunks::<16>();
    dispatch!(Level::baseline(), simd => for ((p, m), c) in p.iter().zip(m).zip(c) {
        let mut lanes = [0; 16];
        for (half, lanes) in lanes.chunks_exact_mut(LANES).enumerate() {
            let wide = |v: &[u16; 16]| load_u16_as_i32(simd, &v[half * LANES..]);
            (wide(p) + wide(m) - wide(c) * 2).store_slice(lanes);
        }
        out.extend_from_slice(&lanes);
    });
    out.extend(
        p_tail
            .iter()
            .zip(m_tail)
            .zip(c_tail)
            .map(|((&p, &m), &c)| i32::from(p) + i32::from(m) - 2 * i32::from(c)),
    );
}

#[cfg(test)]
fn pair_features_scalar(plus: &[u16], minus: &[u16], centre: &[u16], out: &mut Vec<i32>) {
    out.clear();
    out.extend(
        plus.iter()
            .zip(minus)
            .zip(centre)
            .map(|((&p, &m), &c)| i32::from(p) + i32::from(m) - 2 * i32::from(c)),
    );
}

/// `a - b` per sample into `out`, the length of the shorter input.
pub(crate) fn differences(a: &[u16], b: &[u16], out: &mut Vec<i32>) {
    let n = a.len().min(b.len());
    out.clear();
    out.reserve(n);
    let (a, a_tail) = a[..n].as_chunks::<16>();
    let (b, b_tail) = b[..n].as_chunks::<16>();
    dispatch!(Level::baseline(), simd => for (a, b) in a.iter().zip(b) {
        let mut lanes = [0; 16];
        for (half, lanes) in lanes.chunks_exact_mut(LANES).enumerate() {
            let wide = |v: &[u16; 16]| load_u16_as_i32(simd, &v[half * LANES..]);
            (wide(a) - wide(b)).store_slice(lanes);
        }
        out.extend_from_slice(&lanes);
    });
    out.extend(
        a_tail
            .iter()
            .zip(b_tail)
            .map(|(&a, &b)| i32::from(a) - i32::from(b)),
    );
}

#[cfg(test)]
fn differences_scalar(a: &[u16], b: &[u16], out: &mut Vec<i32>) {
    out.clear();
    out.extend(a.iter().zip(b).map(|(&a, &b)| i32::from(a) - i32::from(b)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// `(coefficients, bound, rows of (features, target))`.
    type Case = (usize, i32, Vec<(Vec<Vec<i32>>, Vec<i32>)>);

    proptest! {
        /// Asymmetric rows of different lengths, any sample value.
        #[test]
        fn feature_rows_match_scalar(
            a in proptest::collection::vec(any::<u16>(), 0..80),
            b in proptest::collection::vec(any::<u16>(), 0..80),
            c in proptest::collection::vec(any::<u16>(), 0..80),
        ) {
            let (mut simd, mut scalar) = (vec![5], vec![6]);
            pair_features(&a, &b, &c, &mut simd);
            pair_features_scalar(&a, &b, &c, &mut scalar);
            prop_assert_eq!(&simd, &scalar);
            differences(&a, &c, &mut simd);
            differences_scalar(&a, &c, &mut scalar);
            prop_assert_eq!(simd, scalar);
        }
    }

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

    fn run_n<const N: usize>(
        max: i32,
        rows: &[(Vec<Vec<i32>>, Vec<i32>)],
    ) -> (WienerStats, WienerStats) {
        let (mut scalar, mut simd) = (WienerStats::new(N), WienerStats::new(N));
        let mut lanes = WienerAccumulator::<N>::new(4 * (max as u64).pow(2));
        for (features, target) in rows {
            let features: [&[i32]; N] = core::array::from_fn(|k| features[k].as_slice());
            accumulate_scalar(&features, target, &mut scalar);
            lanes.add(&features, target, &mut simd);
        }
        lanes.flush(&mut simd);
        (scalar, simd)
    }

    fn run(n: usize, max: i32, rows: &[(Vec<Vec<i32>>, Vec<i32>)]) -> (WienerStats, WienerStats) {
        if n == 16 {
            run_n::<16>(max, rows)
        } else {
            run_n::<18>(max, rows)
        }
    }

    proptest! {
        #[test]
        fn class_simd_matches_scalar(
            (_, max, rows) in case(),
            seed in any::<u64>(),
        ) {
            let classes = 7usize;
            let mut state = seed | 1;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % classes as u64) as u8
            };
            let n = rows.first().map_or(16, |row| row.0.len());
            let (mut scalar, mut simd) = (vec![WienerStats::new(n); classes], vec![WienerStats::new(n); classes]);
            let mut touched = Vec::new();
            if n == 16 {
                let mut lanes = WienerClassAccumulator::<16>::new(classes, 4 * (max as u64).pow(2));
                for (features, target) in &rows {
                    let cells: Vec<u8> = (0..target.len().div_ceil(4)).map(|_| next()).collect();
                    let features: [&[i32]; 16] = core::array::from_fn(|k| features[k].as_slice());
                    for (x, &t) in target.iter().enumerate() {
                        let one: [&[i32]; 16] = core::array::from_fn(|k| &features[k][x..=x]);
                        accumulate_scalar(&one, &[t], &mut scalar[usize::from(cells[x / 4])]);
                    }
                    lanes.add(&features, target, &cells, &mut simd);
                }
                touched = lanes.flush(&mut simd);
            }
            prop_assert_eq!(&scalar, &simd);
            prop_assert!(touched.windows(2).all(|pair| pair[0] < pair[1]));
        }

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
