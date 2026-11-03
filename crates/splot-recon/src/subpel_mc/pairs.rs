// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Two rows per 16-lane vector for 8-bit narrow blocks.
//!
//! A [`Pair`] holds two rows, one per 128-bit half, so every slide,
//! interleave and pack stays inside a half. Two vectors whose halves are
//! consecutive rows interleave into tap pairs for one multiply-add each; the
//! low and high halves then yield two output rows with the same taps.

use super::{
    ACTIVE_TAP_SPANS, INTER_ROUND0, NUM_FILTER_TYPES, NUM_PHASES, NUM_TAPS, SUBPEL_FILTERS, tap_mac,
};
use std::simd::{Simd, Swizzle, ToBytes, cmp::SimdOrd, num::SimdInt, num::SimdUint, simd_swizzle};

pub(super) type Pair = Simd<i16, 16>;
type Sums = Simd<i32, 8>;

/// Lane `i` of each half reads lane `i + K` of the half's concatenated
/// `(low, high)`: one in-lane `palignr`.
pub(super) struct Slide<const K: usize>;

impl<const K: usize> Swizzle<16> for Slide<K> {
    const INDEX: [usize; 16] = {
        let mut index = [0; 16];
        let mut lane = 0;
        while lane < 16 {
            let (half, i) = (lane / 8, lane % 8 + K);
            index[lane] = if i < 8 {
                half * 8 + i
            } else {
                16 + half * 8 + i - 8
            };
            lane += 1;
        }
        index
    };
}

/// In-lane interleave of the low (`HIGH == 0`) or high four lanes of each
/// half of two vectors: `punpcklwd` / `punpckhwd`.
pub(super) struct Interleave<const HIGH: usize>;

impl<const HIGH: usize> Swizzle<16> for Interleave<HIGH> {
    const INDEX: [usize; 16] = {
        let mut index = [0; 16];
        let mut lane = 0;
        while lane < 16 {
            let source = (lane / 8) * 8 + HIGH * 4 + (lane % 8) / 2;
            index[lane] = source + (lane % 2) * 16;
            lane += 1;
        }
        index
    };
}

/// `packssdw`: each half's four `low` lanes, then its four `high` lanes.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn pack(low: Sums, high: Sums) -> Pair {
    let lanes: Simd<i32, 16> = simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    );
    lanes
        .simd_max(Simd::splat(i32::from(i16::MIN)))
        .simd_min(Simd::splat(i32::from(i16::MAX)))
        .cast()
}

/// `Round2(Σ weights * (a, b), shift)` of interleaved lanes, packed back to
/// the lane order of `a`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn weigh<const SHIFT: u32>(pairs: &[(Pair, Pair, Pair)]) -> Pair {
    let bias = Sums::splat(1 << (SHIFT - 1));
    let (mut low, mut high) = (bias, bias);
    for &(a, b, weights) in pairs {
        low += multiply_add_pairs(Interleave::<0>::concat_swizzle(a, b), weights);
        high += multiply_add_pairs(Interleave::<1>::concat_swizzle(a, b), weights);
    }
    pack(low >> SHIFT as i32, high >> SHIFT as i32)
}

/// The two 16-byte source windows as one vector, one row per half.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn join(first: Simd<u8, 16>, second: Simd<u8, 16>) -> Simd<u8, 32> {
    simd_swizzle!(
        first,
        second,
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31
        ]
    )
}

/// Each half's samples `0..8` and `8..16` as `i16` lanes.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn widen(bytes: Simd<u8, 32>) -> (Pair, Pair) {
    let zero = Simd::splat(0);
    let (bytes, zero) = if cfg!(target_endian = "big") {
        (zero, bytes)
    } else {
        (bytes, zero)
    };
    let low: Simd<u8, 32> = simd_swizzle!(
        bytes,
        zero,
        [
            0, 32, 1, 33, 2, 34, 3, 35, 4, 36, 5, 37, 6, 38, 7, 39, 16, 48, 17, 49, 18, 50, 19, 51,
            20, 52, 21, 53, 22, 54, 23, 55
        ]
    );
    let high: Simd<u8, 32> = simd_swizzle!(
        bytes,
        zero,
        [
            8, 40, 9, 41, 10, 42, 11, 43, 12, 44, 13, 45, 14, 46, 15, 47, 24, 56, 25, 57, 26, 58,
            27, 59, 28, 60, 29, 61, 30, 62, 31, 63
        ]
    );
    let lanes = |bytes| Simd::<u16, 16>::from_ne_bytes(bytes).cast::<i16>();
    (lanes(low), lanes(high))
}

/// The halved-tap horizontal pass of a pair of rows, as in `small_compound::eight_bit_row`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn horizontal(bytes: Simd<u8, 32>, taps: Simd<i16, NUM_TAPS>) -> Pair {
    let (low, high) = widen(bytes);
    macro_rules! tap {
        ($k:literal) => {
            Slide::<$k>::concat_swizzle(low, high) * Simd::splat(taps[$k])
        };
    }
    let half = Pair::splat(1 << (INTER_ROUND0 - 2))
        + low * Simd::splat(taps[0])
        + tap!(1)
        + tap!(2)
        + tap!(3)
        + tap!(4)
        + tap!(5)
        + tap!(6)
        + tap!(7);
    half >> (INTER_ROUND0 - 1) as i16
}

