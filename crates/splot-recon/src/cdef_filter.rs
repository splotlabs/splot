// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18 CDEF (Constrained Directional Enhancement Filter) sample math.
//!
//! This module implements the scheduler-free per-block AV2 CDEF primitives
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)):
//! the § 7.18.2 direction search ([`cdef_direction`], `#s-7-18-2`), which measures
//! the dominant direction (`yDir`) and variance (`var`) of an 8x8 luma block from
//! the pre-shifted (`>> (BitDepth - 8)`, minus 128) reconstructed samples using the
//! § 7.18.2 `partial[][]` accumulation and the `Div_Table` cost; the § 7.18.3
//! `constrain` clamp ([`cdef_constrain`], `#s-7-18-3`); and the § 7.18.3 per-sample
//! primary/secondary tap accumulation ([`cdef_filter_sample`], `#s-7-18-3`), which
//! combines the center sample, the two primary directional taps, and the four
//! secondary directional taps (each already fetched and availability-flagged by the
//! caller) into one deringed sample via `Cdef_Pri_Taps` / `Cdef_Sec_Taps`, the
//! `(8 + sum - (sum < 0)) >> 4` rounding, and the `Clip3(min, max, ...)` clamp.
//!
//! The § 7.18.3 `Cdef_Directions` direction-offset table and the § 7.18.1
//! `Cdef_Uv_Dir` chroma-direction remap are exposed as constants for the caller's
//! `cdef_get_at` neighbour addressing; the § 7.18.2 `Div_Table` and the § 7.18.3
//! `Cdef_Pri_Taps` / `Cdef_Sec_Taps` are consumed internally.
//!
//! Scope: these are the per-block direction/variance derivation and the per-sample
//! tap math over caller-resolved spec-derived values. The § 7.18 / § 7.18.1
//! 64x64-unit → 8x8-block traversal, the `cdef_idx` per-64x64 parameter lookup, the
//! § 7.18.1 `skip` / `skipChroma` / strength derivation, the § 5.20.9.3
//! `is_inside_filter_region` availability check, and the `CurrFrame` / `CdefFrame`
//! sample I/O stay with the caller — it passes the resolved 8x8 luma block, the
//! center value, the fetched directional neighbours, and the strength / damping
//! scalars, exactly as the other `splot-recon` primitives take caller-resolved
//! spec-derived values. It does not read frame, segment, or tile state.
//!
//! Feature tracking: `RECON-CDEF-FILTER`.

use std::simd::{
    Select, Simd, cmp::SimdOrd, cmp::SimdPartialEq, cmp::SimdPartialOrd, num::SimdInt,
    num::SimdUint, simd_swizzle,
};

/// AV2 § 7.18.2 `Div_Table[9]`: reciprocal-scaling weights for the direction cost.
const DIV_TABLE: [i32; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];

/// AV2 § 7.18.3 `Cdef_Pri_Taps[2][2]`: primary-tap weights, selected by
/// `(priStr >> coeffShift) & 1`.
const CDEF_PRI_TAPS: [[i32; 2]; 2] = [[4, 2], [3, 3]];

/// AV2 § 7.18.3 `Cdef_Sec_Taps[2][2]`: secondary-tap weights, selected by
/// `(priStr >> coeffShift) & 1`.
const CDEF_SEC_TAPS: [[i32; 2]; 2] = [[2, 1], [2, 1]];

/// AV2 § 7.18.3 `Cdef_Directions[8][2][2]`: the `(dy, dx)` neighbour offsets for
/// direction `dir` and tap index `k` (`[dir][k][0]` is the row offset, `[dir][k][1]`
/// the column offset). The caller's `cdef_get_at` adds `sign * Cdef_Directions[dir]
/// [k]` to the sample position.
pub const CDEF_DIRECTIONS: [[[i32; 2]; 2]; 8] = [
    [[-1, 1], [-2, 2]],
    [[0, 1], [-1, 2]],
    [[0, 1], [0, 2]],
    [[0, 1], [1, 2]],
    [[1, 1], [2, 2]],
    [[1, 0], [2, 1]],
    [[1, 0], [2, 0]],
    [[1, 0], [2, -1]],
];

/// AV2 § 7.18.1 `Cdef_Uv_Dir[2][2][8]`: the chroma direction remap, indexed by
/// `[SubsamplingX][SubsamplingY][yDir]`, producing the chroma `dir` from the luma
/// `yDir`.
pub const CDEF_UV_DIR: [[[usize; 8]; 2]; 2] = [
    [[0, 1, 2, 3, 4, 5, 6, 7], [1, 2, 2, 2, 3, 4, 6, 0]],
    [[7, 0, 2, 4, 5, 6, 6, 6], [0, 1, 2, 3, 4, 5, 6, 7]],
];

/// AV2 § 4.7 `FloorLog2(x)`: the position of the most significant set bit
/// (`FloorLog2(0)` is unreachable in CDEF — `constrain` short-circuits on a zero
/// threshold and `var >> 6` is gated nonzero before its `FloorLog2`).
const fn floor_log2(x: u32) -> u32 {
    if x == 0 { 0 } else { x.ilog2() }
}

/// AV2 § 7.18.2 CDEF direction process.
///
/// `block` is the 8x8 luma neighbourhood with each sample already reduced to the
/// spec's `x = (CurrFrame[0][y0+i][x0+j] >> (BitDepth - 8)) - 128` form (`block[i][j]`
/// = row `i`, column `j`). Returns `(yDir, var)`: the dominant direction index in
/// `0..8` and the variance `var = (bestCost - cost[(yDir + 4) & 7]) >> 10`.
///
/// The `partial[][]` sums of eight 8-bit-normalized terms fit in `i16`.
/// The squared partials times `Div_Table` use the same `i32` accumulators as AVM;
/// the spec-bounded pre-shifted samples keep every directional cost in range.
pub fn cdef_direction(block: &[[i32; 8]; 8]) -> (usize, i32) {
    debug_assert!(block.iter().flatten().all(|&x| (-128..=127).contains(&x)));
    cdef_direction_rows(block.map(|row| Simd::from_array(row).cast::<i16>()))
}

/// AV2 § 7.18.2 CDEF direction process over the interior padded block layout.
///
/// The 8x8 luma block begins at row 2, column 2 of `pad`; `coeff_shift` is
/// `BitDepth - 8`. The result matches [`cdef_direction`] without materializing
/// the intermediate shifted 8x8 array.
pub fn cdef_direction_padded(pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32) -> (usize, i32) {
    cdef_direction_strided::<CDEF_PADDED_SIDE, _>(pad, coeff_shift)
}

/// [`cdef_direction_padded`] for one block of the luma segment scratch that
/// [`cdef_filter_block_segment`] reads.
pub fn cdef_direction_segment(
    pad: &[u16; CDEF_SEGMENT_BLOCK_AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    cdef_direction_strided::<CDEF_SEGMENT_STRIDE, _>(pad, coeff_shift)
}

#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_direction_strided<const STRIDE: usize, const AREA: usize>(
    pad: &[u16; AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    let rows = core::array::from_fn(|i| {
        let start = (i + 2) * STRIDE + 2;
        (Simd::<u16, 8>::from_slice(&pad[start..start + 8]) >> coeff_shift as u16).cast::<i16>()
            - Simd::splat(128)
    });
    cdef_direction_rows(rows)
}

/// A 16-lane partial-sum accumulator held as its low and high halves.
type CdefPartial = [Simd<i16, 8>; 2];

/// Moves every lane of a partial up by one, dropping the top lane.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn shift_partial_up(partial: CdefPartial) -> CdefPartial {
    let zero = Simd::splat(0);
    [
        simd_swizzle!(zero, partial[0], [7, 8, 9, 10, 11, 12, 13, 14]),
        simd_swizzle!(partial[0], partial[1], [7, 8, 9, 10, 11, 12, 13, 14]),
    ]
}

/// Moves every lane of a partial down by one, dropping lane 0.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn shift_partial_down(partial: CdefPartial) -> CdefPartial {
    let zero = Simd::splat(0);
    [
        simd_swizzle!(partial[0], partial[1], [1, 2, 3, 4, 5, 6, 7, 8]),
        simd_swizzle!(partial[1], zero, [1, 2, 3, 4, 5, 6, 7, 8]),
    ]
}

/// Sums of adjacent lane pairs, `a`'s pairs then `b`'s.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn pairwise_sum(a: Simd<i16, 8>, b: Simd<i16, 8>) -> Simd<i16, 8> {
    simd_swizzle!(a, b, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(a, b, [1, 3, 5, 7, 9, 11, 13, 15])
}

