// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The two passes of an unscaled § 7.13.3.20 extended-warp 4x4 unit as
//! `i16` multiply-adds.
//!
//! A horizontal output column is the dot product of its six samples with the
//! phase's taps; the window at the unit's first (second) column fills both
//! halves of a 16-lane vector, and the taps at lane 0 and shifted by two lanes
//! give its first and third (second and fourth) columns. An 8- or 10-bit
//! horizontal value stays below `155 * 1023 / 8`, so the intermediate fits
//! `i16`, and the vertical pass pairs consecutive intermediate rows as in
//! `subpel_mc::pairs`. Integer sums are exact, so the regrouping leaves the
//! per-tap result unchanged.

use super::{EXT_WARP_TAPS, hadd_blocks};
use fearless_simd::{Simd, i16x8, i16x16, i32x4, i32x8, prelude::*};
use splot_core::tables::warp_filter::EXT_WARPED_FILTERS;

const PHASES: usize = EXT_WARPED_FILTERS.len();

/// Each phase's taps at lanes `0..6`, then shifted to lanes `10..16`. Loading
/// them keeps the weights opaque, which keeps the multiply-add one `pmaddwd`.
const HORIZONTAL: [[i16; 16]; PHASES] = {
    let mut taps = [[0; 16]; PHASES];
    let mut phase = 0;
    while phase < PHASES {
        let mut tap = 0;
        while tap < EXT_WARP_TAPS {
            let value = EXT_WARPED_FILTERS[phase][tap] as i16;
            taps[phase][tap] = value;
            taps[phase][10 + tap] = value;
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

/// The four horizontal sums of a row from its samples at the unit's first
/// column (`first`) and the next (`second`).
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
pub(super) fn horizontal<S: Simd>(
    simd: S,
    first: i16x8<S>,
    second: i16x8<S>,
    phase: usize,
) -> i32x4<S> {
    let taps = i16x16::load_array_ref(simd, &HORIZONTAL[phase % PHASES]);
    let sums = |window: i16x8<S>| window.combine(window).pairwise_dot_product(taps);
    let (low, high) = hadd_blocks(simd, sums(first), sums(second)).split();
    low.unzip_low(high) + low.unzip_high(high)
}

/// `Round2` of the vertical sums of the unit's nine intermediate rows of four
/// lanes, in row-major order.
#[allow(clippy::inline_always, reason = "measured warp hot path")]
#[inline(always)]
pub(super) fn vertical<S: Simd>(
    simd: S,
    intermediate: &[i16; 36],
    phase: usize,
    round1: u32,
) -> [i32; 16] {
    let pairs = &VERTICAL[phase % PHASES];
    let line = |k: usize| {
        let lanes = intermediate
            .get(4 * k..)
            .and_then(<[i16]>::first_chunk::<16>);
        lanes.map_or(i16x16::splat(simd, 0), |lanes| {
            i16x16::load_array_ref(simd, lanes)
        })
    };
    let bias = i32x8::splat(simd, 1 << (round1 - 1));
    let (mut low, mut high) = (bias, bias);
    for (pair, weights) in pairs.iter().enumerate() {
        let ((a0, a1), (b0, b1)) = (line(2 * pair).split(), line(2 * pair + 1).split());
        let weights = i16x16::load_array_ref(simd, weights);
        let (near, far) = (a0.interleave(b0), a1.interleave(b1));
        low += near.0.combine(far.0).pairwise_dot_product(weights);
        high += near.1.combine(far.1).pairwise_dot_product(weights);
    }
    let ((row0, row2), (row1, row3)) = ((low >> round1).split(), (high >> round1).split());
    let mut rows = [0; 16];
    for (rows, row) in rows
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip([row0, row1, row2, row3])
    {
        *rows = row.to_array();
    }
    rows
}
