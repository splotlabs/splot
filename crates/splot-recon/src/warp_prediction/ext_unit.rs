// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The two passes of an unscaled § 7.13.3.20 extended-warp 4x4 unit as
//! `i16` multiply-adds.
//!
//! A horizontal output column is the dot product of its six samples with
//! the phase's taps; two columns share one 16-lane vector whose halves are the
//! sample windows at the unit's first and second column, and the taps shifted
//! by two lanes give the third and fourth. An 8- or 10-bit horizontal value
//! stays below `155 * 1023 / 8`, so the intermediate fits `i16`, and the
//! vertical pass pairs consecutive intermediate rows as in
//! `subpel_mc::pairs`. Integer sums are exact, so the regrouping leaves the
//! per-tap result unchanged.

use super::EXT_WARP_TAPS;
use splot_core::tables::warp_filter::EXT_WARPED_FILTERS;
use std::simd::{Simd, num::SimdInt, simd_swizzle};

const PHASES: usize = EXT_WARPED_FILTERS.len();

/// Each phase's taps at lanes `0..6` (and `8..14`), then shifted to lanes
/// `2..8` (and `10..16`). Loading them keeps the weights opaque, which keeps
/// the multiply-add one `pmaddwd`.
const HORIZONTAL: [[[i16; 16]; 2]; PHASES] = {
    let mut taps = [[[0; 16]; 2]; PHASES];
    let mut phase = 0;
    while phase < PHASES {
        let mut tap = 0;
        while tap < EXT_WARP_TAPS {
            let value = EXT_WARPED_FILTERS[phase][tap] as i16;
            taps[phase][0][tap] = value;
            taps[phase][0][8 + tap] = value;
            taps[phase][1][2 + tap] = value;
            taps[phase][1][10 + tap] = value;
            tap += 1;
        }
        phase += 1;
    }
    taps
};

/// Each phase's tap pairs `2j, 2j + 1` repeated across 16 lanes.
const VERTICAL: [[[i16; 16]; EXT_WARP_TAPS / 2]; PHASES] = {
    let mut pairs = [[[0; 16]; EXT_WARP_TAPS / 2]; PHASES];
    let mut phase = 0;
    while phase < PHASES {
        let mut lane = 0;
        while lane < 16 * (EXT_WARP_TAPS / 2) {
            pairs[phase][lane / 16][lane % 16] =
                EXT_WARPED_FILTERS[phase][2 * (lane / 16) + lane % 2] as i16;
            lane += 1;
        }
        phase += 1;
    }
    pairs
};

#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
fn multiply_add_pairs(values: Simd<i16, 16>, weights: Simd<i16, 16>) -> Simd<i32, 8> {
    let product = values.cast::<i32>() * weights.cast::<i32>();
    simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15])
}

/// The four horizontal sums of a row from its samples at the unit's first
/// column (`first`) and the next (`second`).
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
pub(super) fn horizontal(first: Simd<i16, 8>, second: Simd<i16, 8>, phase: usize) -> Simd<i32, 4> {
    let taps = &HORIZONTAL[phase % PHASES];
    let samples: Simd<i16, 16> = simd_swizzle!(
        first,
        second,
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    let near = multiply_add_pairs(samples, Simd::from_array(taps[0]));
    let far = multiply_add_pairs(samples, Simd::from_array(taps[1]));
    let halves = simd_swizzle!(near, far, [0, 2, 8, 10, 4, 6, 12, 14])
        + simd_swizzle!(near, far, [1, 3, 9, 11, 5, 7, 13, 15]);
    simd_swizzle!(halves, [0, 4, 2, 6]) + simd_swizzle!(halves, [1, 5, 3, 7])
}

/// `Round2` of the vertical sums of the unit's nine intermediate rows of four
/// lanes, in row-major order.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
pub(super) fn vertical(intermediate: &[i16; 36], phase: usize, round1: u32) -> [i32; 16] {
    let pairs = &VERTICAL[phase % PHASES];
    let line = |k: usize| Simd::<i16, 16>::from_slice(&intermediate[4 * k..]);
    let bias = Simd::<i32, 8>::splat(1 << (round1 - 1));
    let (mut low, mut high) = (bias, bias);
    for (pair, weights) in pairs.iter().enumerate() {
        let (a, b) = (line(2 * pair), line(2 * pair + 1));
        let weights = Simd::from_array(*weights);
        low += multiply_add_pairs(
            simd_swizzle!(
                a,
                b,
                [0, 16, 1, 17, 2, 18, 3, 19, 8, 24, 9, 25, 10, 26, 11, 27]
            ),
            weights,
        );
        high += multiply_add_pairs(
            simd_swizzle!(
                a,
                b,
                [4, 20, 5, 21, 6, 22, 7, 23, 12, 28, 13, 29, 14, 30, 15, 31]
            ),
            weights,
        );
    }
    let rows: Simd<i32, 16> = simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    );
    (rows >> round1 as i32).to_array()
}