/// `Σ partial² · weight` over a partial, in four `i32` lanes that sum to it.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_cost(partial: CdefPartial, weights: [[i32; 8]; 2]) -> Simd<i32, 4> {
    let low = partial[0].cast::<i32>();
    let high = partial[1].cast::<i32>();
    let cost =
        low * low * Simd::from_array(weights[0]) + high * high * Simd::from_array(weights[1]);
    simd_swizzle!(cost, [0, 1, 2, 3]) + simd_swizzle!(cost, [4, 5, 6, 7])
}

/// [`cdef_cost`] of a line partial, every entry of which weighs `Div_Table[8]`.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_line_cost(partial: Simd<i16, 8>) -> Simd<i32, 4> {
    let square = partial.cast::<i32>() * partial.cast::<i32>();
    (simd_swizzle!(square, [0, 1, 2, 3]) + simd_swizzle!(square, [4, 5, 6, 7]))
        * Simd::splat(DIV_TABLE[8])
}

/// The eight costs, each summed from its four lanes by a pairwise-add tree.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_cost_sums(lanes: [Simd<i32, 4>; 8]) -> [Simd<i32, 4>; 2] {
    let pairs = |a: Simd<i32, 4>, b: Simd<i32, 4>| {
        simd_swizzle!(a, b, [0, 2, 4, 6]) + simd_swizzle!(a, b, [1, 3, 5, 7])
    };
    [0, 4].map(|at| {
        pairs(
            pairs(lanes[at], lanes[at + 1]),
            pairs(lanes[at + 2], lanes[at + 3]),
        )
    })
}

/// § 7.18.2 `Div_Table` weights of a `len`-entry partial (15 for the
/// diagonals, 11 for the alternates) whose entry `n` sits in lane `n + lane`.
const fn cdef_cost_weights(len: usize, lane: usize) -> [[i32; 8]; 2] {
    let mut weights = [[0; 8]; 2];
    let mut n = 0;
    while n < len {
        let mirror = if n < len - 1 - n { n } else { len - 1 - n };
        weights[(n + lane) / 8][(n + lane) % 8] = if len == 15 {
            DIV_TABLE[mirror + 1]
        } else if mirror >= 3 {
            DIV_TABLE[8]
        } else {
            DIV_TABLE[2 * mirror + 2]
        };
        n += 1;
    }
    weights
}

/// § 7.18.2 partial sums and costs over eight pre-shifted rows.
///
/// Each partial is built by Horner steps: before row `i` is added, the
/// accumulator moves one lane, so row `i` ends `7 - i` lanes from where it was
/// added. Partials 1, 4 and 5 grow upward and partials 0, 3 and 7 downward,
/// the last three ending one or five lanes up. Every cost weighs entry `n`
/// like entry `len - 1 - n`, so the reversed order of partials 3 and 4 does
/// not change it. Costs are never negative, so the § 7.18.2 scan, which
/// keeps a direction only when its cost beats the best so far (from 0),
/// ends on the first direction that holds the largest cost.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_direction_rows(rows: [Simd<i16, 8>; 8]) -> (usize, i32) {
    let zero = Simd::splat(0);
    let mut partial0 = [zero; 2];
    let mut partial1 = [zero; 2];
    let mut partial3 = [zero; 2];
    let mut partial4 = [zero; 2];
    let mut partial5 = [zero; 2];
    let mut partial7 = [zero; 2];
    let mut vertical = zero;
    let mut row_sums = [zero; 4];
    for (k, row_sum) in row_sums.iter_mut().enumerate() {
        let (first, second) = (rows[2 * k], rows[2 * k + 1]);
        *row_sum = pairwise_sum(first, second);
        let first_pairs = simd_swizzle!(*row_sum, zero, [0, 1, 2, 3, 8, 8, 8, 8]);
        let second_pairs = simd_swizzle!(*row_sum, zero, [4, 5, 6, 7, 8, 8, 8, 8]);
        for (row, pairs) in [(first, first_pairs), (second, second_pairs)] {
            partial0 = shift_partial_down(partial0);
            partial0[1] += row;
            partial4 = shift_partial_up(partial4);
            partial4[0] += row;
            partial1 = shift_partial_down(partial1);
            partial1[1] += pairs;
            partial3 = shift_partial_up(partial3);
            partial3[0] += pairs;
        }
        let both = first + second;
        partial5 = shift_partial_up(partial5);
        partial5[0] += both;
        partial7 = shift_partial_down(partial7);
        partial7[1] += both;
        vertical += both;
    }
    let horizontal = pairwise_sum(
        pairwise_sum(row_sums[0], row_sums[1]),
        pairwise_sum(row_sums[2], row_sums[3]),
    );
    let [low, high] = cdef_cost_sums([
        cdef_cost(partial0, const { cdef_cost_weights(15, 1) }),
        cdef_cost(partial1, const { cdef_cost_weights(11, 1) }),
        cdef_line_cost(horizontal),
        cdef_cost(partial3, const { cdef_cost_weights(11, 0) }),
        cdef_cost(partial4, const { cdef_cost_weights(15, 0) }),
        cdef_cost(partial5, const { cdef_cost_weights(11, 0) }),
        cdef_line_cost(vertical),
        cdef_cost(partial7, const { cdef_cost_weights(11, 5) }),
    ]);
    let best_cost = low.simd_max(high).reduce_max();
    let first = |cost: Simd<i32, 4>, dir: [u32; 4]| {
        cost.simd_eq(Simd::splat(best_cost))
            .select(Simd::from_array(dir), Simd::splat(8))
    };
    let y_dir = first(low, [0, 1, 2, 3])
        .simd_min(first(high, [4, 5, 6, 7]))
        .reduce_min() as usize;
    let cost = simd_swizzle!(low, high, [0, 1, 2, 3, 4, 5, 6, 7]).to_array();
    let var = (best_cost - cost[(y_dir + 4) & 7]) >> 10;
    (y_dir, var)
}

/// AV2 § 7.18.3 `constrain(diff, threshold, damping)`.
///
/// Returns `0` when `threshold` is `0`; otherwise it signs and clamps `diff` toward
/// zero by `threshold - (Abs(diff) >> dampingAdj)` where `dampingAdj = Max(0, damping
/// - FloorLog2(threshold))`.
pub fn cdef_constrain(diff: i32, threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let damping_adj = (damping - floor_log2(threshold as u32) as i32).max(0);
    let abs = diff.abs();
    let magnitude = (threshold - (abs >> damping_adj)).clamp(0, abs);
    if diff < 0 { -magnitude } else { magnitude }
}

/// One CDEF directional neighbour fetched by the caller's `cdef_get_at`: its value
/// and whether it was inside the filter region (`CdefAvailable`).
#[derive(Clone, Copy, Debug)]
pub struct CdefTap {
    /// The fetched `CurrFrame[plane][y][x]` sample value (ignored when `!available`).
    pub value: i32,
    /// `CdefAvailable`: whether the candidate position was inside the filter region.
    pub available: bool,
}

/// AV2 § 7.18.3 per-sample primary/secondary tap inputs for one output sample.
///
/// `center` is `CurrFrame[plane][y0 + i][x0 + j]` (the `x` of § 7.18.3). For each
/// `k` in `0..2` and each `sign` in `{-1, +1}`, `primary[k][sign_index]` is the
/// `cdef_get_at(..., dir, k, sign, ...)` primary tap and `secondary[k][sign_index]`
/// holds the two `(dir + dirOff) & 7` (`dirOff in {-2, +2}`) secondary taps. The
/// `sign_index` is `0` for `sign == -1` and `1` for `sign == +1`.
#[derive(Clone, Copy, Debug)]
pub struct CdefSampleTaps {
    /// The center sample value (`x`).
    pub center: i32,
    /// Primary taps `[k][sign_index]`.
    pub primary: [[CdefTap; 2]; 2],
    /// Secondary taps `[k][sign_index][dir_off_index]`.
    pub secondary: [[[CdefTap; 2]; 2]; 2],
}

