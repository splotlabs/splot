// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Two rows per 16-lane vector for 8-bit narrow blocks.
//!
//! A [`Pair`] holds two rows, one per 128-bit half, so every slide,
//! interleave and pack stays inside a half. Two vectors whose halves are
//! consecutive rows interleave into tap pairs for one multiply-add each; the
//! low and high halves then yield two output rows with the same taps.

use super::{INTER_ROUND0, NUM_TAPS, multiply_add_pairs};
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
