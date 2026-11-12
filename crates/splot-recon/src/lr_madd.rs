// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! 16-lane § 7.20.3 and § 7.20.4 row kernels on `pmaddwd` tap pairs.
//!
//! Both filters sum symmetric taps first, so every tap sum, the center and
//! each weight fit `i16`, and a pair of tap sums times a pair of weights is
//! one `pmaddwd`. The last pair weighs the center and a lane of ones by the
//! center weight and the rounding offset. Four 4-sample cells share a
//! vector; each cell's weights are a [`MaddPattern`] of four lanes.
//!
//! On x86 the pairs interleave in-lane, so the weights of cells 0 and 2 and
//! of cells 1 and 3 share a vector, and the final pack restores the order;
//! the portable kernel interleaves in sample order.

#[cfg(not(target_arch = "x86_64"))]
use fearless_simd::i32x4;
use fearless_simd::{Simd, SimdIntWiden, i16x16, i32x8, prelude::*, u16x16};

/// Runs `$body` with the baseline SIMD token bound to `$simd`; on x86 the
/// AVX2 token binds directly, since a `dispatch!` closure does not inline
/// into per-row callers.
macro_rules! lr_simd {
    ($simd:ident => $body:expr) => {
        baseline_simd!($simd => $body)
    };
}
pub(crate) use lr_simd;

use crate::wienerns_filter::LumaSimdSource;

/// One 4-sample cell's weights for one pair: `[w0, w1]` per lane.
pub(crate) type MaddPattern = [i16; 8];

/// Both `pmaddwd` weight vectors of one pair over four cells.
pub(crate) type MaddWeights = [[i16; 16]; 2];

/// Whether [`MaddWeights`] over `T` rows take the in-lane arrangement of the
/// x86 kernel.
#[inline]
fn in_lane<T: LumaSimdSource>() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        T::WIDE && fearless_simd::Level::baseline().as_avx2().is_some()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Writes the weights of one pair for cells `0..4` in the kernel's
/// arrangement.
#[inline]
pub(crate) fn arrange_into<T: LumaSimdSource>(weights: &mut MaddWeights, cells: [&MaddPattern; 4]) {
    let [a, b, c, d] = if in_lane::<T>() {
        [cells[0], cells[2], cells[1], cells[3]]
    } else {
        cells
    };
    #[cfg(not(target_arch = "x86_64"))]
    {
        weights[0] = [
            a[0], a[0], a[0], a[0], b[0], b[0], b[0], b[0], c[0], c[0], c[0], c[0], d[0], d[0],
            d[0], d[0],
        ];
        weights[1] = [
            a[1], a[1], a[1], a[1], b[1], b[1], b[1], b[1], c[1], c[1], c[1], c[1], d[1], d[1],
            d[1], d[1],
        ];
    }
    #[cfg(target_arch = "x86_64")]
    for (half, [low, high]) in weights.iter_mut().zip([[a, b], [c, d]]) {
        let (first, second) = half.split_at_mut(8);
        first.copy_from_slice(low); // splot-copy-ok: one cell's weights into its vector half
        second.copy_from_slice(high); // splot-copy-ok: one cell's weights into its vector half
    }
}

/// The weights of one pair shared by all four cells.
#[inline]
pub(crate) fn uniform(pattern: &MaddPattern) -> MaddWeights {
    #[cfg(not(target_arch = "x86_64"))]
    {
        [[pattern[0]; 16], [pattern[1]; 16]]
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut weights = [[0; 16]; 2];
        for half in &mut weights {
            for (lane, weight) in half.iter_mut().enumerate() {
                *weight = pattern[lane % 8];
            }
        }
        weights
    }
}

/// The symmetric taps `(dy, dx)` paired with `(-dy, -dx)` of one filter.
pub(crate) trait TapPairs {
    /// Tap reach in rows and columns.
    const RADIUS: usize;
    /// Pair `j` of the symmetric taps.
    const TAPS: &'static [(isize, isize)];
    /// The x86 kernel over `u16` rows.
    #[cfg(target_arch = "x86_64")]
    fn avx2<const ROWS: usize, const REACH: usize>(
        t: fearless_simd::Avx2,
        window: &[&[u16; REACH]; ROWS],
        weights: &[MaddWeights],
        max_sample: u16,
    ) -> Option<[u16; 16]>;
}

