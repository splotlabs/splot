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
    Select, Simd, cmp::SimdOrd, cmp::SimdPartialEq, num::SimdInt, num::SimdUint, simd_swizzle,
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
#[allow(clippy::needless_range_loop)]
pub fn cdef_direction(block: &[[i32; 8]; 8]) -> (usize, i32) {
    let mut partial_hv = [[0i16; 8]; 2];
    let mut partial_diag = [[0i16; 15]; 2];
    let mut partial_alt = [[0i16; 11]; 4];
    let mut vertical = Simd::<i16, 8>::splat(0);
    for i in 0..8 {
        debug_assert!(
            block[i]
                .iter()
                .all(|&sample| (-128..=127).contains(&sample))
        );
        let row = Simd::from_array(block[i]).cast::<i16>();
        partial_hv[0][i] = row.reduce_sum();
        vertical += row;
        accumulate_cdef_direction_row(i, row, &mut partial_diag, &mut partial_alt);
    }
    partial_hv[1] = vertical.to_array();
    finish_cdef_direction(&partial_hv, &partial_diag, &partial_alt)
}

/// AV2 § 7.18.2 CDEF direction process over the interior padded block layout.
///
/// The 8x8 luma block begins at row 2, column 2 of `pad`; `coeff_shift` is
/// `BitDepth - 8`. The result matches [`cdef_direction`] without materializing
/// the intermediate shifted 8x8 array.
pub fn cdef_direction_padded(pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32) -> (usize, i32) {
    let mut partial_hv = [[0i16; 8]; 2];
    let mut vertical = Simd::<i16, 8>::splat(0);
    let mut diag_low = [Simd::<i16, 8>::splat(0); 2];
    let mut diag_high = [Simd::<i16, 8>::splat(0); 2];
    let mut alt_low = [Simd::<i16, 8>::splat(0); 4];
    let mut alt_high = [Simd::<i16, 8>::splat(0); 4];
    macro_rules! accumulate_row {
        ($row:literal, $alt2:literal, $alt3:literal) => {{
            let start = ($row + 2) * CDEF_PADDED_SIDE + 2;
            let samples = (Simd::<u16, 8>::from_slice(&pad[start..]) >> coeff_shift as u16)
                .cast::<i16>()
                - Simd::splat(128);
            partial_hv[0][$row] = samples.reduce_sum();
            vertical += samples;
            accumulate_cdef_diagonal::<$row>(samples, &mut diag_low[0], &mut diag_high[0]);
            let reversed = simd_swizzle!(samples, [7, 6, 5, 4, 3, 2, 1, 0]);
            accumulate_cdef_diagonal::<$row>(reversed, &mut diag_low[1], &mut diag_high[1]);
            let pairs = simd_swizzle!(samples, [0, 2, 4, 6]) + simd_swizzle!(samples, [1, 3, 5, 7]);
            let pairs = Simd::from_array([pairs[0], pairs[1], pairs[2], pairs[3], 0, 0, 0, 0]);
            accumulate_cdef_diagonal::<$row>(pairs, &mut alt_low[0], &mut alt_high[0]);
            let reversed_pairs = simd_swizzle!(pairs, [3, 2, 1, 0, 4, 5, 6, 7]);
            accumulate_cdef_diagonal::<$row>(reversed_pairs, &mut alt_low[1], &mut alt_high[1]);
            accumulate_cdef_diagonal::<$alt2>(samples, &mut alt_low[2], &mut alt_high[2]);
            accumulate_cdef_diagonal::<$alt3>(samples, &mut alt_low[3], &mut alt_high[3]);
        }};
    }
    accumulate_row!(0, 3, 0);
    accumulate_row!(1, 3, 0);
    accumulate_row!(2, 2, 1);
    accumulate_row!(3, 2, 1);
    accumulate_row!(4, 1, 2);
    accumulate_row!(5, 1, 2);
    accumulate_row!(6, 0, 3);
    accumulate_row!(7, 0, 3);
    partial_hv[1] = vertical.to_array();
    let partial_diag = [
        combine_cdef_diagonal(diag_low[0], diag_high[0]),
        combine_cdef_diagonal(diag_low[1], diag_high[1]),
    ];
    let partial_alt = core::array::from_fn(|i| combine_cdef_alt(alt_low[i], alt_high[i]));
    finish_cdef_direction(&partial_hv, &partial_diag, &partial_alt)
}