/// AV2 § 7.18.3 per-sample CDEF filter: combines the center sample, the two primary
/// directional taps, and the four secondary directional taps into one deringed
/// output sample.
///
/// `pri_str` / `sec_str` are the bit-depth-scaled primary / secondary strengths
/// (`cdef_*_pri_strength << coeffShift` etc.), `damping` the § 7.18.1 damping shift,
/// and `coeff_shift` is `BitDepth - 8` (selects the `Cdef_Pri_Taps` / `Cdef_Sec_Taps`
/// row via `(pri_str >> coeff_shift) & 1`). Unavailable taps are skipped (they
/// contribute neither to `sum` nor to the `min` / `max` clamp), matching the spec's
/// `if (CdefAvailable)` guard.
pub fn cdef_filter_sample(
    taps: &CdefSampleTaps,
    pri_str: i32,
    sec_str: i32,
    damping: i32,
    coeff_shift: u32,
) -> i32 {
    let tap_row = ((pri_str >> coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(pri_str, damping);
    let sec_adj = constrain_damping_adj(sec_str, damping);

    let mut sum = 0i32;
    let mut max = taps.center;
    let mut min = taps.center;
    for k in 0..2 {
        for sign_index in 0..2 {
            let p = taps.primary[k][sign_index];
            if p.available {
                sum += pri_taps[k] * constrain_with_adj(p.value - taps.center, pri_str, pri_adj);
                max = max.max(p.value);
                min = min.min(p.value);
            }
            for dir_off_index in 0..2 {
                let s = taps.secondary[k][sign_index][dir_off_index];
                if s.available {
                    sum +=
                        sec_taps[k] * constrain_with_adj(s.value - taps.center, sec_str, sec_adj);
                    max = max.max(s.value);
                    min = min.min(s.value);
                }
            }
        }
    }

    let rounded = taps.center + ((8 + sum - i32::from(sum < 0)) >> 4);
    rounded.clamp(min, max)
}

/// Side length of the padded per-block scratch consumed by
/// [`cdef_filter_block_interior`]: an 8x8 block plus the § 7.18.3
/// `Cdef_Directions` tap reach of 2 on every side.
pub const CDEF_PADDED_SIDE: usize = 12;

/// Sample count of the padded per-block scratch. It holds the
/// `CDEF_PADDED_SIDE`-square layout and leaves room for a tap view of up to
/// 96 samples from any byte-sized start, so binding a view needs no bounds
/// check.
pub const CDEF_PADDED_AREA: usize = u8::MAX as usize + 97;

/// Padded-tap marker used by the SIMD boundary kernel for unavailable samples.
pub const CDEF_UNAVAILABLE: u16 = i16::MAX as u16;

/// Blocks in one row segment of the strip scratch that
/// [`cdef_filter_block_segment`] and [`cdef_filter_block_chroma_pair_segment`]
/// read: the segment is gathered once for all of its blocks.
pub const CDEF_SEGMENT_BLOCKS: usize = 4;

/// Lanes per row of the luma segment scratch: the segment's 8-sample blocks
/// plus the tap reach of two on each side. Block `b` starts at lane `8 * b`.
pub const CDEF_SEGMENT_STRIDE: usize = 8 * CDEF_SEGMENT_BLOCKS + 4;

/// Samples of the luma segment scratch from one block's first lane on: room
/// for a tap view from any byte start, as in [`CDEF_PADDED_AREA`].
pub const CDEF_SEGMENT_BLOCK_AREA: usize = u8::MAX as usize + 7 * CDEF_SEGMENT_STRIDE + 8;

/// Lanes per row of the interleaved chroma-pair segment scratch: four
/// columns of both planes per block plus the tap reach. Block `b` starts at
/// lane `8 * b`.
pub const CDEF_PAIR_SEGMENT_STRIDE: usize = 8 * CDEF_SEGMENT_BLOCKS + 8;

/// [`CDEF_SEGMENT_BLOCK_AREA`] for the chroma-pair segment scratch.
pub const CDEF_PAIR_SEGMENT_BLOCK_AREA: usize = u8::MAX as usize + 3 * CDEF_PAIR_SEGMENT_STRIDE + 8;

/// Per-block § 7.18.3 filter constants for [`cdef_filter_block_interior`].
#[derive(Clone, Copy, Debug)]
pub struct CdefBlockFilter {
    /// Bit-depth-scaled (and, for luma, variance-adjusted) primary strength.
    pub pri_str: i32,
    /// Bit-depth-scaled secondary strength.
    pub sec_str: i32,
    /// § 7.18.1 damping shift, already plane-adjusted.
    pub damping: i32,
    /// Direction index in `0..8`.
    pub dir: usize,
    /// `BitDepth - 8`.
    pub coeff_shift: u32,
}

type CdefPrimaryOffsets = [[isize; 2]; 2];
type CdefSecondaryOffsets = [[[isize; 2]; 2]; 2];
type CdefTapOffsets = [(CdefPrimaryOffsets, CdefSecondaryOffsets); 8];

/// Tap displacements for every direction in a padded layout `stride` lanes per
/// row whose column step is `col_step` lanes.
const fn cdef_tap_offsets(stride: isize, col_step: isize) -> CdefTapOffsets {
    let mut offsets = [([[0; 2]; 2], [[[0; 2]; 2]; 2]); 8];
    let mut dir = 0;
    while dir < 8 {
        let mut tap = 0;
        while tap < 2 {
            let mut sign_index = 0;
            while sign_index < 2 {
                let sign = if sign_index == 0 { -1 } else { 1 };
                let mut slot = 0;
                while slot < 3 {
                    let rotated = [dir, dir + 6, dir + 2][slot];
                    let [dy, dx] = CDEF_DIRECTIONS[rotated & 7][tap];
                    let offset = (sign * dy) as isize * stride + (sign * dx) as isize * col_step;
                    if slot == 0 {
                        offsets[dir].0[tap][sign_index] = offset;
                    } else {
                        offsets[dir].1[tap][sign_index][slot - 1] = offset;
                    }
                    slot += 1;
                }
                sign_index += 1;
            }
            tap += 1;
        }
        dir += 1;
    }
    offsets
}

const CDEF_RELATIVE_OFFSETS: CdefTapOffsets = cdef_tap_offsets(CDEF_PADDED_SIDE as isize, 1);

/// Scratch index of each direction's twelve taps for the block's first
/// sample, four primary taps (`[k][sign]`) then eight secondary
/// (`[k][sign][dirOff]`).
type CdefTapStarts = [[u8; 12]; 8];

const fn cdef_tap_starts(offsets: &CdefTapOffsets, center: usize) -> CdefTapStarts {
    let mut starts = [[0; 12]; 8];
    let mut dir = 0;
    while dir < 8 {
        let (primary, secondary) = &offsets[dir];
        let mut tap = 0;
        while tap < 2 {
            let mut sign = 0;
            while sign < 2 {
                let center = center as isize;
                starts[dir][tap * 2 + sign] = tap_start(center + primary[tap][sign]);
                starts[dir][4 + tap * 4 + sign * 2] = tap_start(center + secondary[tap][sign][0]);
                starts[dir][5 + tap * 4 + sign * 2] = tap_start(center + secondary[tap][sign][1]);
                sign += 1;
            }
            tap += 1;
        }
        dir += 1;
    }
    starts
}

const fn tap_start(start: isize) -> u8 {
    assert!(0 <= start && start <= u8::MAX as isize);
    start as u8
}

const CDEF_TAP_STARTS: CdefTapStarts =
    cdef_tap_starts(&CDEF_RELATIVE_OFFSETS, 2 * CDEF_PADDED_SIDE + 2);

const CDEF_SEGMENT_TAP_STARTS: CdefTapStarts = cdef_tap_starts(
    &cdef_tap_offsets(CDEF_SEGMENT_STRIDE as isize, 1),
    2 * CDEF_SEGMENT_STRIDE + 2,
);

/// [`CDEF_TAP_STARTS`] for the interleaved chroma-pair layout: rows are
/// `CDEF_PAIR_STRIDE` lanes apart and a column displacement moves two lanes
/// because the two planes alternate.
const CDEF_PAIR_TAP_STARTS: CdefTapStarts = cdef_tap_starts(
    &cdef_tap_offsets(CDEF_PAIR_STRIDE as isize, 2),
    2 * CDEF_PAIR_STRIDE + 4,
);

const CDEF_PAIR_SEGMENT_TAP_STARTS: CdefTapStarts = cdef_tap_starts(
    &cdef_tap_offsets(CDEF_PAIR_SEGMENT_STRIDE as isize, 2),
    2 * CDEF_PAIR_SEGMENT_STRIDE + 4,
);

/// Two consecutive `W`-lane rows of one tap view as one `V`-lane vector.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_row_pair<const W: usize, const V: usize, const STRIDE: usize, const SPAN: usize>(
    view: &[u16; SPAN],
    row: usize,
) -> Option<Simd<i16, V>> {
    let first = view.get(row * STRIDE..)?.first_chunk::<W>()?;
    let second = view.get((row + 1) * STRIDE..)?.first_chunk::<W>()?;
    Some(Simd::from_array(cdef_pair::<W, V>(first, second)).cast())
}

/// Per-block § 7.18.3 `constrain` threshold and damping shift, splatted once.
#[derive(Clone, Copy)]
struct CdefConstrain<const V: usize> {
    threshold: Simd<u16, V>,
    shift: Simd<u16, V>,
}

impl<const V: usize> CdefConstrain<V> {
    fn new(threshold: i32, damping: i32) -> Self {
        Self {
            threshold: Simd::splat(threshold as u16),
            shift: Simd::splat(constrain_damping_adj(threshold, damping) as u16),
        }
    }

    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply(self, diff: Simd<i16, V>) -> Simd<i16, V> {
        let clip = self
            .threshold
            .saturating_sub(diff.abs().cast::<u16>() >> self.shift)
            .cast::<i16>();
        diff.simd_min(clip).simd_max(-clip)
    }
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_pair<const W: usize, const V: usize>(first: &[u16; W], second: &[u16; W]) -> [u16; V] {
    debug_assert_eq!(V, W * 2);
    core::array::from_fn(|i| if i < W { first[i] } else { second[i - W] })
}

/// A block's centre view and its four primary and eight secondary tap views.
type CdefTapViews<'a, const SPAN: usize> =
    (&'a [u16; SPAN], [&'a [u16; SPAN]; 4], [&'a [u16; SPAN]; 8]);

/// Binds the tap views of direction `dir` for [`cdef_filter_rows`]; an
/// inactive tap family gets the centre view.
///
/// Each of the twelve taps is bound once per block to a `SPAN`-lane view of
/// `pad` that starts at its displacement from the block's first sample. The
/// scratch leaves room for a view from any byte start, so the compiler knows
/// that the view is in bounds.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_tap_views<
    'a,
    const STRIDE: usize,
    const CENTER: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    pad: &'a [u16; AREA],
    dir: usize,
    starts: &CdefTapStarts,
) -> Option<CdefTapViews<'a, SPAN>> {
    const { assert!(u8::MAX as usize + SPAN <= AREA) };
    let starts = &starts[dir & 7];
    let view =
        |tap: usize| -> Option<&[u16; SPAN]> { pad.get(usize::from(starts[tap])..)?.first_chunk() };
    let center_view = pad.get(2 * STRIDE + CENTER..)?.first_chunk()?;
    let pri_views = if PRI {
        [view(0)?, view(1)?, view(2)?, view(3)?]
    } else {
        [center_view; 4]
    };
    let sec_views = if SEC {
        [
            view(4)?,
            view(5)?,
            view(6)?,
            view(7)?,
            view(8)?,
            view(9)?,
            view(10)?,
            view(11)?,
        ]
    } else {
        [center_view; 8]
    };
    Some((center_view, pri_views, sec_views))
}

/// § 7.18.3 over a padded block, two rows per `V`-lane vector.
///
/// The taps are read through [`cdef_tap_views`]. The row pairs are unrolled,
/// so every row load is a constant offset from a view. `ROWS` is the most
/// rows the layout holds; rows at or past `h` are computed and discarded.
/// With `HAS_UNAVAILABLE`, taps equal to [`CDEF_UNAVAILABLE`] leave the max
/// unchanged and constrain to zero.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows<
    const W: usize,
    const V: usize,
    const HAS_UNAVAILABLE: bool,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    pad: &[u16; AREA],
    h: usize,
    filter: &CdefBlockFilter,
    starts: &CdefTapStarts,
    out: &mut [u16],
    out_stride: usize,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| Simd::<i16, V>::splat(tap as i16));
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| Simd::<i16, V>::splat(tap as i16));
    let pri = CdefConstrain::<V>::new(filter.pri_str, filter.damping);
    let sec = CdefConstrain::<V>::new(filter.sec_str, filter.damping);
    macro_rules! filter_row_pair {
        ($row:literal) => {{
            let row: usize = $row;
            if row < ROWS && row < h {
                let center = cdef_row_pair::<W, V, STRIDE, SPAN>(center_view, row)?;
                let mut sum = Simd::<i16, V>::splat(0);
                let mut min = center;
                let mut max = center;
                macro_rules! add_pair {
                    ($first:expr, $second:expr, $constrain:expr, $weight:expr) => {{
                        let first = cdef_row_pair::<W, V, STRIDE, SPAN>($first, row)?;
                        let second = cdef_row_pair::<W, V, STRIDE, SPAN>($second, row)?;
                        if PRI && SEC {
                            min = min.simd_min(first).simd_min(second);
                            let unavailable = Simd::splat(CDEF_UNAVAILABLE as i16);
                            let (first_max, second_max) = if HAS_UNAVAILABLE {
                                (
                                    first.simd_eq(unavailable).select(center, first),
                                    second.simd_eq(unavailable).select(center, second),
                                )
                            } else {
                                (first, second)
                            };
                            max = max.simd_max(first_max).simd_max(second_max);
                        }
                        sum += $weight
                            * ($constrain.apply(first - center) + $constrain.apply(second - center));
                    }};
                }
                if PRI {
                    add_pair!(pri_views[0], pri_views[1], pri, pri_taps[0]);
                    add_pair!(pri_views[2], pri_views[3], pri, pri_taps[1]);
                }
                if SEC {
                    add_pair!(sec_views[0], sec_views[1], sec, sec_taps[0]);
                    add_pair!(sec_views[2], sec_views[3], sec, sec_taps[0]);
                    add_pair!(sec_views[4], sec_views[5], sec, sec_taps[1]);
                    add_pair!(sec_views[6], sec_views[7], sec, sec_taps[1]);
                }
                let negative = sum.is_negative().select(Simd::splat(1), Simd::splat(0));
                let mut filtered = center + ((Simd::splat(8) + sum - negative) >> 4);
                if PRI && SEC {
                    filtered = filtered.simd_max(min).simd_min(max);
                }
                let filtered = filtered.cast::<u16>().to_array();
                cdef_output_row::<W>(out, out_stride, row)?.copy_from_slice(&filtered[..W]); // splot-copy-ok: publish paired SIMD-filtered rows into output
                if row + 1 < h {
                    cdef_output_row::<W>(out, out_stride, row + 1)?.copy_from_slice(&filtered[W..]); // splot-copy-ok: publish paired SIMD-filtered rows into output
                }
            }
        }};
    }
    filter_row_pair!(0);
    filter_row_pair!(2);
    filter_row_pair!(4);
    filter_row_pair!(6);
    Some(())
}