/// The 16 filtered samples of one row from padded `window` rows, each slice
/// starting `RADIUS` columns left of the first sample.
#[allow(
    clippy::inline_always,
    reason = "measured: the row kernel must inline into its group loop"
)]
#[inline(always)]
pub(crate) fn filter16<F: TapPairs, T: LumaSimdSource, const ROWS: usize, const REACH: usize>(
    window: &[&[T; REACH]; ROWS],
    weights: &[MaddWeights],
    max_sample: u16,
) -> [u16; 16] {
    #[cfg(target_arch = "x86_64")]
    if let (Some(token), Some(rows)) = (
        fearless_simd::Level::baseline().as_avx2(),
        T::u16_rows(window),
    ) && let Some(values) = F::avx2(token, &rows, weights, max_sample)
    {
        return values;
    }
    lr_simd!(simd => portable::<_, F, T, ROWS, REACH>(simd, window, weights, max_sample))
}

#[allow(
    clippy::inline_always,
    reason = "measured: the row kernel must inline into its group loop"
)]
#[inline(always)]
fn portable<S: Simd, F: TapPairs, T: LumaSimdSource, const ROWS: usize, const REACH: usize>(
    simd: S,
    window: &[&[T; REACH]; ROWS],
    weights: &[MaddWeights],
    max_sample: u16,
) -> [u16; 16] {
    let r = F::RADIUS;
    let load = |row: usize, col: usize| -> i16x16<S> {
        window
            .get(row)
            .and_then(|row| row.get(col..))
            .and_then(<[T]>::first_chunk::<16>)
            .map_or_else(
                || i16x16::splat(simd, 0),
                |lanes| T::widen16(simd, lanes).bitcast(),
            )
    };
    let term = |j: usize| {
        let (dy, dx) = F::TAPS[j];
        load(r.wrapping_add_signed(dy), r.wrapping_add_signed(dx))
            + load(r.wrapping_add_signed(-dy), r.wrapping_add_signed(-dx))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let mut sums = [i32x4::splat(simd, 0); 4];
    #[cfg(not(target_arch = "x86_64"))]
    let mut add_term = |j: usize, w: &[i16; 16]| {
        let (a_lo, a_hi) = simd.split_i16x16(term(j));
        let w_lo = fearless_simd::i16x8::from_slice(simd, &w[..8]);
        let w_hi = fearless_simd::i16x8::from_slice(simd, &w[8..]);
        let (p0, p1) = a_lo.widening_mul(w_lo);
        let (p2, p3) = a_hi.widening_mul(w_hi);
        sums[0] += p0;
        sums[1] += p1;
        sums[2] += p2;
        sums[3] += p3;
    };
    #[cfg(target_arch = "x86_64")]
    let mut sums = [i32x8::splat(simd, 0); 2];
    #[cfg(target_arch = "x86_64")]
    let mut add = |(low, high): (i16x16<S>, i16x16<S>), weights: &MaddWeights| {
        sums[0] += low.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[0]));
        sums[1] += high.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[1]));
    };
    let pairs = F::TAPS.len() / 2;
    macro_rules! pair_step {
        ($pair:literal) => {
            if $pair < pairs
                && let Some(weights) = weights.get($pair)
            {
                #[cfg(not(target_arch = "x86_64"))]
                {
                    add_term(2 * $pair, &weights[0]);
                    add_term(2 * $pair + 1, &weights[1]);
                }
                #[cfg(target_arch = "x86_64")]
                add(term(2 * $pair).interleave(term(2 * $pair + 1)), weights);
            }
        };
    }
    pair_step!(0);
    pair_step!(1);
    pair_step!(2);
    pair_step!(3);
    pair_step!(4);
    pair_step!(5);
    pair_step!(6);
    pair_step!(7);
    if let Some(weights) = weights.get(pairs) {
        #[cfg(not(target_arch = "x86_64"))]
        {
            let (a_lo, a_hi) = simd.split_i16x16(load(r, r));
            let w_lo = fearless_simd::i16x8::from_slice(simd, &weights[0][..8]);
            let w_hi = fearless_simd::i16x8::from_slice(simd, &weights[0][8..]);
            let (p0, p1) = a_lo.widening_mul(w_lo);
            let (p2, p3) = a_hi.widening_mul(w_hi);
            let round = i32x4::splat(simd, i32::from(weights[1][0]));
            sums[0] += p0 + round;
            sums[1] += p1 + round;
            sums[2] += p2 + round;
            sums[3] += p3 + round;
        }
        #[cfg(target_arch = "x86_64")]
        add(load(r, r).interleave(i16x16::splat(simd, 1)), weights);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let sums = [
        simd.combine_i32x4(sums[0], sums[1]),
        simd.combine_i32x4(sums[2], sums[3]),
    ];
    let [low, high] =
        sums.map(|sum| (sum >> 7).clamp(i32x8::splat(simd, 0), i32x8::splat(simd, 65535)));
    let values: u16x16<S> = low.narrow(high).bitcast();
    values.min(u16x16::splat(simd, max_sample)).into()
}