#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn accumulate_cdef_diagonal<const ROW: usize>(
    samples: Simd<i16, 8>,
    low: &mut Simd<i16, 8>,
    high: &mut Simd<i16, 8>,
) {
    let zero = Simd::splat(0);
    let (low_add, high_add) = match ROW {
        0 => (samples, zero),
        1 => (
            simd_swizzle!(zero, samples, [0, 8, 9, 10, 11, 12, 13, 14]),
            simd_swizzle!(zero, samples, [15, 0, 1, 2, 3, 4, 5, 6]),
        ),
        2 => (
            simd_swizzle!(zero, samples, [0, 1, 8, 9, 10, 11, 12, 13]),
            simd_swizzle!(zero, samples, [14, 15, 0, 1, 2, 3, 4, 5]),
        ),
        3 => (
            simd_swizzle!(zero, samples, [0, 1, 2, 8, 9, 10, 11, 12]),
            simd_swizzle!(zero, samples, [13, 14, 15, 0, 1, 2, 3, 4]),
        ),
        4 => (
            simd_swizzle!(zero, samples, [0, 1, 2, 3, 8, 9, 10, 11]),
            simd_swizzle!(zero, samples, [12, 13, 14, 15, 0, 1, 2, 3]),
        ),
        5 => (
            simd_swizzle!(zero, samples, [0, 1, 2, 3, 4, 8, 9, 10]),
            simd_swizzle!(zero, samples, [11, 12, 13, 14, 15, 0, 1, 2]),
        ),
        6 => (
            simd_swizzle!(zero, samples, [0, 1, 2, 3, 4, 5, 8, 9]),
            simd_swizzle!(zero, samples, [10, 11, 12, 13, 14, 15, 0, 1]),
        ),
        _ => (
            simd_swizzle!(zero, samples, [0, 1, 2, 3, 4, 5, 6, 8]),
            simd_swizzle!(zero, samples, [9, 10, 11, 12, 13, 14, 15, 0]),
        ),
    };
    *low += low_add;
    *high += high_add;
}

fn combine_cdef_diagonal(low: Simd<i16, 8>, high: Simd<i16, 8>) -> [i16; 15] {
    let low = low.to_array();
    let high = high.to_array();
    [
        low[0], low[1], low[2], low[3], low[4], low[5], low[6], low[7], high[0], high[1], high[2],
        high[3], high[4], high[5], high[6],
    ]
}

fn combine_cdef_alt(low: Simd<i16, 8>, high: Simd<i16, 8>) -> [i16; 11] {
    let low = low.to_array();
    let high = high.to_array();
    [
        low[0], low[1], low[2], low[3], low[4], low[5], low[6], low[7], high[0], high[1], high[2],
    ]
}

#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn accumulate_cdef_direction_row(
    row: usize,
    samples: Simd<i16, 8>,
    partial_diag: &mut [[i16; 15]; 2],
    partial_alt: &mut [[i16; 11]; 4],
) {
    let reversed = simd_swizzle!(samples, [7, 6, 5, 4, 3, 2, 1, 0]);
    let add8 = |target: &mut [i16], values: Simd<i16, 8>| {
        let sum = Simd::from_slice(target) + values;
        target[..8].copy_from_slice(&sum.to_array()); // splot-copy-ok: publish SIMD sums into direction scratch
    };
    add8(&mut partial_diag[0][row..], samples);
    add8(&mut partial_diag[1][row..], reversed);
    accumulate_cdef_alt_row(row, samples, partial_alt);
}

#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn accumulate_cdef_alt_row(row: usize, samples: Simd<i16, 8>, partial_alt: &mut [[i16; 11]; 4]) {
    let pair_sums = simd_swizzle!(samples, [0, 2, 4, 6]) + simd_swizzle!(samples, [1, 3, 5, 7]);
    let reversed_pairs = simd_swizzle!(pair_sums, [3, 2, 1, 0]);
    let add8 = |target: &mut [i16], values: Simd<i16, 8>| {
        let sum = Simd::from_slice(target) + values;
        target[..8].copy_from_slice(&sum.to_array()); // splot-copy-ok: publish SIMD sums into direction scratch
    };
    let add4 = |target: &mut [i16], values: Simd<i16, 4>| {
        let sum = Simd::from_slice(target) + values;
        target[..4].copy_from_slice(&sum.to_array()); // splot-copy-ok: publish SIMD sums into direction scratch
    };
    add4(&mut partial_alt[0][row..], pair_sums);
    add4(&mut partial_alt[1][row..], reversed_pairs);
    add8(&mut partial_alt[2][3 - row / 2..], samples);
    add8(&mut partial_alt[3][row / 2..], samples);
}