/// Per-block § 7.18.3 `constrain` for 8-bit samples in `u8` lanes.
#[derive(Clone, Copy)]
struct CdefConstrain8 {
    threshold: Simd<u8, 16>,
    shift: Simd<u8, 16>,
}

impl CdefConstrain8 {
    fn new(threshold: i32, damping: i32) -> Self {
        Self {
            threshold: Simd::splat(threshold as u8),
            shift: Simd::splat(constrain_damping_adj(threshold, damping) as u8),
        }
    }

    /// `constrain(tap - center)` as `sign * min(|diff|, clip)`, which is the
    /// clamp to `[-clip, clip]` that [`CdefConstrain::apply`] computes.
    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply(self, tap: Simd<u8, 16>, center: Simd<u8, 16>) -> Simd<i8, 16> {
        let diff = tap.simd_max(center) - tap.simd_min(center);
        let clip = self.threshold.saturating_sub(diff >> self.shift);
        let magnitude = diff.simd_min(clip).cast::<i8>();
        tap.simd_lt(center).select(-magnitude, magnitude)
    }
}

/// Whether [`cdef_filter_rows_8bit`] computes `filter` exactly: 8-bit
/// samples, strengths below 64 so that a constrained tap pair fits in `i8`,
/// and damping shifts below 8 so that `u8` shifts do not wrap.
fn cdef_8bit_lanes_fit(filter: &CdefBlockFilter) -> bool {
    filter.coeff_shift == 0
        && [filter.pri_str, filter.sec_str]
            .into_iter()
            .all(|strength| {
                (0..64).contains(&strength) && constrain_damping_adj(strength, filter.damping) < 8
            })
}

