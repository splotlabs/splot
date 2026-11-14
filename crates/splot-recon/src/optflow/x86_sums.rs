// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The x86_64 form of the § 7.13.3.9 sums of an 8x8 optical-flow unit: rows
//! in pairs, row `2k` in the low 128-bit block of pair `k` and row `2k + 1`
//! in the high block, so that one 256-bit operation and one `pmaddwd` serve
//! two rows and the sums need no recombination.

use fearless_simd::{i16x8, i16x16, i32x8, mask16x16, prelude::*, u8x32, u16x16, u64x4};

use super::{GradientTile, accumulate_unit_sums, gradient_row, solve_sums, unit_sums, whole_8x8};
use crate::Result;

/// Byte indices that reverse the eight `i16` lanes of each 128-bit block.
const REVERSE: [u8; 32] = {
    let mut out = [0; 32];
    let mut byte = 0;
    while byte < 32 {
        out[byte] = ((byte / 16) * 16 + 14 - (byte % 16) / 2 * 2 + byte % 2) as u8;
        byte += 1;
    }
    out
};

/// [`super::optflow_unit_8x8_u8`] of its eight-byte rows and reduced
/// `distances`, in row pairs.
#[inline]
pub(super) fn unit_8x8<S: Simd>(
    simd: S,
    rows: &[[u64; 8]; 2],
    distances: [i32; 2],
    sad_threshold: u32,
) -> Result<Option<[[i32; 2]; 2]>> {
    let [first, second] = rows.each_ref().map(|rows| byte_rows(simd, rows));
    let difference: [i16x16<S>; 4] = core::array::from_fn(|pair| first[pair] - second[pair]);
    let sad = difference
        .iter()
        .fold(i16x16::splat(simd, 0), |sum, pair| sum + pair.abs());
    if u32::from(sad.reduce_sum().unsigned_abs()) < sad_threshold {
        return Ok(None);
    }
    if distances == [0; 2] {
        return Ok(Some([[0; 2]; 2]));
    }
    let [near, far] = distances.map(|distance| distance as i16);
    let weighted = core::array::from_fn(|pair| first[pair] * near - second[pair] * far);
    solve_sums(whole_8x8(sums(simd, weighted, difference)), distances).map(Some)
}

/// [`super::unit_16x16_sums`] with whole 16-lane rows: the left unit's
/// products fill the low 128-bit block and the right unit's the high one.
#[inline]
pub(super) fn unit_16x16_lane_sums<S: Simd>(
    simd: S,
    rows: &GradientTile<'_>,
    difference: &GradientTile<'_>,
) -> [[i32; 5]; 4] {
    let half = |first: usize| {
        let mut sums = [i32x8::splat(simd, 0); 5];
        for (row, difference) in difference.iter().enumerate().skip(first).take(8) {
            let (u, v) = gradient_row(simd, rows, row);
            let w = i16x16::load_array_ref(simd, difference);
            accumulate_unit_sums(&mut sums, u, v, w);
        }
        let blocks = sums.map(|lanes| {
            let (left, right) = lanes.split();
            [left.reduce_sum(), right.reduce_sum()]
        });
        [0, 1].map(|unit| unit_sums(blocks.map(|sums| sums[unit]), 64))
    };
    let ([top_left, top_right], [bottom_left, bottom_right]) = (half(0), half(8));
    [top_left, top_right, bottom_left, bottom_right]
}

/// The lane sums of the top and bottom halves of an 8x8 unit of rows in
/// lanes, paired first.
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
pub(super) fn rows_8x8_sums<S: Simd>(
    simd: S,
    weighted: &[i16x8<S>; 8],
    difference: &[i16x8<S>; 8],
) -> [[i32x8<S>; 5]; 2] {
    let pairs = |rows: &[i16x8<S>; 8]| -> [i16x16<S>; 4] {
        core::array::from_fn(|pair| rows[2 * pair].combine(rows[2 * pair + 1]))
    };
    sums(simd, pairs(weighted), pairs(difference))
}

