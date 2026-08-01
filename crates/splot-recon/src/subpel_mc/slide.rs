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

use super::{NUM_TAPS, tap_mac};
use std::simd::{Simd, num::SimdUint, simd_swizzle};

/// One accumulator width's full-span horizontal convolution over slid windows.
pub(super) trait SlideLanes: Sized {
    /// Samples the two loads read from the window origin.
    ///
    /// The windows only use `first..first + NUM_TAPS - 1 + LANES`; the rest of
    /// the second load is discarded by the slide but still has to be readable,
    /// which is what [`SlideLanes::admits`] checks.
    const SPAN: usize;

    /// Reports whether `available` samples from the window origin admit the
    /// sliding load shape for the column starting at `column`.
    fn admits(available: usize, column: usize) -> bool {
        available >= column + Self::SPAN
    }

    /// Accumulates the eight full-span taps at `first` from two loads.
    fn slid_tap_sum(source: &[u16], first: usize, taps: &[i32; NUM_TAPS]) -> Self;
}

/// Reads `LANES` consecutive samples as `i16`.
///
/// § 6 Table 6.3 admits only `BitDepth` 8 and 10, so every reference sample is
/// at most 1023 and the narrowing preserves the value, the same argument
/// [`tap_mac`] already relies on.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn lanes_at<const LANES: usize>(source: &[u16], start: usize) -> Simd<i16, LANES> {
    Simd::<u16, LANES>::from_slice(&source[start..]).cast()
}

/// Accumulates prebuilt windows with a constant tap index so they stay in
/// registers.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn accumulate<const LANES: usize>(
    windows: [Simd<i16, LANES>; NUM_TAPS],
    taps: &[i32; NUM_TAPS],
) -> Simd<i32, LANES> {
    let mut sum = Simd::splat(0);
    for tap in 0..NUM_TAPS {
        sum = tap_mac(sum, windows[tap], taps[tap]);
    }
    sum
}

impl SlideLanes for Simd<i32, 4> {
    const SPAN: usize = 16;

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_tap_sum(source: &[u16], first: usize, taps: &[i32; NUM_TAPS]) -> Self {
        let lo = lanes_at::<8>(source, first);
        let hi = lanes_at::<8>(source, first + 8);
        accumulate(
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

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_tap_sum(source: &[u16], first: usize, taps: &[i32; NUM_TAPS]) -> Self {
        let lo = lanes_at::<8>(source, first);
        let hi = lanes_at::<8>(source, first + 8);
        accumulate(
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

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn slid_tap_sum(source: &[u16], first: usize, taps: &[i32; NUM_TAPS]) -> Self {
        let lo = lanes_at::<16>(source, first);
        let hi = lanes_at::<16>(source, first + 16);
        accumulate(
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
    use super::{NUM_TAPS, Simd, SlideLanes};

    fn source() -> Vec<u16> {
        (0..64u32).map(|i| ((i * 37) % 1024) as u16).collect()
    }

    fn overlapping_sum(source: &[u16], first: usize, taps: &[i32; NUM_TAPS], lane: usize) -> i32 {
        (0..NUM_TAPS)
            .map(|tap| taps[tap] * i32::from(source[first + tap + lane]))
            .sum()
    }

    #[test]
    fn slid_sums_match_overlapping_loads_at_every_offset() {
        let source = source();
        let taps: [i32; NUM_TAPS] = [-2, 6, -12, 84, 68, -14, 6, -2];
        for first in 0..24 {
            let four = <Simd<i32, 4> as SlideLanes>::slid_tap_sum(&source, first, &taps);
            let eight = <Simd<i32, 8> as SlideLanes>::slid_tap_sum(&source, first, &taps);
            let sixteen = <Simd<i32, 16> as SlideLanes>::slid_tap_sum(&source, first, &taps);
            for lane in 0..16 {
                let expected = overlapping_sum(&source, first, &taps, lane);
                if lane < 4 {
                    assert_eq!(four[lane], expected, "4 lanes at {first}");
                }
                if lane < 8 {
                    assert_eq!(eight[lane], expected, "8 lanes at {first}");
                }
                assert_eq!(sixteen[lane], expected, "16 lanes at {first}");
            }
        }
    }

    #[test]
    fn admits_reserves_the_whole_second_load() {
        assert!(!<Simd<i32, 8> as SlideLanes>::admits(15, 0));
        assert!(<Simd<i32, 8> as SlideLanes>::admits(16, 0));
        assert!(!<Simd<i32, 8> as SlideLanes>::admits(16, 1));
        assert!(!<Simd<i32, 4> as SlideLanes>::admits(15, 0));
        assert!(<Simd<i32, 4> as SlideLanes>::admits(16, 0));
        assert!(!<Simd<i32, 16> as SlideLanes>::admits(31, 0));
        assert!(<Simd<i32, 16> as SlideLanes>::admits(32, 0));
    }
}