fn finish_cdef_direction(
    partial_hv: &[[i16; 8]; 2],
    partial_diag: &[[i16; 15]; 2],
    partial_alt: &[[i16; 11]; 4],
) -> (usize, i32) {
    let mut cost = [0i32; 8];
    let horizontal = Simd::from_array(partial_hv[0]).cast::<i32>();
    let vertical = Simd::from_array(partial_hv[1]).cast::<i32>();
    cost[2] = (horizontal * horizontal).reduce_sum() * DIV_TABLE[8];
    cost[6] = (vertical * vertical).reduce_sum() * DIV_TABLE[8];
    for (dir, partial) in [(0, &partial_diag[0]), (4, &partial_diag[1])] {
        let low = Simd::from_array([
            partial[0], partial[1], partial[2], partial[3], partial[4], partial[5], partial[6],
            partial[7],
        ])
        .cast::<i32>();
        let high = Simd::from_array([
            partial[14],
            partial[13],
            partial[12],
            partial[11],
            partial[10],
            partial[9],
            partial[8],
            0,
        ])
        .cast::<i32>();
        let weights = Simd::from_array([
            DIV_TABLE[1],
            DIV_TABLE[2],
            DIV_TABLE[3],
            DIV_TABLE[4],
            DIV_TABLE[5],
            DIV_TABLE[6],
            DIV_TABLE[7],
            DIV_TABLE[8],
        ]);
        cost[dir] = ((low * low + high * high) * weights).reduce_sum();
    }
    for (n, partial) in partial_alt.iter().enumerate() {
        let i = n * 2 + 1;
        let low = Simd::from_array([
            partial[3], partial[4], partial[5], partial[6], partial[7], partial[0], partial[1],
            partial[2],
        ])
        .cast::<i32>();
        let high =
            Simd::from_array([0, 0, 0, 0, 0, partial[10], partial[9], partial[8]]).cast::<i32>();
        let weights = Simd::from_array([
            DIV_TABLE[8],
            DIV_TABLE[8],
            DIV_TABLE[8],
            DIV_TABLE[8],
            DIV_TABLE[8],
            DIV_TABLE[2],
            DIV_TABLE[4],
            DIV_TABLE[6],
        ]);
        cost[i] = ((low * low + high * high) * weights).reduce_sum();
    }

    let mut best_cost = 0i32;
    let mut y_dir = 0usize;
    for (dir, &c) in cost.iter().enumerate() {
        if c > best_cost {
            best_cost = c;
            y_dir = dir;
        }
    }
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

/// Sample count of the padded per-block scratch: `CDEF_PADDED_SIDE` squared.
pub const CDEF_PADDED_AREA: usize = CDEF_PADDED_SIDE * CDEF_PADDED_SIDE;

/// Padded-tap marker used by the SIMD boundary kernel for unavailable samples.
pub const CDEF_UNAVAILABLE: u16 = i16::MAX as u16;

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

type CdefPrimaryStarts = [[usize; 2]; 2];
type CdefSecondaryStarts = [[[usize; 2]; 2]; 2];
type CdefPrimaryOffsets = [[isize; 2]; 2];
type CdefSecondaryOffsets = [[[isize; 2]; 2]; 2];
type CdefRowStarts = [(CdefPrimaryStarts, CdefSecondaryStarts); 8];

const fn cdef_relative_offset(direction: usize, tap: usize, sign: i32) -> isize {
    let [dy, dx] = CDEF_DIRECTIONS[direction & 7][tap];
    (sign * dy) as isize * CDEF_PADDED_SIDE as isize + (sign * dx) as isize
}

const CDEF_RELATIVE_OFFSETS: [(CdefPrimaryOffsets, CdefSecondaryOffsets); 8] = {
    let mut offsets = [([[0; 2]; 2], [[[0; 2]; 2]; 2]); 8];
    let mut dir = 0;
    while dir < 8 {
        let mut tap = 0;
        while tap < 2 {
            let mut sign_index = 0;
            while sign_index < 2 {
                let sign = if sign_index == 0 { -1 } else { 1 };
                offsets[dir].0[tap][sign_index] = cdef_relative_offset(dir, tap, sign);
                offsets[dir].1[tap][sign_index][0] = cdef_relative_offset(dir + 6, tap, sign);
                offsets[dir].1[tap][sign_index][1] = cdef_relative_offset(dir + 2, tap, sign);
                sign_index += 1;
            }
            tap += 1;
        }
        dir += 1;
    }
    offsets
};

const CDEF_ROW_STARTS: [CdefRowStarts; 8] = {
    let mut starts = [[([[0; 2]; 2], [[[0; 2]; 2]; 2]); 8]; 8];
    let mut dir = 0;
    while dir < 8 {
        let mut row = 0;
        while row < 8 {
            let center = (row + 2) * CDEF_PADDED_SIDE + 2;
            let mut tap = 0;
            while tap < 2 {
                let mut sign = 0;
                while sign < 2 {
                    starts[dir][row].0[tap][sign] =
                        (center as isize + CDEF_RELATIVE_OFFSETS[dir].0[tap][sign]) as usize;
                    let mut secondary = 0;
                    while secondary < 2 {
                        starts[dir][row].1[tap][sign][secondary] = (center as isize
                            + CDEF_RELATIVE_OFFSETS[dir].1[tap][sign][secondary])
                            as usize;
                        secondary += 1;
                    }
                    sign += 1;
                }
                tap += 1;
            }
            row += 1;
        }
        dir += 1;
    }
    starts
};

/// [`CDEF_ROW_STARTS`] for the interleaved chroma-pair layout: rows are
/// `CDEF_PAIR_STRIDE` lanes apart, the block starts at lane 4, and a column
/// displacement moves two lanes because the two planes alternate.
const CDEF_PAIR_ROW_STARTS: [CdefRowStarts; 8] = {
    let mut starts = [[([[0; 2]; 2], [[[0; 2]; 2]; 2]); 8]; 8];
    let mut dir = 0;
    while dir < 8 {
        let mut row = 0;
        while row < 8 {
            let center = (row + 2) * CDEF_PAIR_STRIDE + 4;
            let mut tap = 0;
            while tap < 2 {
                let mut sign = 0;
                while sign < 2 {
                    let signed = if sign == 0 { -1 } else { 1 };
                    starts[dir][row].0[tap][sign] =
                        (center as isize + cdef_pair_offset(dir, tap, signed)) as usize;
                    let mut secondary = 0;
                    while secondary < 2 {
                        let rotation = if secondary == 0 { 6 } else { 2 };
                        starts[dir][row].1[tap][sign][secondary] = (center as isize
                            + cdef_pair_offset(dir + rotation, tap, signed))
                            as usize;
                        secondary += 1;
                    }
                    sign += 1;
                }
                tap += 1;
            }
            row += 1;
        }
        dir += 1;
    }
    starts
};

const fn cdef_pair_offset(direction: usize, tap: usize, sign: i32) -> isize {
    let [dy, dx] = CDEF_DIRECTIONS[direction & 7][tap];
    (sign * dy) as isize * CDEF_PAIR_STRIDE as isize + (sign * dx) as isize * 2
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_padded_row<const W: usize>(
    pad: &[u16; CDEF_PADDED_AREA],
    start: usize,
) -> Option<&[u16; W]> {
    let end = start.checked_add(W)?;
    pad.get(start..end)?.try_into().ok()
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

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_primary_rows<'a, const W: usize>(
    pad: &'a [u16; CDEF_PADDED_AREA],
    starts: &CdefPrimaryStarts,
) -> Option<[[&'a [u16; W]; 2]; 2]> {
    Some([
        [
            cdef_padded_row(pad, starts[0][0])?,
            cdef_padded_row(pad, starts[0][1])?,
        ],
        [
            cdef_padded_row(pad, starts[1][0])?,
            cdef_padded_row(pad, starts[1][1])?,
        ],
    ])
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_secondary_rows<'a, const W: usize>(
    pad: &'a [u16; CDEF_PADDED_AREA],
    starts: &CdefSecondaryStarts,
) -> Option<[[[&'a [u16; W]; 2]; 2]; 2]> {
    Some([
        [
            [
                cdef_padded_row(pad, starts[0][0][0])?,
                cdef_padded_row(pad, starts[0][0][1])?,
            ],
            [
                cdef_padded_row(pad, starts[0][1][0])?,
                cdef_padded_row(pad, starts[0][1][1])?,
            ],
        ],
        [
            [
                cdef_padded_row(pad, starts[1][0][0])?,
                cdef_padded_row(pad, starts[1][0][1])?,
            ],
            [
                cdef_padded_row(pad, starts[1][1][0])?,
                cdef_padded_row(pad, starts[1][1][1])?,
            ],
        ],
    ])
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_primary_row_simd<const W: usize>(
    center_row: &[u16; W],
    pri_rows: &[[&[u16; W]; 2]; 2],
    pri_taps: [i32; 2],
    pri_str: i32,
    pri_adj: i32,
) -> [u16; W] {
    let center = Simd::from_array(*center_row).cast::<i16>();
    let p00 = Simd::from_array(*pri_rows[0][0]).cast::<i16>();
    let p01 = Simd::from_array(*pri_rows[0][1]).cast::<i16>();
    let p10 = Simd::from_array(*pri_rows[1][0]).cast::<i16>();
    let p11 = Simd::from_array(*pri_rows[1][1]).cast::<i16>();
    let sum = Simd::splat(pri_taps[0] as i16)
        * (constrain_with_adj_simd(p00 - center, pri_str, pri_adj)
            + constrain_with_adj_simd(p01 - center, pri_str, pri_adj))
        + Simd::splat(pri_taps[1] as i16)
            * (constrain_with_adj_simd(p10 - center, pri_str, pri_adj)
                + constrain_with_adj_simd(p11 - center, pri_str, pri_adj));
    let negative = sum.is_negative().select(Simd::splat(1), Simd::splat(0));
    (center + ((Simd::splat(8) + sum - negative) >> 4))
        .cast::<u16>()
        .to_array()
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_secondary_row_simd<const W: usize>(
    center_row: &[u16; W],
    sec_rows: &[[[&[u16; W]; 2]; 2]; 2],
    sec_taps: [i32; 2],
    sec_str: i32,
    sec_adj: i32,
) -> [u16; W] {
    let center = Simd::from_array(*center_row).cast::<i16>();
    let s000 = Simd::from_array(*sec_rows[0][0][0]).cast::<i16>();
    let s001 = Simd::from_array(*sec_rows[0][0][1]).cast::<i16>();
    let s010 = Simd::from_array(*sec_rows[0][1][0]).cast::<i16>();
    let s011 = Simd::from_array(*sec_rows[0][1][1]).cast::<i16>();
    let s100 = Simd::from_array(*sec_rows[1][0][0]).cast::<i16>();
    let s101 = Simd::from_array(*sec_rows[1][0][1]).cast::<i16>();
    let s110 = Simd::from_array(*sec_rows[1][1][0]).cast::<i16>();
    let s111 = Simd::from_array(*sec_rows[1][1][1]).cast::<i16>();
    let sum = Simd::splat(sec_taps[0] as i16)
        * (constrain_with_adj_simd(s000 - center, sec_str, sec_adj)
            + constrain_with_adj_simd(s001 - center, sec_str, sec_adj)
            + constrain_with_adj_simd(s010 - center, sec_str, sec_adj)
            + constrain_with_adj_simd(s011 - center, sec_str, sec_adj))
        + Simd::splat(sec_taps[1] as i16)
            * (constrain_with_adj_simd(s100 - center, sec_str, sec_adj)
                + constrain_with_adj_simd(s101 - center, sec_str, sec_adj)
                + constrain_with_adj_simd(s110 - center, sec_str, sec_adj)
                + constrain_with_adj_simd(s111 - center, sec_str, sec_adj));
    let negative = sum.is_negative().select(Simd::splat(1), Simd::splat(0));
    (center + ((Simd::splat(8) + sum - negative) >> 4))
        .cast::<u16>()
        .to_array()
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn constrain_with_adj_simd<const W: usize>(
    diff: Simd<i16, W>,
    threshold: i32,
    damping_adj: i32,
) -> Simd<i16, W> {
    let abs = diff.abs().cast::<u16>();
    let clip = Simd::splat(threshold as u16)
        .saturating_sub(abs >> damping_adj as u16)
        .cast::<i16>();
    diff.simd_min(clip).simd_max(-clip)
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_pair<const W: usize, const V: usize>(first: &[u16; W], second: &[u16; W]) -> [u16; V] {
    debug_assert_eq!(V, W * 2);
    core::array::from_fn(|i| if i < W { first[i] } else { second[i - W] })
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_full_rows_paired<const W: usize, const V: usize, const HAS_UNAVAILABLE: bool>(
    center_row: &[u16; V],
    pri_rows: &[[[&[u16; W]; 2]; 2]; 2],
    sec_rows: &[[[[&[u16; W]; 2]; 2]; 2]; 2],
    pri_taps: [i32; 2],
    sec_taps: [i32; 2],
    pri_str: i32,
    sec_str: i32,
    pri_adj: i32,
    sec_adj: i32,
) -> [u16; V] {
    let center = Simd::from_array(*center_row).cast::<i16>();
    let mut sum = Simd::splat(0);
    let mut min = center;
    let mut max = center;
    macro_rules! add_pair {
        ($first:expr, $second:expr, $strength:expr, $adjustment:expr, $weight:expr) => {{
            let first = Simd::from_array(cdef_pair::<W, V>($first[0], $first[1])).cast::<i16>();
            let second = Simd::from_array(cdef_pair::<W, V>($second[0], $second[1])).cast::<i16>();
            min = min.simd_min(first).simd_min(second);
            let first_available = if HAS_UNAVAILABLE {
                first
                    .simd_eq(Simd::splat(CDEF_UNAVAILABLE as i16))
                    .select(center, first)
            } else {
                first
            };
            let second_available = if HAS_UNAVAILABLE {
                second
                    .simd_eq(Simd::splat(CDEF_UNAVAILABLE as i16))
                    .select(center, second)
            } else {
                second
            };
            max = max.simd_max(first_available).simd_max(second_available);
            sum += Simd::splat($weight as i16)
                * (constrain_with_adj_simd(first - center, $strength, $adjustment)
                    + constrain_with_adj_simd(second - center, $strength, $adjustment));
        }};
    }
    add_pair!(
        [&pri_rows[0][0][0], &pri_rows[1][0][0]],
        [&pri_rows[0][0][1], &pri_rows[1][0][1]],
        pri_str,
        pri_adj,
        pri_taps[0]
    );
    add_pair!(
        [&pri_rows[0][1][0], &pri_rows[1][1][0]],
        [&pri_rows[0][1][1], &pri_rows[1][1][1]],
        pri_str,
        pri_adj,
        pri_taps[1]
    );
    add_pair!(
        [&sec_rows[0][0][0][0], &sec_rows[1][0][0][0]],
        [&sec_rows[0][0][0][1], &sec_rows[1][0][0][1]],
        sec_str,
        sec_adj,
        sec_taps[0]
    );
    add_pair!(
        [&sec_rows[0][0][1][0], &sec_rows[1][0][1][0]],
        [&sec_rows[0][0][1][1], &sec_rows[1][0][1][1]],
        sec_str,
        sec_adj,
        sec_taps[0]
    );
    add_pair!(
        [&sec_rows[0][1][0][0], &sec_rows[1][1][0][0]],
        [&sec_rows[0][1][0][1], &sec_rows[1][1][0][1]],
        sec_str,
        sec_adj,
        sec_taps[1]
    );
    add_pair!(
        [&sec_rows[0][1][1][0], &sec_rows[1][1][1][0]],
        [&sec_rows[0][1][1][1], &sec_rows[1][1][1][1]],
        sec_str,
        sec_adj,
        sec_taps[1]
    );
    let negative = sum.is_negative().select(Simd::splat(1), Simd::splat(0));
    let rounded = center + ((Simd::splat(8) + sum - negative) >> 4);
    rounded.simd_max(min).simd_min(max).cast::<u16>().to_array()
}

fn cdef_filter_block_interior_rows_paired<
    const W: usize,
    const V: usize,
    const HAS_UNAVAILABLE: bool,
    const PAD_STRIDE: usize,
    const PAD_CENTER: usize,
>(
    pad: &[u16; CDEF_PADDED_AREA],
    h: usize,
    filter: &CdefBlockFilter,
    row_starts: &CdefRowStarts,
    out: &mut [u16],
    out_stride: usize,
) -> Option<()> {
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(filter.pri_str, filter.damping);
    let sec_adj = constrain_damping_adj(filter.sec_str, filter.damping);
    let center_start = 2 * PAD_STRIDE + PAD_CENTER;
    for row in (0..h).step_by(2) {
        let next_row = (row + 1).min(h - 1);
        let center_rows = [
            cdef_padded_row::<W>(pad, center_start + row * PAD_STRIDE)?,
            cdef_padded_row::<W>(pad, center_start + next_row * PAD_STRIDE)?,
        ];
        let center = cdef_pair::<W, V>(center_rows[0], center_rows[1]);
        let filtered = if filter.pri_str != 0 && filter.sec_str != 0 {
            let pri_rows = [
                cdef_primary_rows::<W>(pad, &row_starts[row].0)?,
                cdef_primary_rows::<W>(pad, &row_starts[next_row].0)?,
            ];
            let sec_rows = [
                cdef_secondary_rows::<W>(pad, &row_starts[row].1)?,
                cdef_secondary_rows::<W>(pad, &row_starts[next_row].1)?,
            ];
            cdef_filter_full_rows_paired::<W, V, HAS_UNAVAILABLE>(
                &center,
                &pri_rows,
                &sec_rows,
                pri_taps,
                sec_taps,
                filter.pri_str,
                filter.sec_str,
                pri_adj,
                sec_adj,
            )
        } else if filter.pri_str != 0 {
            let pri_rows = [
                cdef_primary_rows::<W>(pad, &row_starts[row].0)?,
                cdef_primary_rows::<W>(pad, &row_starts[next_row].0)?,
            ];
            let pri_data: [[[u16; V]; 2]; 2] = core::array::from_fn(|tap| {
                core::array::from_fn(|sign| {
                    cdef_pair::<W, V>(pri_rows[0][tap][sign], pri_rows[1][tap][sign])
                })
            });
            let pri_refs =
                core::array::from_fn(|tap| core::array::from_fn(|sign| &pri_data[tap][sign]));
            cdef_filter_primary_row_simd(&center, &pri_refs, pri_taps, filter.pri_str, pri_adj)
        } else if filter.sec_str != 0 {
            let sec_rows = [
                cdef_secondary_rows::<W>(pad, &row_starts[row].1)?,
                cdef_secondary_rows::<W>(pad, &row_starts[next_row].1)?,
            ];
            let sec_data: [[[[u16; V]; 2]; 2]; 2] = core::array::from_fn(|tap| {
                core::array::from_fn(|sign| {
                    core::array::from_fn(|dir| {
                        cdef_pair::<W, V>(sec_rows[0][tap][sign][dir], sec_rows[1][tap][sign][dir])
                    })
                })
            });
            let sec_refs = core::array::from_fn(|tap| {
                core::array::from_fn(|sign| core::array::from_fn(|dir| &sec_data[tap][sign][dir]))
            });
            cdef_filter_secondary_row_simd(&center, &sec_refs, sec_taps, filter.sec_str, sec_adj)
        } else {
            center
        };
        cdef_output_row::<W>(out, out_stride, row)?.copy_from_slice(&filtered[..W]); // splot-copy-ok: publish paired SIMD-filtered rows into output
        if row + 1 < h {
            cdef_output_row::<W>(out, out_stride, row + 1)?.copy_from_slice(&filtered[W..]); // splot-copy-ok: publish paired SIMD-filtered rows into output
        }
    }
    Some(())
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
    let row_starts = &CDEF_ROW_STARTS[filter.dir & 7];
    match w.min(8) {
        8 => cdef_filter_block_interior_rows_paired::<8, 16, HAS_UNAVAILABLE, CDEF_PADDED_SIDE, 2>(
            pad,
            h.min(8),
            filter,
            row_starts,
            out,
            out_stride,
        ),
        4 => cdef_filter_block_interior_rows_paired::<4, 8, HAS_UNAVAILABLE, CDEF_PADDED_SIDE, 2>(
            pad,
            h.min(8),
            filter,
            row_starts,
            out,
            out_stride,
        ),
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
    cdef_filter_block_interior_rows_paired::<8, 16, false, CDEF_PAIR_STRIDE, 4>(
        pad,
        h,
        filter,
        &CDEF_PAIR_ROW_STARTS[filter.dir & 7],
        out,
        8,
    )
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
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 13) & 0x3ff) as u16
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
                    coeff_shift: 2,
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