/// [`rows_8x8_sums`] of rows that are already paired.
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn sums<S: Simd>(
    simd: S,
    weighted: [i16x16<S>; 4],
    difference: [i16x16<S>; 4],
) -> [[i32x8<S>; 5]; 2] {
    let blocks = weighted.map(SimdSplit::split);
    let join = |low: i16x8<S>, high: i16x8<S>| low.combine(high);
    let first = join(blocks[0].0, blocks[0].0);
    let shifted = [
        join(blocks[0].1, blocks[1].0),
        join(blocks[1].1, blocks[2].0),
        join(blocks[2].1, blocks[3].0),
        join(blocks[3].1, blocks[3].1),
    ];
    let near = [
        [shifted[0], first],
        [shifted[1], shifted[0]],
        [shifted[2], shifted[1]],
        [shifted[3], shifted[2]],
    ];
    let far = [
        [weighted[1], first],
        [weighted[2], weighted[0]],
        [weighted[3], weighted[1]],
        [shifted[3], weighted[2]],
    ];
    let reverse = u8x32::load_array(simd, REVERSE);
    let mut halves = [[i32x8::splat(simd, 0); 5]; 2];
    for pair in 0..4 {
        let rows = weighted[pair];
        let reversed = rows.swizzle_dyn_within_blocks(reverse);
        let next = rows.slide_within_blocks::<1>(reversed);
        let prev = reversed.slide_within_blocks::<7>(rows);
        let u = gradient(
            [next, prev],
            [
                next.slide_within_blocks::<1>(reversed),
                reversed.slide_within_blocks::<7>(prev),
            ],
            |magnitude| edge_round(magnitude, 0x8181),
        );
        let v = match pair {
            0 => gradient(near[0], far[0], |magnitude| edge_round(magnitude, 0x00ff)),
            3 => gradient(near[3], far[3], |magnitude| edge_round(magnitude, 0xff00)),
            _ => gradient(near[pair], far[pair], |magnitude| (magnitude + 64) >> 7),
        };
        let w = difference[pair];
        let sums = &mut halves[pair / 2];
        sums[0] += u.pairwise_dot_product(u);
        sums[1] += v.pairwise_dot_product(v);
        sums[2] += u.pairwise_dot_product(v);
        sums[3] += u.pairwise_dot_product(w);
        sums[4] += v.pairwise_dot_product(w);
    }
    halves
}

/// The eight-bit rows of an 8x8 prediction, two rows to a pair.
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn byte_rows<S: Simd>(simd: S, rows: &[u64; 8]) -> [i16x16<S>; 4] {
    let (quads, _) = rows.as_chunks::<4>();
    let [(first, second), (third, fourth)] = [0, 1].map(|quad| {
        u64x4::load_array_ref(simd, &quads[quad])
            .bitcast::<u8x32<S>>()
            .widen()
    });
    [first, second, third, fourth].map(fearless_simd::Bytes::bitcast)
}

/// `(magnitude + 64) >> 7` with the `edges` lanes, whose taps are doubled,
/// rounded by 6 bits instead.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn edge_round<S: Simd>(magnitude: u16x16<S>, edges: u64) -> u16x16<S> {
    mask16x16::from_bitmask(magnitude.simd, edges)
        .select((magnitude + 32) >> 6, (magnitude + 64) >> 7)
}

/// [`super::gradient_i16`] of sixteen lanes.
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient<S: Simd>(
    near: [i16x16<S>; 2],
    far: [i16x16<S>; 2],
    round: impl FnOnce(u16x16<S>) -> u16x16<S>,
) -> i16x16<S> {
    let near = (near[0] - near[1]) * 42;
    let far = (far[0] - far[1]) * 5;
    let magnitude: i16x16<S> = round((near.max(far) - near.min(far)).bitcast()).bitcast();
    let sign = far.simd_gt(near).to_vector();
    (magnitude ^ sign) - sign
}