/// Implements [`TapPairs::avx2`] for a tap table with one x86 kernel over
/// `ROWS` rows of `REACH` columns.
#[cfg(target_arch = "x86_64")]
macro_rules! madd_avx2 {
    ($taps:ty, $rows:literal, $reach:literal) => {
        #[allow(
            clippy::inline_always,
            reason = "measured: the row kernel must inline into its group loop"
        )]
        #[inline(always)]
        fn avx2<const ROWS: usize, const REACH: usize>(
            t: fearless_simd::Avx2,
            window: &[&[u16; REACH]; ROWS],
            weights: &[$crate::lr_madd::MaddWeights],
            max_sample: u16,
        ) -> Option<[u16; 16]> {
            use core::arch::x86_64::*;
            use fearless_simd::{Avx2, SimdBase, SimdInto, i16x16, u16x16};
            use $crate::lr_madd::MaddWeights;

            fearless_simd::kernel!(
                #[inline(always)]
                fn kernel(
                    t: Avx2,
                    window: &[&[u16; $reach]; $rows],
                    weights: &[MaddWeights],
                    max_sample: u16,
                ) -> [u16; 16] {
                    let r = <$taps as $crate::lr_madd::TapPairs>::RADIUS;
                    let taps = <$taps as $crate::lr_madd::TapPairs>::TAPS;
                    let load = |row: usize, col: usize| -> __m256i {
                        match window
                            .get(row)
                            .and_then(|row| row.get(col..))
                            .and_then(<[u16]>::first_chunk::<16>)
                        {
                            Some(lanes) => u16x16::load_array_ref(t, lanes).into(),
                            None => _mm256_setzero_si256(),
                        }
                    };
                    let term = |j: usize| {
                        let (dy, dx) = taps[j];
                        _mm256_add_epi16(
                            load(r.wrapping_add_signed(dy), r.wrapping_add_signed(dx)),
                            load(r.wrapping_add_signed(-dy), r.wrapping_add_signed(-dx)),
                        )
                    };
                    let half = |weights: &MaddWeights, half: usize| -> __m256i {
                        i16x16::load_array_ref(t, &weights[half]).into()
                    };
                    let mut sums = [_mm256_setzero_si256(); 2];
                    let mut add = |a: __m256i, b: __m256i, weights: &MaddWeights| {
                        sums[0] = _mm256_add_epi32(
                            sums[0],
                            _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), half(weights, 0)),
                        );
                        sums[1] = _mm256_add_epi32(
                            sums[1],
                            _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), half(weights, 1)),
                        );
                    };
                    let pairs = taps.len() / 2;
                    for (pair, weights) in weights.iter().enumerate().take(pairs) {
                        add(term(2 * pair), term(2 * pair + 1), weights);
                    }
                    if let Some(weights) = weights.get(pairs) {
                        add(load(r, r), _mm256_set1_epi16(1), weights);
                    }
                    let values = _mm256_min_epu16(
                        _mm256_packus_epi32(
                            _mm256_srai_epi32::<7>(sums[0]),
                            _mm256_srai_epi32::<7>(sums[1]),
                        ),
                        _mm256_set1_epi16(max_sample as i16),
                    );
                    let values: u16x16<Avx2> = values.simd_into(t);
                    values.into()
                }
            );

            if ROWS != $rows {
                return None;
            }
            let mut rows: [&[u16; $reach]; $rows] = [&[0; $reach]; $rows];
            for (slot, row) in rows.iter_mut().zip(window) {
                *slot = row.as_slice().first_chunk::<$reach>()?;
            }
            Some(kernel(t, &rows, weights, max_sample))
        }
    };
}