/// The first tap and the even tap count, 2, 4, 6 or 8, of a filter row's
/// active span, for kernels with a constant vertical tap count.
pub(super) const fn even_tap_span(filter: usize, phase: usize) -> (usize, usize) {
    let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
    let count = match end - start {
        0..=2 => 2,
        3..=4 => 4,
        5..=6 => 6,
        _ => NUM_TAPS,
    };
    let start = if start < NUM_TAPS - count {
        start
    } else {
        NUM_TAPS - count
    };
    (start, count)
}

/// Taps `start + 2j, start + 2j + 1` of every [`even_tap_span`] repeated
/// across 8 lanes, zero past the span. Loading them from a table keeps the
/// weights opaque, which keeps [`multiply_add_pairs`] one `pmaddwd`.
const PAIRED_TAPS: [[[[i16; 8]; NUM_TAPS / 2]; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut pairs = [[[[0; 8]; NUM_TAPS / 2]; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 0;
        while phase < NUM_PHASES {
            let (start, count) = even_tap_span(filter, phase);
            let mut lane = 0;
            while lane < count * 4 {
                let tap = start + 2 * (lane / 8) + lane % 2;
                pairs[filter][phase][lane / 8][lane % 8] =
                    SUBPEL_FILTERS[filter][phase][tap] as i16;
                lane += 1;
            }
            phase += 1;
        }
        filter += 1;
    }
    pairs
};

/// The [`PAIRED_TAPS`] of a filter row as 16-lane weights.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn paired_taps(filter: usize, phase: usize) -> [Simd<i16, 16>; NUM_TAPS / 2] {
    PAIRED_TAPS[filter][phase].map(|pair| {
        let pair = Simd::from_array(pair);
        simd_swizzle!(pair, [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7])
    })
}

/// Adjacent `i16` products summed in `i32`, which lowers to `pmaddwd`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn multiply_add_pairs(values: Simd<i16, 16>, weights: Simd<i16, 16>) -> Simd<i32, 8> {
    let product = values.cast::<i32>() * weights.cast::<i32>();
    simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15])
}

/// `Σ taps[t] * rows(t)` over `t < TAPS` for `pairs` from [`paired_taps`],
/// each pair of rows one interleave and one multiply-add. Integer sums are
/// exact, so the pairing leaves the result of the per-tap order unchanged.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn paired_tap_sum<const LANES: usize, const TAPS: usize>(
    pairs: &[Simd<i16, 16>; NUM_TAPS / 2],
    rows: impl Fn(usize) -> Simd<i16, LANES>,
) -> Simd<i32, LANES> {
    let row16 = |t| Simd::<i16, 16>::from_slice(rows(t).as_array());
    let row8 = |t| Simd::<i16, 8>::from_slice(rows(t).as_array());
    match LANES {
        16 => {
            let (mut low, mut high) = (Simd::splat(0), Simd::splat(0));
            for (pair, &weights) in pairs.iter().enumerate().take(TAPS / 2) {
                let (a, b) = (row16(2 * pair), row16(2 * pair + 1));
                let lo = simd_swizzle!(
                    a,
                    b,
                    [0, 16, 1, 17, 2, 18, 3, 19, 8, 24, 9, 25, 10, 26, 11, 27]
                );
                let hi = simd_swizzle!(
                    a,
                    b,
                    [4, 20, 5, 21, 6, 22, 7, 23, 12, 28, 13, 29, 14, 30, 15, 31]
                );
                low += multiply_add_pairs(lo, weights);
                high += multiply_add_pairs(hi, weights);
            }
            let sum: Simd<i32, 16> = simd_swizzle!(
                low,
                high,
                [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
            );
            Simd::from_slice(sum.as_array())
        }
        8 => {
            let mut sum = Simd::<i32, 8>::splat(0);
            for (pair, &weights) in pairs.iter().enumerate().take(TAPS / 2) {
                let (a, b) = (row8(2 * pair), row8(2 * pair + 1));
                let x = simd_swizzle!(a, b, [0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15]);
                sum += multiply_add_pairs(x, weights);
            }
            Simd::from_slice(sum.as_array())
        }
        4 => {
            let row4 = |t| Simd::<i16, 4>::from_slice(rows(t).as_array());
            let mut sum = Simd::<i32, 4>::splat(0);
            for (pair, &weights) in pairs.iter().enumerate().take(TAPS / 2) {
                let x = simd_swizzle!(row4(2 * pair), row4(2 * pair + 1), [0, 4, 1, 5, 2, 6, 3, 7]);
                let weights: Simd<i16, 8> = simd_swizzle!(weights, [0, 1, 2, 3, 4, 5, 6, 7]);
                let product = x.cast::<i32>() * weights.cast::<i32>();
                sum += simd_swizzle!(product, [0, 2, 4, 6]) + simd_swizzle!(product, [1, 3, 5, 7]);
            }
            Simd::from_slice(sum.as_array())
        }
        _ => {
            let mut sum = Simd::<i32, LANES>::splat(0);
            for t in 0..TAPS {
                sum = tap_mac(sum, rows(t), i32::from(pairs[t / 2][t % 2]));
            }
            sum
        }
    }
}
