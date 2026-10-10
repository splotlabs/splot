// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sliding source windows for the AV2 § 7.13.3.18 horizontal filter pass.
//!
//! A full-span phase reads eight overlapping `LANES`-wide windows of one
//! reference row. Loading the whole span once and sliding it by lane leaves
//! every window's values untouched, so the convolution is unchanged; only the
//! load shape differs. `simd_swizzle!` over two vectors lowers to `ext.16b` on
//! AArch64, which is what turns eight overlapping unaligned loads into two
//! loads plus seven slides.

use super::{INTER_ROUND0, NUM_TAPS, reference_lanes, round2_simd, tap_mac};
use crate::format::ReconSample;
use std::simd::{Simd, num::SimdInt, num::SimdUint, simd_swizzle};

/// One accumulator width's full-span horizontal convolution over slid windows.
pub(super) trait SlideLanes: Sized {
    /// Samples the two loads read from the window origin.
    ///
    /// The windows only use `first..first + NUM_TAPS - 1 + LANES`; the rest of
    /// the second load is discarded by the slide but still has to be readable,
    /// so callers pass a source of at least `SPAN` samples.
    const SPAN: usize;

    /// The `i16` lanes of one horizontal-pass intermediate row.
    type Intermediate;

    /// Returns `Round2(sum, InterRound0)` of the eight taps at `first`, with
    /// `taps` from [`intermediate_taps`].
    fn slid_intermediate<T: ReconSample>(
        source: &[T],
        first: usize,
        taps: Simd<i16, NUM_TAPS>,
    ) -> Self::Intermediate;
}

/// Reads `LANES` consecutive samples as `i16`.
///
/// § 6 Table 6.3 admits only `BitDepth` 8 and 10, so every reference sample is
/// at most 1023 and the narrowing preserves the value, the same argument
/// [`tap_mac`] already relies on.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn lanes_at<const LANES: usize, T: ReconSample>(source: &[T], start: usize) -> Simd<i16, LANES> {
    reference_lanes::<LANES, T>(source, start).cast()
}

