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

use fearless_simd::{Simd, i16x16, i32x8, prelude::*, u16x16};

/// Runs `$body` with the baseline SIMD token bound to `$simd`; on x86 the
/// AVX2 token binds directly, since a `dispatch!` closure does not inline
/// into per-row callers.
macro_rules! lr_simd {
    ($simd:ident => $body:expr) => {{
        #[cfg(target_arch = "x86_64")]
        let value = match fearless_simd::Level::baseline().as_avx2() {
            Some($simd) => $body,
            None => fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body),
        };
        #[cfg(not(target_arch = "x86_64"))]
        let value = fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body);
        value
    }};
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
    let concat = |low: &MaddPattern, high: &MaddPattern| -> [i16; 16] {
        let mut half = [0; 16];
        half[..8].copy_from_slice(low); // splot-copy-ok: one cell's weights into its vector half
        half[8..].copy_from_slice(high); // splot-copy-ok: one cell's weights into its vector half
        half
    };
    *weights = [concat(a, b), concat(c, d)];
}

/// The weights of one pair shared by all four cells.
#[inline]
pub(crate) fn uniform(pattern: &MaddPattern) -> MaddWeights {
    let mut weights = [[0; 16]; 2];
    for half in &mut weights {
        for (lane, weight) in half.iter_mut().enumerate() {
            *weight = pattern[lane % 8];
        }
    }
    weights
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
    let mut sums = [i32x8::splat(simd, 0); 2];
    let mut add = |(low, high): (i16x16<S>, i16x16<S>), weights: &MaddWeights| {
        sums[0] += low.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[0]));
        sums[1] += high.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[1]));
    };
    let pairs = F::TAPS.len() / 2;
    for (pair, weights) in weights.iter().enumerate().take(pairs) {
        add(term(2 * pair).interleave(term(2 * pair + 1)), weights);
    }
    if let Some(weights) = weights.get(pairs) {
        add(load(r, r).interleave(i16x16::splat(simd, 1)), weights);
    }
    let max = i32x8::splat(simd, i32::from(max_sample));
    let [low, high] = sums.map(|sum| (sum >> 7).clamp(i32x8::splat(simd, 0), max));
    let values: u16x16<S> = low.narrow(high).bitcast();
    values.into()
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