#[cfg(target_arch = "x86_64")]
pub(crate) use madd_avx2;

/// The AArch64 kernel for runs of 16-column groups that share one class,
/// and for single groups of four 4-column cells of different classes.
///
/// The weights stay in registers for the whole run, each tap pair is its
/// symmetric sum times a weight lane (one `smlal` per 4-output cell), and a
/// run whose last pairs weigh zero takes a variant that stops before them:
/// the far taps that end both tap tables are the ones classes zero most.
/// Tap sums of samples validated against a `BitDepth` of at most 10 (§ 6
/// Table 6.3) stay below `2^11`, so they are `i16` lanes; `sqrshrun` by 7 is
/// `Round2` and the clip to `0`, `umin` the clip to the maximum sample.
#[cfg(target_arch = "aarch64")]
pub(crate) mod by_lane {
    use core::arch::aarch64::*;
    use fearless_simd::{Neon, SimdBase, SimdFrom, i16x8, u16x8};

    use super::TapPairs;

    /// One class: the weight of each pair of `F`, then the center weight.
    pub(crate) type Class = ([i16; 16], i16);

    /// Filters the 16-column groups from column `x` to `end` of `rows`
    /// output rows, row `r` stored at `r * stride + x` of `output`, cell `k`
    /// of every group with `classes[k]`, or every cell with `classes[0]`
    /// when `uniform`. Window row `k` of output row `r` is `window[r + k]`,
    /// read from the group's column on; returns `false` when a row or the
    /// output is short.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[target_feature(enable = "neon")]
    pub(crate) fn filter_run<F: TapPairs, const ROWS: usize, const REACH: usize>(
        t: Neon,
        window: &[&[u16]],
        rows: usize,
        span: (usize, usize),
        (classes, uniform): (&[Class; 4], bool),
        output: (&mut [u16], usize),
        max_sample: u16,
    ) -> bool {
        if !uniform {
            return run::<F, true, 16, ROWS, REACH>(
                t, window, rows, span, classes, output, max_sample,
            );
        }
        let pairs = F::TAPS.len().min(16);
        let used = classes[0].0[..pairs]
            .iter()
            .rposition(|&weight| weight != 0)
            .map_or(0, |last| last + 1);
        macro_rules! variant {
            ($used:literal) => {
                run::<F, false, $used, ROWS, REACH>(
                    t, window, rows, span, classes, output, max_sample,
                )
            };
        }
        match used {
            0..=6 => variant!(6),
            7..=10 => variant!(10),
            11 | 12 => variant!(12),
            13 | 14 => variant!(14),
            _ => variant!(16),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[target_feature(enable = "neon")]
    fn run<
        F: TapPairs,
        const MIXED: bool,
        const USED: usize,
        const ROWS: usize,
        const REACH: usize,
    >(
        t: Neon,
        window: &[&[u16]],
        rows: usize,
        (x, end): (usize, usize),
        classes: &[Class; 4],
        (output, stride): (&mut [u16], usize),
        max_sample: u16,
    ) -> bool {
        let Some(first) = window.first() else {
            return false;
        };
        let class = |k: usize| &classes[if MIXED { k } else { 0 }];
        let half = |k: usize, at: usize| -> int16x8_t {
            match class(k).0.get(at..).and_then(<[i16]>::first_chunk::<8>) {
                Some(lanes) => i16x8::load_array_ref(t, lanes).into(),
                None => vdupq_n_s16(0),
            }
        };
        let low: [int16x8_t; 4] = core::array::from_fn(|k| half(k, 0));
        let high: [int16x8_t; 4] = core::array::from_fn(|k| half(k, 8));
        let center: int16x8_t =
            i16x8::load_array(t, core::array::from_fn(|k| class(k % 4).1)).into();
        let max = vdupq_n_u16(max_sample);
        let tap = |offset: isize| F::RADIUS.wrapping_add_signed(offset);
        let reach = (end - x + REACH).saturating_sub(16);
        for r in 0..rows {
            let (Some(window_rows), Some(target)) = (
                window.get(r..).and_then(<[_]>::first_chunk::<ROWS>),
                output.get_mut(r * stride + x..r * stride + end),
            ) else {
                return false;
            };
            let mut source = [*first; ROWS];
            for (slot, row) in source.iter_mut().zip(window_rows) {
                match row.get(x..x + reach) {
                    Some(row) => *slot = row,
                    None => return false,
                }
            }
            for target in target.chunks_exact_mut(16) {
                let Some(row) = source.first().and_then(|row| row.first_chunk::<REACH>()) else {
                    return false;
                };
                let mut group = [row; ROWS];
                for (slot, row) in group.iter_mut().zip(&mut source) {
                    match (row.first_chunk::<REACH>(), row.get(16..)) {
                        (Some(window), Some(rest)) => (*slot, *row) = (window, rest),
                        _ => return false,
                    }
                }
                let (low_out, high_out) = target.split_at_mut(8);
                let (Some(low_out), Some(high_out)) = (
                    low_out.first_chunk_mut::<8>(),
                    high_out.first_chunk_mut::<8>(),
                ) else {
                    return false;
                };
                let load = |row: usize, col: usize| -> int16x8_t {
                    match group
                        .get(row)
                        .and_then(|row| row.get(col..))
                        .and_then(<[u16]>::first_chunk::<8>)
                    {
                        Some(lanes) => {
                            vreinterpretq_s16_u16(u16x8::load_array_ref(t, lanes).into())
                        }
                        None => vdupq_n_s16(0),
                    }
                };
                let (m0, m1) = (load(tap(0), tap(0)), load(tap(0), tap(8)));
                let mut acc = [
                    vmull_laneq_s16::<0>(vget_low_s16(m0), center),
                    vmull_high_laneq_s16::<1>(m0, center),
                    vmull_laneq_s16::<2>(vget_low_s16(m1), center),
                    vmull_high_laneq_s16::<3>(m1, center),
                ];
                macro_rules! pairs {
                    ($($j:literal)*) => {$(
                        if $j < F::TAPS.len() && $j < USED {
                            const L: i32 = $j % 8;
                            let (dy, dx) = F::TAPS[$j];
                            let (p, q) = ((tap(dy), tap(dx)), (tap(-dy), tap(-dx)));
                            let s0 = vaddq_s16(load(p.0, p.1), load(q.0, q.1));
                            let s1 = vaddq_s16(load(p.0, p.1 + 8), load(q.0, q.1 + 8));
                            let w = if $j < 8 { &low } else { &high };
                            acc[0] = vmlal_laneq_s16::<L>(acc[0], vget_low_s16(s0), w[0]);
                            acc[1] = vmlal_high_laneq_s16::<L>(acc[1], s0, w[1]);
                            acc[2] = vmlal_laneq_s16::<L>(acc[2], vget_low_s16(s1), w[2]);
                            acc[3] = vmlal_high_laneq_s16::<L>(acc[3], s1, w[3]);
                        }
                    )*};
                }
                pairs!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
                let narrow =
                    |a, b| vminq_u16(vqrshrun_high_n_s32::<7>(vqrshrun_n_s32::<7>(a), b), max);
                u16x8::<Neon>::simd_from(t, narrow(acc[0], acc[1])).store_array(low_out);
                u16x8::<Neon>::simd_from(t, narrow(acc[2], acc[3])).store_array(high_out);
            }
        }
        true
    }
}