/// [`cdef_filter_rows`] for 8-bit samples in an interior layout 8 lanes
/// wide. Each row pair narrows to one 16-lane `u8` vector, so a tap's
/// constrain is one vector operation per step instead of two. The weighted
/// sum widens to `i16`, and the result is the same as the `i16` kernel's.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows_8bit<
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    pad: &[u16; AREA],
    h: usize,
    filter: &CdefBlockFilter,
    starts: &CdefTapStarts,
    out: &mut [u16],
    out_stride: usize,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = (filter.pri_str & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| Simd::<i16, 16>::splat(tap as i16));
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| Simd::<i16, 16>::splat(tap as i16));
    let pri = CdefConstrain8::new(filter.pri_str, filter.damping);
    let sec = CdefConstrain8::new(filter.sec_str, filter.damping);
    let row_pair = |view: &[u16; SPAN], row: usize| -> Option<Simd<u8, 16>> {
        let first = Simd::<u16, 8>::from_array(*view.get(row * STRIDE..)?.first_chunk()?);
        let second = Simd::<u16, 8>::from_array(*view.get((row + 1) * STRIDE..)?.first_chunk()?);
        Some(simd_swizzle!(
            first.cast::<u8>(),
            second.cast::<u8>(),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        ))
    };
    macro_rules! filter_row_pair {
        ($row:literal) => {{
            let row: usize = $row;
            if row < ROWS && row < h {
                let center = row_pair(center_view, row)?;
                let mut sum = Simd::<i16, 16>::splat(0);
                let mut min = center;
                let mut max = center;
                macro_rules! add_pair {
                    ($first:expr, $second:expr, $constrain:expr, $weight:expr) => {{
                        let first = row_pair($first, row)?;
                        let second = row_pair($second, row)?;
                        if PRI && SEC {
                            min = min.simd_min(first).simd_min(second);
                            max = max.simd_max(first).simd_max(second);
                        }
                        let pair = $constrain.apply(first, center) + $constrain.apply(second, center);
                        sum += pair.cast::<i16>() * $weight;
                    }};
                }
                if PRI {
                    add_pair!(pri_views[0], pri_views[1], pri, pri_taps[0]);
                    add_pair!(pri_views[2], pri_views[3], pri, pri_taps[1]);
                }
                if SEC {
                    add_pair!(sec_views[0], sec_views[1], sec, sec_taps[0]);
                    add_pair!(sec_views[2], sec_views[3], sec, sec_taps[0]);
                    add_pair!(sec_views[4], sec_views[5], sec, sec_taps[1]);
                    add_pair!(sec_views[6], sec_views[7], sec, sec_taps[1]);
                }
                let negative = sum.is_negative().select(Simd::splat(1), Simd::splat(0));
                let mut filtered = center.cast::<i16>() + ((Simd::splat(8) + sum - negative) >> 4);
                if PRI && SEC {
                    filtered = filtered.simd_max(min.cast()).simd_min(max.cast());
                }
                let filtered = filtered.cast::<u16>().to_array();
                cdef_output_row::<8>(out, out_stride, row)?.copy_from_slice(&filtered[..8]); // splot-copy-ok: publish paired SIMD-filtered rows into output
                if row + 1 < h {
                    cdef_output_row::<8>(out, out_stride, row + 1)?.copy_from_slice(&filtered[8..]); // splot-copy-ok: publish paired SIMD-filtered rows into output
                }
            }
        }};
    }
    filter_row_pair!(0);
    filter_row_pair!(2);
    filter_row_pair!(4);
    filter_row_pair!(6);
    Some(())
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_output_row<const W: usize>(
    out: &mut [u16],
    stride: usize,
    row: usize,
) -> Option<&mut [u16; W]> {
    out.get_mut(row.checked_mul(stride)?..)?.first_chunk_mut()
}

/// Dispatches [`cdef_filter_rows`] on which tap families are active; with
/// neither, the block is its centre samples.
fn cdef_filter_block_rows<
    const W: usize,
    const V: usize,
    const HAS_UNAVAILABLE: bool,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
>(
    pad: &[u16; AREA],
    h: usize,
    filter: &CdefBlockFilter,
    starts: &CdefTapStarts,
    out: &mut [u16],
    out_stride: usize,
) -> Option<()> {
    if W == 8 && !HAS_UNAVAILABLE && cdef_8bit_lanes_fit(filter) {
        match (filter.pri_str != 0, filter.sec_str != 0) {
            (true, true) => {
                return cdef_filter_rows_8bit::<STRIDE, CENTER, ROWS, SPAN, AREA, true, true>(
                    pad, h, filter, starts, out, out_stride,
                );
            }
            (true, false) => {
                return cdef_filter_rows_8bit::<STRIDE, CENTER, ROWS, SPAN, AREA, true, false>(
                    pad, h, filter, starts, out, out_stride,
                );
            }
            (false, true) => {
                return cdef_filter_rows_8bit::<STRIDE, CENTER, ROWS, SPAN, AREA, false, true>(
                    pad, h, filter, starts, out, out_stride,
                );
            }
            (false, false) => {}
        }
    }
    match (filter.pri_str != 0, filter.sec_str != 0) {
        (true, true) => {
            cdef_filter_rows::<W, V, HAS_UNAVAILABLE, STRIDE, CENTER, ROWS, SPAN, AREA, true, true>(
                pad, h, filter, starts, out, out_stride,
            )
        }
        (true, false) => {
            cdef_filter_rows::<W, V, HAS_UNAVAILABLE, STRIDE, CENTER, ROWS, SPAN, AREA, true, false>(
                pad, h, filter, starts, out, out_stride,
            )
        }
        (false, true) => {
            cdef_filter_rows::<W, V, HAS_UNAVAILABLE, STRIDE, CENTER, ROWS, SPAN, AREA, false, true>(
                pad, h, filter, starts, out, out_stride,
            )
        }
        (false, false) => {
            for row in 0..h.min(ROWS) {
                let start = (2 + row) * STRIDE + CENTER;
                let center = pad.get(start..)?.first_chunk::<W>()?;
                cdef_output_row::<W>(out, out_stride, row)?.copy_from_slice(center); // splot-copy-ok: unfiltered block keeps its centre samples
            }
            Some(())
        }
    }
}

/// AV2 § 7.18.3 CDEF filter for one fully-interior block written to a strided output.
///
/// Returns `false` when the output geometry cannot hold the 8- or 4-sample-wide
/// CDEF block. The padded input layout matches [`cdef_filter_block_interior`].
pub fn cdef_filter_block_interior_to(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    let w = w.min(8);
    if out_stride < w {
        return false;
    }
    cdef_filter_block_interior_to_valid_stride(pad, w, h, filter, out, out_stride)
}

/// Variant of [`cdef_filter_block_interior_to`] for an already-validated output view.
#[doc(hidden)]
pub fn cdef_filter_block_interior_to_valid_stride(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    cdef_filter_block_padded_to_valid_stride::<false>(pad, w, h, filter, out, out_stride)
}

/// SIMD CDEF filtering for a padded boundary block.
///
/// Tap slots outside the active filter region must contain [`CDEF_UNAVAILABLE`].
/// Returns `false` for unsupported output geometry.
#[doc(hidden)]
pub fn cdef_filter_block_boundary_to_valid_stride(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    cdef_filter_block_padded_to_valid_stride::<true>(pad, w, h, filter, out, out_stride)
}

fn cdef_filter_block_padded_to_valid_stride<const HAS_UNAVAILABLE: bool>(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    let starts = &CDEF_TAP_STARTS;
    let h = h.min(8);
    match w.min(8) {
        8 => cdef_filter_block_rows::<
            8,
            16,
            HAS_UNAVAILABLE,
            CDEF_PADDED_SIDE,
            2,
            8,
            { 7 * CDEF_PADDED_SIDE + 8 },
            CDEF_PADDED_AREA,
        >(pad, h, filter, starts, out, out_stride),
        4 => cdef_filter_block_rows::<
            4,
            8,
            HAS_UNAVAILABLE,
            CDEF_PADDED_SIDE,
            2,
            8,
            { 7 * CDEF_PADDED_SIDE + 4 },
            CDEF_PADDED_AREA,
        >(pad, h, filter, starts, out, out_stride),
        _ => None,
    }
    .is_some()
}

/// Lanes per padded row of the interleaved chroma-pair scratch consumed by
/// [`cdef_filter_block_chroma_pair`]: four block columns plus the tap reach of
/// two on each side, two planes deep.
pub const CDEF_PAIR_STRIDE: usize = 16;

/// Sample count of one interleaved chroma-pair filter result: four rows of four
/// U and four V samples, U and V alternating.
pub const CDEF_PAIR_OUTPUT: usize = 32;