/// Packs one `Subpel_Filters` row for [`SlideLanes::slid_intermediate`],
/// halved for eight-bit storage.
///
/// Every tap is even, and the halved positive and negative taps of a row sum
/// to at most 92 and -28, so an eight-bit row sums to `-7140..=23460` in `i16`
/// lanes, and `Round2(2 * half, 3) == Round2(half, 2)`.
pub(super) fn intermediate_taps<T: ReconSample>(taps: &[i32; NUM_TAPS]) -> Simd<i16, NUM_TAPS> {
    let taps = Simd::from_array(*taps).cast::<i16>();
    if T::MAX_VALUE > u16::from(u8::MAX) {
        taps
    } else {
        taps >> 1
    }
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn intermediate<const LANES: usize, T: ReconSample>(
    windows: [Simd<i16, LANES>; NUM_TAPS],
    taps: Simd<i16, NUM_TAPS>,
) -> Simd<i16, LANES> {
    if T::MAX_VALUE > u16::from(u8::MAX) {
        let mut sum = Simd::splat(0);
        for tap in 0..NUM_TAPS {
            sum = tap_mac(sum, windows[tap], i32::from(taps[tap]));
        }
        return round2_simd(sum, INTER_ROUND0).cast();
    }
    let mut half = Simd::<i16, LANES>::splat(0);
    for tap in 0..NUM_TAPS {
        half += windows[tap] * Simd::splat(taps[tap]);
    }
    (half + Simd::splat(1 << (INTER_ROUND0 - 2))) >> (INTER_ROUND0 - 1) as i16
}

impl SlideLanes for Simd<i32, 4> {
    const SPAN: usize = 16;
    type Intermediate = Simd<i16, 4>;

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_intermediate<T: ReconSample>(
        source: &[T],
        first: usize,
        taps: Simd<i16, NUM_TAPS>,
    ) -> Self::Intermediate {
        let lo = lanes_at::<8, T>(source, first);
        let hi = lanes_at::<8, T>(source, first + 8);
        intermediate::<4, T>(
            [
                simd_swizzle!(lo, hi, [0, 1, 2, 3]),
                simd_swizzle!(lo, hi, [1, 2, 3, 4]),
                simd_swizzle!(lo, hi, [2, 3, 4, 5]),
                simd_swizzle!(lo, hi, [3, 4, 5, 6]),
                simd_swizzle!(lo, hi, [4, 5, 6, 7]),
                simd_swizzle!(lo, hi, [5, 6, 7, 8]),
                simd_swizzle!(lo, hi, [6, 7, 8, 9]),
                simd_swizzle!(lo, hi, [7, 8, 9, 10]),
            ],
            taps,
        )
    }
}

impl SlideLanes for Simd<i32, 8> {
    const SPAN: usize = 16;
    type Intermediate = Simd<i16, 8>;

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_intermediate<T: ReconSample>(
        source: &[T],
        first: usize,
        taps: Simd<i16, NUM_TAPS>,
    ) -> Self::Intermediate {
        let lo = lanes_at::<8, T>(source, first);
        let hi = lanes_at::<8, T>(source, first + 8);
        intermediate::<8, T>(
            [
                lo,
                simd_swizzle!(lo, hi, [1, 2, 3, 4, 5, 6, 7, 8]),
                simd_swizzle!(lo, hi, [2, 3, 4, 5, 6, 7, 8, 9]),
                simd_swizzle!(lo, hi, [3, 4, 5, 6, 7, 8, 9, 10]),
                simd_swizzle!(lo, hi, [4, 5, 6, 7, 8, 9, 10, 11]),
                simd_swizzle!(lo, hi, [5, 6, 7, 8, 9, 10, 11, 12]),
                simd_swizzle!(lo, hi, [6, 7, 8, 9, 10, 11, 12, 13]),
                simd_swizzle!(lo, hi, [7, 8, 9, 10, 11, 12, 13, 14]),
            ],
            taps,
        )
    }
}

impl SlideLanes for Simd<i32, 16> {
    const SPAN: usize = 32;
    type Intermediate = Simd<i16, 16>;

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_intermediate<T: ReconSample>(
        source: &[T],
        first: usize,
        taps: Simd<i16, NUM_TAPS>,
    ) -> Self::Intermediate {
        let lo = lanes_at::<16, T>(source, first);
        let hi = lanes_at::<16, T>(source, first + 16);
        intermediate::<16, T>(
            [
                lo,
                simd_swizzle!(
                    lo,
                    hi,
                    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21]
                ),
                simd_swizzle!(
                    lo,
                    hi,
                    [7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22]
                ),
            ],
            taps,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{INTER_ROUND0, NUM_TAPS, Simd, SlideLanes, intermediate_taps};
    use crate::format::ReconSample;
    use crate::subpel_mc::SUBPEL_FILTERS;

    fn expected<T: ReconSample>(
        source: &[T],
        first: usize,
        taps: &[i32; NUM_TAPS],
        lane: usize,
    ) -> i16 {
        let sum: i32 = (0..NUM_TAPS)
            .map(|tap| taps[tap] * i32::from(source[first + tap + lane].to_u16()))
            .sum();
        ((sum + (1 << (INTER_ROUND0 - 1))) >> INTER_ROUND0) as i16
    }

    fn check<T: ReconSample>(source: &[T], taps: &[i32; NUM_TAPS]) {
        let packed = intermediate_taps::<T>(taps);
        for first in 0..=source.len() - 32 {
            let four = <Simd<i32, 4> as SlideLanes>::slid_intermediate(source, first, packed);
            let eight = <Simd<i32, 8> as SlideLanes>::slid_intermediate(source, first, packed);
            let sixteen = <Simd<i32, 16> as SlideLanes>::slid_intermediate(source, first, packed);
            for lane in 0..16 {
                let expected = expected(source, first, taps, lane);
                if lane < 4 {
                    assert_eq!(four[lane], expected, "4 lanes at {first} {taps:?}");
                }
                if lane < 8 {
                    assert_eq!(eight[lane], expected, "8 lanes at {first} {taps:?}");
                }
                assert_eq!(sixteen[lane], expected, "16 lanes at {first} {taps:?}");
            }
        }
    }

    #[test]
    fn slid_intermediates_match_overlapping_loads_for_every_filter_row() {
        let ten_bit = (0..64u32)
            .map(|i| ((i * 37) % 1024) as u16)
            .collect::<Vec<_>>();
        let eight_bit = (0..64u32)
            .map(|i| ((i * 37) % 256) as u8)
            .collect::<Vec<_>>();
        for taps in SUBPEL_FILTERS.iter().flatten() {
            check(&ten_bit, taps);
            check(&eight_bit, taps);
            for sign in [1, -1] {
                let extreme = (0..40)
                    .map(|i| {
                        if sign * taps[i % NUM_TAPS] > 0 {
                            u8::MAX
                        } else {
                            0
                        }
                    })
                    .collect::<Vec<_>>();
                check(&extreme, taps);
            }
        }
    }
}