/// AV2 § 7.18.3 CDEF over one 4x4 chroma block of both chroma planes at once.
///
/// The two chroma planes of a block share every § 7.18.1 filter parameter, and
/// the § 7.18.3 kernel is per sample, so filtering them as one 16-lane vector is
/// the same arithmetic as two 8-lane vectors. `pad` holds the two planes'
/// neighbourhoods interleaved at `CDEF_PAIR_STRIDE` lanes per row, sample
/// `(col, row)` of plane `p` at `row * CDEF_PAIR_STRIDE + col * 2 + p`, so a
/// lane pair carries one position of both planes and every tap displacement is
/// the § 7.18.3 one with its column term doubled. `out` receives the filtered
/// `4 x 4` block in the same interleaved order.
///
/// Returns `false` when `h` exceeds the four rows the scratch covers.
pub fn cdef_filter_block_chroma_pair(
    pad: &[u16; CDEF_PADDED_AREA],
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    if h > 4 {
        return false;
    }
    cdef_filter_block_rows::<
        8,
        16,
        false,
        CDEF_PAIR_STRIDE,
        4,
        4,
        { 3 * CDEF_PAIR_STRIDE + 8 },
        CDEF_PADDED_AREA,
    >(pad, h, filter, &CDEF_PAIR_TAP_STARTS, out, 8)
    .is_some()
}

/// [`cdef_filter_block_chroma_pair`] for one block of the chroma-pair segment
/// scratch: rows `CDEF_PAIR_SEGMENT_STRIDE` lanes apart, with the block's
/// first tap-reach lane at index 0.
pub fn cdef_filter_block_chroma_pair_segment(
    pad: &[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA],
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    cdef_filter_block_rows::<
        8,
        16,
        false,
        CDEF_PAIR_SEGMENT_STRIDE,
        4,
        4,
        { 3 * CDEF_PAIR_SEGMENT_STRIDE + 8 },
        CDEF_PAIR_SEGMENT_BLOCK_AREA,
    >(pad, 4, filter, &CDEF_PAIR_SEGMENT_TAP_STARTS, out, 8)
    .is_some()
}

/// AV2 § 7.18.3 CDEF over one interior 8x8 luma block of the luma segment
/// scratch: rows `CDEF_SEGMENT_STRIDE` lanes apart, with the block's first
/// tap-reach lane at index 0. Returns `false` when `out` cannot hold the
/// block at `out_stride`.
pub fn cdef_filter_block_segment(
    pad: &[u16; CDEF_SEGMENT_BLOCK_AREA],
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    cdef_filter_block_rows::<
        8,
        16,
        false,
        CDEF_SEGMENT_STRIDE,
        2,
        8,
        { 7 * CDEF_SEGMENT_STRIDE + 8 },
        CDEF_SEGMENT_BLOCK_AREA,
    >(pad, 8, filter, &CDEF_SEGMENT_TAP_STARTS, out, out_stride)
    .is_some()
}

/// AV2 § 7.18.3 CDEF filter for one fully-interior block over a padded scratch.
///
/// `pad` holds the `CDEF_PADDED_SIDE x CDEF_PADDED_SIDE` row-major
/// neighbourhood in native `u16` sample storage, widened to `i32` per tap. Its
/// `(w x h)` output block starts at row 2, column 2; the
/// caller guarantees every tap position is inside the § 5.20.9.3 filter region
/// (`CdefAvailable` everywhere), which is what makes the per-tap availability
/// guard of [`cdef_filter_sample`] statically true. Bit-exact with calling
/// [`cdef_filter_sample`] per sample on all-available taps.
///
/// Filtered samples are written in native `u16` storage to `out[i * w + j]`
/// for `i in 0..h`, `j in 0..w`; `w` and `h` are clamped to 8. Every tap index
/// provably stays inside the scratch: the center index is at least
/// `2 * CDEF_PADDED_SIDE + 2` and the largest tap displacement is
/// `2 * CDEF_PADDED_SIDE + 2` in either
/// direction.
pub fn cdef_filter_block_interior(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16; 64],
) {
    let w = w.min(8);
    let h = h.min(8);

    let (pri_rel, sec_rel) = CDEF_RELATIVE_OFFSETS[filter.dir & 7];
    if cdef_filter_block_padded_to_valid_stride::<false>(pad, w, h, filter, out, w) {
        return;
    }

    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(filter.pri_str, filter.damping);
    let sec_adj = constrain_damping_adj(filter.sec_str, filter.damping);

    if filter.pri_str != 0 && filter.sec_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                let mut max = center;
                let mut min = center;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        let p = i32::from(
                            pad[center_index.wrapping_add_signed(pri_rel[k][sign_index])],
                        );
                        sum +=
                            pri_taps[k] * constrain_with_adj(p - center, filter.pri_str, pri_adj);
                        max = max.max(p);
                        min = min.min(p);
                        for dir_off_index in 0..2 {
                            let s = i32::from(
                                pad[center_index
                                    .wrapping_add_signed(sec_rel[k][sign_index][dir_off_index])],
                            );
                            sum += sec_taps[k]
                                * constrain_with_adj(s - center, filter.sec_str, sec_adj);
                            max = max.max(s);
                            min = min.min(s);
                        }
                    }
                }
                let rounded = center + ((8 + sum - i32::from(sum < 0)) >> 4);
                out[i * w + j] = rounded.clamp(min, max) as u16;
            }
        }
    // A lone tap family has weight 12, so rounding stays in range without clamping.
    } else if filter.pri_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        let p = i32::from(
                            pad[center_index.wrapping_add_signed(pri_rel[k][sign_index])],
                        );
                        sum +=
                            pri_taps[k] * constrain_with_adj(p - center, filter.pri_str, pri_adj);
                    }
                }
                out[i * w + j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else if filter.sec_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        for dir_off_index in 0..2 {
                            let s = i32::from(
                                pad[center_index
                                    .wrapping_add_signed(sec_rel[k][sign_index][dir_off_index])],
                            );
                            sum += sec_taps[k]
                                * constrain_with_adj(s - center, filter.sec_str, sec_adj);
                        }
                    }
                }
                out[i * w + j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else {
        for i in 0..h {
            for j in 0..w {
                out[i * w + j] = pad[(i + 2) * CDEF_PADDED_SIDE + (j + 2)];
            }
        }
    }
}

/// The `constrain` dampingAdj, which depends only on the per-call strengths
/// and is therefore derived once per sample instead of once per tap.
const fn constrain_damping_adj(threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let adj = damping - floor_log2(threshold as u32) as i32;
    if adj < 0 { 0 } else { adj }
}

const fn constrain_with_adj(diff: i32, threshold: i32, damping_adj: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let abs = diff.abs();
    let reduced = threshold - (abs >> damping_adj);
    let magnitude = if reduced < 0 {
        0
    } else if reduced > abs {
        abs
    } else {
        reduced
    };
    if diff < 0 { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_block_has_zero_variance() {
        let block = [[0i32; 8]; 8];
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!((y_dir, var), (0, 0), "flat block: yDir 0, var 0");
    }

    #[test]
    fn horizontal_gradient_picks_a_horizontal_direction() {
        let mut block = [[0i32; 8]; 8];
        for (i, row) in block.iter_mut().enumerate() {
            for cell in row.iter_mut() {
                *cell = i as i32 * 16 - 56; // distinct per-row value, zero mean-ish
            }
        }
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!(y_dir, 2, "row-varying block selects direction 2");
        assert!(var > 0, "a non-flat block has positive variance: var={var}");
    }

    /// § 7.18.2 as written, one sample at a time.
    #[allow(clippy::needless_range_loop)]
    fn spec_direction(block: &[[i32; 8]; 8]) -> (usize, i32) {
        let mut partial = [[0i32; 15]; 8];
        for i in 0..8 {
            for j in 0..8 {
                let x = block[i][j];
                partial[0][i + j] += x;
                partial[1][i + j / 2] += x;
                partial[2][i] += x;
                partial[3][3 + i - j / 2] += x;
                partial[4][7 + i - j] += x;
                partial[5][3 - i / 2 + j] += x;
                partial[6][j] += x;
                partial[7][i / 2 + j] += x;
            }
        }
        let square = |v: i32| v * v;
        let mut cost = [0i32; 8];
        for i in 0..8 {
            cost[2] += square(partial[2][i]);
            cost[6] += square(partial[6][i]);
        }
        cost[2] *= DIV_TABLE[8];
        cost[6] *= DIV_TABLE[8];
        for i in 0..7 {
            for d in [0, 4] {
                cost[d] += (square(partial[d][i]) + square(partial[d][14 - i])) * DIV_TABLE[i + 1];
            }
        }
        cost[0] += square(partial[0][7]) * DIV_TABLE[8];
        cost[4] += square(partial[4][7]) * DIV_TABLE[8];
        for i in (1..8).step_by(2) {
            for j in 0..5 {
                cost[i] += square(partial[i][3 + j]);
            }
            cost[i] *= DIV_TABLE[8];
            for j in 0..3 {
                cost[i] +=
                    (square(partial[i][j]) + square(partial[i][10 - j])) * DIV_TABLE[2 * j + 2];
            }
        }
        let (mut best_cost, mut y_dir) = (0, 0);
        for (i, &c) in cost.iter().enumerate() {
            if c > best_cost {
                best_cost = c;
                y_dir = i;
            }
        }
        (y_dir, (best_cost - cost[(y_dir + 4) & 7]) >> 10)
    }

    #[test]
    fn direction_matches_spec_partials_and_costs() {
        let mut state = 0x2545_f491u32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        let mut directions = [0usize; 8];
        for case in 0..4000usize {
            let mut block = [[0i32; 8]; 8];
            let slope = (case % 9) as i32 - 4;
            for (i, row) in block.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    let ramp =
                        slope * (i as i32 * (case % 3) as i32 + j as i32 * (case % 5) as i32);
                    let noise = (next() % 64) as i32 - 32;
                    *cell = (ramp * 6 + noise).clamp(-128, 127);
                }
            }
            if case % 7 == 0 {
                block = [[if case % 2 == 0 { -128 } else { 127 }; 8]; 8];
            }
            let expected = spec_direction(&block);
            directions[expected.0] += 1;
            assert_eq!(cdef_direction(&block), expected, "case {case}");
        }
        assert!(directions.iter().all(|&count| count > 0), "{directions:?}");
    }

    #[test]
    fn padded_direction_matches_materialized_block() {
        for coeff_shift in [0u32, 2, 4] {
            let mut state = 0x9e37_79b9u32 ^ coeff_shift;
            let mut pad = [0u16; CDEF_PADDED_AREA];
            let mut block = [[0i32; 8]; 8];
            for (i, row) in block.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let sample = ((state >> 16) & ((256 << coeff_shift) - 1)) as u16;
                    pad[(i + 2) * CDEF_PADDED_SIDE + j + 2] = sample;
                    *cell = (i32::from(sample) >> coeff_shift) - 128;
                }
            }
            assert_eq!(
                cdef_direction_padded(&pad, coeff_shift),
                cdef_direction(&block),
                "coeff_shift={coeff_shift}"
            );
        }
    }

    #[test]
    fn constrain_with_adj_matches_cdef_constrain() {
        for threshold in [0, 1, 3, 8, 63, 256] {
            for damping in [3, 5, 8] {
                let adj = constrain_damping_adj(threshold, damping);
                for diff in -300..=300 {
                    assert_eq!(
                        cdef_constrain(diff, threshold, damping),
                        constrain_with_adj(diff, threshold, adj),
                        "threshold={threshold} damping={damping} diff={diff}"
                    );
                }
            }
        }
    }

    #[test]
    fn constrain_matches_spec_branches() {
        assert_eq!(cdef_constrain(50, 0, 4), 0, "zero threshold returns 0");
        assert_eq!(cdef_constrain(50, 4, 4), 0, "large diff clamps to 0");
        assert_eq!(
            cdef_constrain(3, 4, 4),
            3,
            "small positive diff passes through"
        );
        assert_eq!(cdef_constrain(-3, 4, 4), -3, "small negative diff negated");
        assert_eq!(
            cdef_constrain(6, 4, 4),
            3,
            "mid diff clamps to threshold-rampdown"
        );
    }

    #[test]
    fn filter_with_all_unavailable_taps_is_identity() {
        let unavail = CdefTap {
            value: 0,
            available: false,
        };
        let taps = CdefSampleTaps {
            center: 130,
            primary: [[unavail; 2]; 2],
            secondary: [[[unavail; 2]; 2]; 2],
        };
        assert_eq!(
            cdef_filter_sample(&taps, 8, 8, 4, 0),
            130,
            "no taps -> identity"
        );
    }

    #[test]
    fn filter_pulls_center_toward_a_brighter_primary_neighbour() {
        let avail = |v| CdefTap {
            value: v,
            available: true,
        };
        let unavail = CdefTap {
            value: 0,
            available: false,
        };
        let taps = CdefSampleTaps {
            center: 100,
            primary: [[unavail, avail(108)], [unavail, unavail]],
            secondary: [[[unavail; 2]; 2]; 2],
        };
        assert_eq!(
            cdef_filter_sample(&taps, 8, 8, 4, 0),
            101,
            "pulled +1 toward neighbour"
        );
    }

    fn per_sample_reference(
        pad: &[u16; CDEF_PADDED_AREA],
        i: usize,
        j: usize,
        filter: &CdefBlockFilter,
    ) -> i32 {
        let at = |dy: isize, dx: isize| -> CdefTap {
            let row = (i + 2).wrapping_add_signed(dy);
            let col = (j + 2).wrapping_add_signed(dx);
            let value = pad[row * CDEF_PADDED_SIDE + col];
            CdefTap {
                value: i32::from(value),
                available: value != CDEF_UNAVAILABLE,
            }
        };
        let fetch = |dir: usize, k: usize, sign: isize| -> CdefTap {
            at(
                sign * CDEF_DIRECTIONS[dir & 7][k][0] as isize,
                sign * CDEF_DIRECTIONS[dir & 7][k][1] as isize,
            )
        };
        let mut taps = CdefSampleTaps {
            center: i32::from(pad[(i + 2) * CDEF_PADDED_SIDE + (j + 2)]),
            primary: [[CdefTap {
                value: 0,
                available: false,
            }; 2]; 2],
            secondary: [[[CdefTap {
                value: 0,
                available: false,
            }; 2]; 2]; 2],
        };
        for k in 0..2 {
            for (sign_index, sign) in [-1isize, 1].into_iter().enumerate() {
                taps.primary[k][sign_index] = fetch(filter.dir, k, sign);
                for (dir_off_index, dir_off) in [6usize, 2].into_iter().enumerate() {
                    taps.secondary[k][sign_index][dir_off_index] =
                        fetch(filter.dir + dir_off, k, sign);
                }
            }
        }
        cdef_filter_sample(
            &taps,
            filter.pri_str,
            filter.sec_str,
            filter.damping,
            filter.coeff_shift,
        )
    }

    #[test]
    fn block_interior_kernel_matches_per_sample_filter() {
        for (coeff_shift, max_sample) in [(0u32, 255u32), (2, 1023), (4, 4095)] {
            let mut state = 0x1234_5678u32 ^ (coeff_shift * 77);
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for cell in &mut pad {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *cell = ((state >> 16) % (max_sample + 1)) as u16;
            }
            for dir in 0..8usize {
                for (pri, sec) in [(0, 0), (1, 0), (0, 2), (4, 2), (7, 3), (63, 63)] {
                    for damping_base in [3i32, 4, 6] {
                        let filter = CdefBlockFilter {
                            pri_str: pri << coeff_shift,
                            sec_str: sec << coeff_shift,
                            damping: damping_base + coeff_shift as i32,
                            dir,
                            coeff_shift,
                        };
                        for (w, h) in [(8usize, 8usize), (4, 8), (4, 4), (5, 3)] {
                            let mut out = [0u16; 64];
                            cdef_filter_block_interior(&pad, w, h, &filter, &mut out);
                            if w == 4 || w == 8 {
                                let mut strided = [u16::MAX; 104];
                                assert!(cdef_filter_block_interior_to(
                                    &pad,
                                    w,
                                    h,
                                    &filter,
                                    &mut strided,
                                    13,
                                ));
                                for i in 0..h {
                                    assert_eq!(
                                        &strided[i * 13..i * 13 + w],
                                        &out[i * w..i * w + w]
                                    );
                                }
                            }
                            for i in 0..h {
                                for j in 0..w {
                                    assert_eq!(
                                        i32::from(out[i * w + j]),
                                        per_sample_reference(&pad, i, j, &filter),
                                        "shift={coeff_shift} dir={dir} pri={pri} sec={sec} \
                                         damping={damping_base} w={w} h={h} i={i} j={j}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        let pad = [128u16; CDEF_PADDED_AREA];
        let filter = CdefBlockFilter {
            pri_str: 4,
            sec_str: 2,
            damping: 3,
            dir: 0,
            coeff_shift: 0,
        };
        let mut out = [u16::MAX; 64];
        assert!(!cdef_filter_block_interior_to(
            &pad, 8, 8, &filter, &mut out, 7,
        ));
        assert!(out.iter().all(|&sample| sample == u16::MAX));
    }

    #[test]
    fn block_boundary_kernel_matches_unavailable_taps() {
        let mut pad = [0u16; CDEF_PADDED_AREA];
        for (index, sample) in pad.iter_mut().enumerate() {
            *sample = ((index * 73 + index / CDEF_PADDED_SIDE * 211) % 1024) as u16;
        }
        for row in 0..CDEF_PADDED_SIDE {
            for col in 0..CDEF_PADDED_SIDE {
                if row < 2 || col < 2 {
                    pad[row * CDEF_PADDED_SIDE + col] = CDEF_UNAVAILABLE;
                }
            }
        }
        for dir in 0..8 {
            for (pri_str, sec_str) in [(8, 0), (0, 4), (8, 4)] {
                let filter = CdefBlockFilter {
                    pri_str,
                    sec_str,
                    damping: 4,
                    dir,
                    coeff_shift: 2,
                };
                let mut output = [u16::MAX; 88];
                assert!(cdef_filter_block_boundary_to_valid_stride(
                    &pad,
                    8,
                    8,
                    &filter,
                    &mut output,
                    11,
                ));
                for row in 0..8 {
                    for col in 0..8 {
                        assert_eq!(
                            i32::from(output[row * 11 + col]),
                            per_sample_reference(&pad, row, col, &filter),
                            "dir={dir} pri={pri_str} sec={sec_str} row={row} col={col}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chroma_pair_matches_two_single_plane_blocks() {
        for coeff_shift in [2u32, 0] {
            chroma_pair_matches_single_planes(coeff_shift);
        }
    }

    fn chroma_pair_matches_single_planes(coeff_shift: u32) {
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 13) & ((256 << coeff_shift) - 1)) as u16
        };
        for dir in 0..8 {
            for (pri_str, sec_str) in [(0, 0), (12, 0), (0, 8), (12, 8), (16, 4)] {
                let mut pair = [0u16; CDEF_PADDED_AREA];
                let mut planes = [[0u16; CDEF_PADDED_AREA]; 2];
                for row in 0..8 {
                    for col in 0..8 {
                        for (plane, single) in planes.iter_mut().enumerate() {
                            let sample = next();
                            pair[row * CDEF_PAIR_STRIDE + col * 2 + plane] = sample;
                            single[row * CDEF_PADDED_SIDE + col] = sample;
                        }
                    }
                }
                let filter = CdefBlockFilter {
                    pri_str,
                    sec_str,
                    damping: 5,
                    dir,
                    coeff_shift,
                };
                let mut paired = [0u16; CDEF_PAIR_OUTPUT];
                assert!(cdef_filter_block_chroma_pair(
                    &pair,
                    4,
                    &filter,
                    &mut paired
                ));
                for (plane, single) in planes.iter().enumerate() {
                    let mut expected = [0u16; 64];
                    assert!(cdef_filter_block_interior_to_valid_stride(
                        single,
                        4,
                        4,
                        &filter,
                        &mut expected,
                        4,
                    ));
                    for row in 0..4 {
                        for col in 0..4 {
                            assert_eq!(
                                paired[row * 8 + col * 2 + plane],
                                expected[row * 4 + col],
                                "dir={dir} pri={pri_str} sec={sec_str} plane={plane} \
                                 row={row} col={col}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn tap_starts_leave_room_for_their_views() {
        for (starts, span, window) in [
            (
                CDEF_TAP_STARTS,
                7 * CDEF_PADDED_SIDE + 8,
                12 * CDEF_PADDED_SIDE,
            ),
            (
                CDEF_PAIR_TAP_STARTS,
                3 * CDEF_PAIR_STRIDE + 8,
                8 * CDEF_PAIR_STRIDE,
            ),
            (
                CDEF_SEGMENT_TAP_STARTS,
                7 * CDEF_SEGMENT_STRIDE + 8,
                11 * CDEF_SEGMENT_STRIDE + 12,
            ),
            (
                CDEF_PAIR_SEGMENT_TAP_STARTS,
                3 * CDEF_PAIR_SEGMENT_STRIDE + 8,
                7 * CDEF_PAIR_SEGMENT_STRIDE + 16,
            ),
        ] {
            assert!(
                starts
                    .iter()
                    .flatten()
                    .all(|&start| usize::from(start) + span <= window)
            );
        }
    }

    #[test]
    fn segment_blocks_match_single_block_kernels() {
        for coeff_shift in [2u32, 0] {
            let mut state = 0x9e37_79b9u32;
            let mut next = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 13) & ((256 << coeff_shift) - 1)) as u16
            };
            let mut luma = [0u16; 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_SEGMENT_BLOCK_AREA];
            let mut pair = [0u16; 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_PAIR_SEGMENT_BLOCK_AREA];
            luma[..12 * CDEF_SEGMENT_STRIDE].fill_with(&mut next);
            pair[..8 * CDEF_PAIR_SEGMENT_STRIDE].fill_with(&mut next);
            for block in 0..CDEF_SEGMENT_BLOCKS {
                let luma_block: &[u16; CDEF_SEGMENT_BLOCK_AREA] =
                    &core::array::from_fn(|lane| luma[8 * block + lane]);
                let pair_block: &[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA] =
                    &core::array::from_fn(|lane| pair[8 * block + lane]);
                let mut single = [0u16; CDEF_PADDED_AREA];
                let mut single_pair = [0u16; CDEF_PADDED_AREA];
                for row in 0..12 {
                    for col in 0..12 {
                        single[row * CDEF_PADDED_SIDE + col] =
                            luma_block[row * CDEF_SEGMENT_STRIDE + col];
                    }
                }
                for row in 0..8 {
                    for lane in 0..16 {
                        single_pair[row * CDEF_PAIR_STRIDE + lane] =
                            pair_block[row * CDEF_PAIR_SEGMENT_STRIDE + lane];
                    }
                }
                assert_eq!(
                    cdef_direction_segment(luma_block, coeff_shift),
                    cdef_direction_padded(&single, coeff_shift)
                );
                for dir in 0..8 {
                    for (pri_str, sec_str) in [(0, 0), (12, 0), (0, 8), (12, 8), (16, 4), (5, 3)] {
                        let filter = CdefBlockFilter {
                            pri_str: pri_str << coeff_shift,
                            sec_str: sec_str << coeff_shift,
                            damping: 5 + coeff_shift as i32,
                            dir,
                            coeff_shift,
                        };
                        let (mut got, mut want) = ([0u16; 80], [0u16; 80]);
                        assert!(cdef_filter_block_segment(luma_block, &filter, &mut got, 10));
                        assert!(cdef_filter_block_interior_to_valid_stride(
                            &single, 8, 8, &filter, &mut want, 10
                        ));
                        assert_eq!(got, want, "luma block={block} dir={dir} pri={pri_str}");
                        let (mut got, mut want) =
                            ([0u16; CDEF_PAIR_OUTPUT], [0u16; CDEF_PAIR_OUTPUT]);
                        assert!(cdef_filter_block_chroma_pair_segment(
                            pair_block, &filter, &mut got
                        ));
                        assert!(cdef_filter_block_chroma_pair(
                            &single_pair,
                            4,
                            &filter,
                            &mut want
                        ));
                        assert_eq!(got, want, "pair block={block} dir={dir} pri={pri_str}");
                    }
                }
            }
        }
    }

    #[test]
    fn uv_dir_table_is_the_spec_remap() {
        // §7.18.1 Cdef_Uv_Dir: spot-check the 4:2:0 (subX=1, subY=1) row.
        assert_eq!(
            CDEF_UV_DIR[1][1],
            [0, 1, 2, 3, 4, 5, 6, 7],
            "4:2:0 is identity"
        );
        assert_eq!(
            CDEF_UV_DIR[0][0],
            [0, 1, 2, 3, 4, 5, 6, 7],
            "4:4:4 is identity"
        );
        assert_eq!(CDEF_UV_DIR[0][1][1], 2, "4:2:2 row remaps yDir 1 -> 2");
    }

    #[test]
    fn directions_table_matches_spec() {
        assert_eq!(CDEF_DIRECTIONS[0], [[-1, 1], [-2, 2]]);
        assert_eq!(CDEF_DIRECTIONS[7], [[1, 0], [2, -1]]);
    }
}
