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

#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, Level, SimdInto, i32x8};
use fearless_simd::{
    Bytes, Select as _, Simd as SimdToken, SimdBase, SimdInt as _, i8x16, i16x8, i16x16, i32x4,
    u8x16, u16x8, u16x16,
};

use crate::ReconSample;

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
    let rows = block.map(|row| row.map(|x| x as i16));
    baseline_simd!( simd => cdef_direction_rows(
        simd,
        rows.map(|row| i16x8::from_slice(simd, &row)),
    ))
}

/// AV2 § 7.18.2 CDEF direction process over the interior padded block layout.
///
/// The 8x8 luma block begins at row 2, column 2 of `pad`; `coeff_shift` is
/// `BitDepth - 8`. The result matches [`cdef_direction`] without materializing
/// the intermediate shifted 8x8 array.
pub fn cdef_direction_padded(pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32) -> (usize, i32) {
    cdef_direction_strided::<CDEF_PADDED_SIDE, _>(pad, coeff_shift)
}

/// [`cdef_direction_padded`] for a block of a luma segment, as
/// [`CdefLumaSegment::block`] gives it.
pub fn cdef_direction_segment(
    pad: &[u16; CDEF_SEGMENT_BLOCK_AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    cdef_direction_strided::<CDEF_SEGMENT_STRIDE, _>(pad, coeff_shift)
}

/// The eight pre-shifted rows of the block at row 2, column 2 of `pad`.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_direction_input<S: SimdToken, const STRIDE: usize, const AREA: usize>(
    simd: S,
    pad: &[u16; AREA],
    coeff_shift: u32,
) -> [i16x8<S>; 8] {
    let row = |i: usize| {
        let start = (i + 2) * STRIDE + 2;
        u16x8::from_slice(simd, &pad[start..start + 8])
    };
    // 8-bit rows skip the shift: a register-count shift is two micro-ops.
    if coeff_shift == 0 {
        core::array::from_fn(|i| row(i).bitcast::<i16x8<S>>() - 128)
    } else {
        core::array::from_fn(|i| (row(i) >> coeff_shift).bitcast::<i16x8<S>>() - 128)
    }
}

#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_direction_strided<const STRIDE: usize, const AREA: usize>(
    pad: &[u16; AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        let rows = cdef_direction_input::<_, STRIDE, AREA>(avx2, pad, coeff_shift);
        return cdef_best_direction(cdef_direction_costs_avx2(avx2, rows));
    }
    baseline_simd!( simd => cdef_direction_rows(
        simd,
        cdef_direction_input::<_, STRIDE, AREA>(simd, pad, coeff_shift),
    ))
}

/// The § 7.18.2 `yDir` and `var` of the eight direction costs: costs are
/// never negative, so the scan, which keeps a direction only when its cost
/// beats the best so far (from 0), ends on the first direction that holds
/// the largest cost.
#[cfg(target_arch = "x86_64")]
fn cdef_best_direction(cost: [i32; 8]) -> (usize, i32) {
    let best_cost = cost.into_iter().max().unwrap_or(0);
    let ties = (0..8).fold(0u32, |ties, dir| {
        ties | (u32::from(cost[dir] == best_cost) << dir)
    });
    let y_dir = (ties.trailing_zeros() & 7) as usize;
    (y_dir, (best_cost - cost[(y_dir + 4) & 7]) >> 10)
}

/// A 16-lane partial-sum accumulator held as its low and high halves.
type CdefPartial<S> = [i16x8<S>; 2];

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
/// not change it.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn cdef_direction_rows<S: SimdToken>(simd: S, rows: [i16x8<S>; 8]) -> (usize, i32) {
    let zero = i16x8::splat(simd, 0);
    let up = |p: CdefPartial<S>| {
        [
            simd.slide_i16x8::<7>(zero, p[0]),
            simd.slide_i16x8::<7>(p[0], p[1]),
        ]
    };
    let down = |p: CdefPartial<S>| {
        [
            simd.slide_i16x8::<1>(p[0], p[1]),
            simd.slide_i16x8::<1>(p[1], zero),
        ]
    };
    let pairwise_sum =
        |a: i16x8<S>, b: i16x8<S>| simd.unzip_low_i16x8(a, b) + simd.unzip_high_i16x8(a, b);
    let low_four = i16x8::from_slice(simd, &[-1, -1, -1, -1, 0, 0, 0, 0]);
    let (mut partial0, mut partial1, mut partial3) = ([zero; 2], [zero; 2], [zero; 2]);
    let (mut partial4, mut partial5, mut partial7) = ([zero; 2], [zero; 2], [zero; 2]);
    let mut vertical = zero;
    let mut row_sums = [zero; 4];
    for (k, row_sum) in row_sums.iter_mut().enumerate() {
        let (first, second) = (rows[2 * k], rows[2 * k + 1]);
        *row_sum = pairwise_sum(first, second);
        let first_pairs = *row_sum & low_four;
        let second_pairs = simd.slide_i16x8::<4>(*row_sum, zero);
        for (row, pairs) in [(first, first_pairs), (second, second_pairs)] {
            partial0 = down(partial0);
            partial0[1] += row;
            partial4 = up(partial4);
            partial4[0] += row;
            partial1 = down(partial1);
            partial1[1] += pairs;
            partial3 = up(partial3);
            partial3[0] += pairs;
        }
        let both = first + second;
        partial5 = up(partial5);
        partial5[0] += both;
        partial7 = down(partial7);
        partial7[1] += both;
        vertical += both;
    }
    let horizontal = pairwise_sum(
        pairwise_sum(row_sums[0], row_sums[1]),
        pairwise_sum(row_sums[2], row_sums[3]),
    );
    let squares = |half: i16x8<S>| {
        let (low, high) = simd.widen_i16x8(half);
        (low * low, high * high)
    };
    let cost = |partial: CdefPartial<S>, weights: [[i32; 8]; 2]| {
        let weight = |half: usize, at: usize| i32x4::from_slice(simd, &weights[half][at..at + 4]);
        let (a, b) = squares(partial[0]);
        let (c, d) = squares(partial[1]);
        a * weight(0, 0) + b * weight(0, 4) + c * weight(1, 0) + d * weight(1, 4)
    };
    let line_cost = |partial: i16x8<S>| {
        let (a, b) = squares(partial);
        (a + b) * DIV_TABLE[8]
    };
    let pairs = |a: i32x4<S>, b: i32x4<S>| simd.unzip_low_i32x4(a, b) + simd.unzip_high_i32x4(a, b);
    let sums = |[a, b, c, d]: [i32x4<S>; 4]| pairs(pairs(a, b), pairs(c, d));
    let low = sums([
        cost(partial0, const { cdef_cost_weights(15, 1) }),
        cost(partial1, const { cdef_cost_weights(11, 1) }),
        line_cost(horizontal),
        cost(partial3, const { cdef_cost_weights(11, 0) }),
    ]);
    let high = sums([
        cost(partial4, const { cdef_cost_weights(15, 0) }),
        cost(partial5, const { cdef_cost_weights(11, 0) }),
        line_cost(vertical),
        cost(partial7, const { cdef_cost_weights(11, 5) }),
    ]);
    let best = i32x4::splat(simd, low.max(high).reduce_max());
    let first = |costs: i32x4<S>, dirs: [i32; 4]| {
        costs
            .simd_eq(best)
            .select(i32x4::from_slice(simd, &dirs), i32x4::splat(simd, 8))
    };
    let y_dir = (first(low, [0, 1, 2, 3])
        .min(first(high, [4, 5, 6, 7]))
        .reduce_min()
        & 7) as usize;
    let var = [(best - high).to_array(), (best - low).to_array()];
    (y_dir, var[y_dir >> 2][y_dir & 3] >> 10)
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// Every row `k` shares a register with row `7 - k` in the upper half, so
    /// each in-lane shift builds a partial and its mirror: the diagonals give
    /// partial 0 below and partial 4 (with entries reversed) above, the column
    /// pair sums give partial 7 below and partial 5 above, and the row pair sums,
    /// taken two rows four apart per register, give partial 1 below and partial
    /// 3 (reversed) above. A partial's entries spill past lane 7 into a second
    /// register, which is reversed so that each entry meets its equal-weight
    /// mirror in one multiply-add.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn cdef_direction_costs_avx2(t: Avx2, rows: [i16x8<Avx2>; 8]) -> [i32; 8] {
        use core::arch::x86_64::{
            __m256i, _mm_add_epi16, _mm_hadd_epi16, _mm256_add_epi16, _mm256_add_epi32,
            _mm256_bslli_epi128, _mm256_bsrli_epi128, _mm256_castsi256_si128,
            _mm256_extracti128_si256, _mm256_hadd_epi16, _mm256_hadd_epi32, _mm256_madd_epi16,
            _mm256_mullo_epi32, _mm256_set_m128i, _mm256_set1_epi32, _mm256_setr_epi8,
            _mm256_setr_epi32, _mm256_shuffle_epi8, _mm256_unpackhi_epi16, _mm256_unpacklo_epi16,
        };
        let r: [__m256i; 8] =
            core::array::from_fn(|k| _mm256_set_m128i(rows[7 - k].into(), rows[k].into()));
        let add = _mm256_add_epi16;
        let column = add(add(r[0], r[1]), add(r[2], r[3]));
        let column = _mm_add_epi16(
            _mm256_castsi256_si128(column),
            _mm256_extracti128_si256::<1>(column),
        );
        let quads = _mm256_hadd_epi16(_mm256_hadd_epi16(r[0], r[1]), _mm256_hadd_epi16(r[2], r[3]));
        let row = _mm_hadd_epi16(
            _mm256_castsi256_si128(quads),
            _mm256_extracti128_si256::<1>(quads),
        );
        let lines = _mm256_set_m128i(row, column);
        let lines = _mm256_mullo_epi32(_mm256_madd_epi16(lines, lines), _mm256_set1_epi32(105));
        let shifted = |v: &[__m256i]| -> (__m256i, __m256i) {
            let mut low = v[0];
            let mut high = _mm256_set1_epi32(0);
            macro_rules! step {
                ($k:literal) => {
                    if $k < v.len() {
                        low = add(low, _mm256_bslli_epi128::<{ 2 * $k }>(v[$k]));
                        high = add(high, _mm256_bsrli_epi128::<{ 16 - 2 * $k }>(v[$k]));
                    }
                };
            }
            step!(1);
            step!(2);
            step!(3);
            step!(4);
            step!(5);
            step!(6);
            step!(7);
            (low, high)
        };
        let cost = |(low, high): (__m256i, __m256i), reverse: __m256i, weights: [__m256i; 2]| {
            let high = _mm256_shuffle_epi8(high, reverse);
            let first = _mm256_unpacklo_epi16(low, high);
            let second = _mm256_unpackhi_epi16(low, high);
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_madd_epi16(first, first), weights[0]),
                _mm256_mullo_epi32(_mm256_madd_epi16(second, second), weights[1]),
            )
        };
        let z = -1;
        let diagonal = _mm256_setr_epi8(
            12, 13, 10, 11, 8, 9, 6, 7, 4, 5, 2, 3, 0, 1, z, z, 12, 13, 10, 11, 8, 9, 6, 7, 4, 5,
            2, 3, 0, 1, z, z,
        );
        let alternate = _mm256_setr_epi8(
            4, 5, 2, 3, 0, 1, z, z, z, z, z, z, z, z, z, z, 4, 5, 2, 3, 0, 1, z, z, z, z, z, z, z,
            z, z, z,
        );
        let div = |a, b, c, d| _mm256_setr_epi32(a, b, c, d, a, b, c, d);
        let diagonal_weights = [div(840, 420, 280, 210), div(168, 140, 120, 105)];
        let alternate_weights = [div(420, 210, 140, 105), div(105, 105, 105, 105)];
        let diagonals = cost(shifted(&r), diagonal, diagonal_weights);
        let pairs: [__m256i; 4] = core::array::from_fn(|i| _mm256_hadd_epi16(r[i], r[i + 4]));
        let pair_costs = cost(shifted(&pairs), alternate, alternate_weights);
        let columns: [__m256i; 4] = core::array::from_fn(|k| add(r[2 * k], r[2 * k + 1]));
        let column_costs = cost(shifted(&columns), alternate, alternate_weights);
        let sums = _mm256_hadd_epi32(
            _mm256_hadd_epi32(diagonals, pair_costs),
            _mm256_hadd_epi32(column_costs, lines),
        );
        let sums = SimdInto::<i32x8<Avx2>, Avx2>::simd_into(sums, t).to_array();
        [0, 1, 7, 5, 4, 6, 3, 2].map(|lane| sums[lane])
    }
);

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
/// [`cdef_filter_block_interior_to`]: an 8x8 block plus the § 7.18.3
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

/// Per-block § 7.18.3 filter constants for [`cdef_filter_block_interior_to`].
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

/// Two consecutive 8-lane rows of one tap view as one 16-lane vector.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_row_pair<S: SimdToken, const STRIDE: usize, const SPAN: usize>(
    simd: S,
    view: &[u16; SPAN],
    row: usize,
) -> Option<u16x16<S>> {
    let first = view.get(row * STRIDE..)?.first_chunk::<8>()?;
    let second = view.get((row + 1) * STRIDE..)?.first_chunk::<8>()?;
    Some(simd.combine_u16x8(
        u16x8::from_slice(simd, first),
        u16x8::from_slice(simd, second),
    ))
}

/// [`cdef_row_pair`] as `i16` lanes.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_row_pair_i16<S: SimdToken, const STRIDE: usize, const SPAN: usize>(
    simd: S,
    view: &[u16; SPAN],
    row: usize,
) -> Option<i16x16<S>> {
    Some(cdef_row_pair::<S, STRIDE, SPAN>(simd, view, row)?.bitcast())
}

/// `i8` lanes widened to `i16`, through the lower half of a doubled vector,
/// which AVX2 widens in one instruction.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn widen_i8<S: SimdToken>(simd: S, lanes: i8x16<S>) -> i16x16<S> {
    simd.widen_i8x32(simd.combine_i8x16(lanes, lanes)).0
}

/// [`widen_i8`] for `u8` lanes.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn widen_u8<S: SimdToken>(simd: S, lanes: u8x16<S>) -> u16x16<S> {
    simd.widen_u8x32(simd.combine_u8x16(lanes, lanes)).0
}

/// `i16` lanes narrowed to `i8` with saturation.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn narrow_i16<S: SimdToken>(simd: S, lanes: i16x16<S>) -> i8x16<S> {
    let (low, high) = simd.split_i16x16(lanes);
    simd.saturating_narrow_i16x8(low, high)
}

/// `(8 + sum - (sum < 0)) >> 4`, the § 7.18.3 rounding of a tap sum.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_round<S: SimdToken>(sum: i16x16<S>) -> i16x16<S> {
    (sum + 8 + (sum >> 15)) >> 4
}

/// Writes the two 8-lane rows of `filtered` to rows `row` and `row + 1` of
/// `out`, `W` samples each; the second only when it is below `h`.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_store_row_pair<S: SimdToken, const W: usize, T: ReconSample>(
    filtered: u16x16<S>,
    out: &mut [T],
    out_stride: usize,
    row: usize,
    h: usize,
) -> Option<()> {
    if let Some(out) = T::u8_slice_mut(out) {
        let (low, high) = filtered.simd.split_u16x16(filtered);
        return cdef_store_rows::<W, _>(
            filtered.simd.saturating_narrow_u16x8(low, high).to_array(),
            out,
            out_stride,
            row,
            h,
        );
    }
    cdef_store_rows::<W, _>(
        filtered.to_array(),
        T::u16_slice_mut(out)?,
        out_stride,
        row,
        h,
    )
}

/// [`cdef_store_row_pair`] of the row pair `lanes`.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_store_rows<const W: usize, E: Copy>(
    lanes: [E; 16],
    out: &mut [E],
    out_stride: usize,
    row: usize,
    h: usize,
) -> Option<()> {
    cdef_output_row::<W, E>(out, out_stride, row)?.copy_from_slice(&lanes[..W]); // splot-copy-ok: publish paired SIMD-filtered rows into output
    if row + 1 < h {
        cdef_output_row::<W, E>(out, out_stride, row + 1)?.copy_from_slice(&lanes[8..8 + W]); // splot-copy-ok: publish paired SIMD-filtered rows into output
    }
    Some(())
}

/// Runs `body`, which returns an `Option`, with `row` set to each `first`
/// below `rows` and `h`, unrolled so that every row offset is a constant.
macro_rules! for_each_row_pair {
    ([$($first:literal),*], $rows:expr, $h:expr, |$row:ident| $body:block) => {
        $({
            let $row: usize = $first;
            if $row < $rows && $row < $h {
                $body?;
            }
        })*
    };
}

/// Per-block § 7.18.3 `constrain` threshold and damping shift.
#[derive(Clone, Copy)]
struct CdefConstrain<S: SimdToken> {
    threshold: u16x16<S>,
    shift: u32,
}

impl<S: SimdToken> CdefConstrain<S> {
    fn new(simd: S, threshold: i32, damping: i32) -> Self {
        Self {
            threshold: u16x16::splat(simd, threshold as u16),
            shift: constrain_damping_adj(threshold, damping) as u32,
        }
    }

    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply(self, diff: i16x16<S>) -> i16x16<S> {
        let clip = self
            .threshold
            .saturating_sub(diff.abs().bitcast::<u16x16<S>>() >> self.shift)
            .bitcast::<i16x16<S>>();
        diff.min(clip).max(-clip)
    }
}

/// A block's centre view and its four primary and eight secondary tap views.
type CdefTapViews<'a, E, const SPAN: usize> =
    (&'a [E; SPAN], [&'a [E; SPAN]; 4], [&'a [E; SPAN]; 8]);

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
    E,
    const STRIDE: usize,
    const CENTER: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    pad: &'a [E; AREA],
    dir: usize,
    starts: &CdefTapStarts,
) -> Option<CdefTapViews<'a, E, SPAN>> {
    const { assert!(u8::MAX as usize + SPAN <= AREA) };
    let starts = &starts[dir & 7];
    let view =
        |tap: usize| -> Option<&[E; SPAN]> { pad.get(usize::from(starts[tap])..)?.first_chunk() };
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

/// The output rows and filter of a block kernel: `pad`, rows below `h`, the
/// § 7.18.3 filter, the tap starts, and `out` with its stride.
type CdefRowsArgs<'a, const AREA: usize, T> = (
    &'a [u16; AREA],
    usize,
    &'a CdefBlockFilter,
    &'a CdefTapStarts,
    &'a mut [T],
    usize,
);

/// § 7.18.3 over a padded block, two 8-lane rows per vector; a block `W`
/// samples wide stores the first `W` lanes of each row.
///
/// The taps are read through [`cdef_tap_views`]. The row pairs are unrolled,
/// so every row load is a constant offset from a view. `ROWS` is the most
/// rows the layout holds; rows at or past `h` are computed and discarded.
/// With `HAS_UNAVAILABLE`, taps equal to [`CDEF_UNAVAILABLE`] leave the max
/// unchanged and constrain to zero.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows<
    S: SimdToken,
    T: ReconSample,
    const W: usize,
    const HAS_UNAVAILABLE: bool,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    (pad, h, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA, T>,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| tap as i16);
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| tap as i16);
    let pri = CdefConstrain::new(simd, filter.pri_str, filter.damping);
    let sec = CdefConstrain::new(simd, filter.sec_str, filter.damping);
    let unavailable = i16x16::splat(simd, CDEF_UNAVAILABLE as i16);
    let pair = |view, row| cdef_row_pair_i16::<S, STRIDE, SPAN>(simd, view, row);
    for_each_row_pair!([0, 2, 4, 6], ROWS, h, |row| {
        let center = pair(center_view, row)?;
        let mut sum = i16x16::splat(simd, 0);
        let (mut min, mut max) = (center, center);
        macro_rules! add_pair {
            ($first:expr, $second:expr, $constrain:expr, $weight:expr) => {{
                let first = pair($first, row)?;
                let second = pair($second, row)?;
                if PRI && SEC {
                    min = min.min(first).min(second);
                    let (first_max, second_max) = if HAS_UNAVAILABLE {
                        (
                            first.simd_eq(unavailable).select(center, first),
                            second.simd_eq(unavailable).select(center, second),
                        )
                    } else {
                        (first, second)
                    };
                    max = max.max(first_max).max(second_max);
                }
                sum += ($constrain.apply(first - center) + $constrain.apply(second - center))
                    * $weight;
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
        let mut filtered = center + cdef_round(sum);
        if PRI && SEC {
            filtered = filtered.max(min).min(max);
        }
        cdef_store_row_pair::<S, W, _>(filtered.bitcast(), out, out_stride, row, h)
    });
    Some(())
}

/// Per-block § 7.18.3 `constrain` for 8-bit samples in `u8` lanes.
#[derive(Clone, Copy)]
struct CdefConstrain8<S: SimdToken> {
    threshold: u8x16<S>,
    shift: u32,
}

impl<S: SimdToken> CdefConstrain8<S> {
    fn new(simd: S, threshold: i32, damping: i32) -> Self {
        Self {
            threshold: u8x16::splat(simd, threshold as u8),
            shift: constrain_damping_adj(threshold, damping) as u32,
        }
    }

    /// `constrain(tap - center)` as `sign * min(|diff|, clip)`, which is the
    /// clamp to `[-clip, clip]` that [`CdefConstrain::apply`] computes.
    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply(self, tap: u8x16<S>, center: u8x16<S>) -> i8x16<S> {
        let diff = tap.max(center) - tap.min(center);
        let clip = self.threshold.saturating_sub(diff >> self.shift);
        let magnitude = diff.min(clip).bitcast::<i8x16<S>>();
        tap.simd_lt(center).select(-magnitude, magnitude)
    }

    /// `constrain(diff)` on a tap difference in `i8` lanes; the wrapping
    /// `abs` read as `u8` is `|diff|` also for `-128`.
    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply_diff(self, diff: i8x16<S>) -> i8x16<S> {
        let magnitude = diff.abs().bitcast::<u8x16<S>>();
        let clip = self
            .threshold
            .saturating_sub(magnitude >> self.shift)
            .bitcast::<i8x16<S>>();
        diff.min(clip).max(-clip)
    }
}

/// Whether [`cdef_filter_rows_8bit`] computes `filter` exactly on samples
/// that fit `u8` lanes: strengths below 64 so that a constrained tap pair
/// fits in `i8`, and damping shifts below 8 so that `u8` shifts do not wrap.
fn cdef_u8_lanes_fit(filter: &CdefBlockFilter) -> bool {
    [filter.pri_str, filter.sec_str]
        .into_iter()
        .all(|strength| {
            (0..64).contains(&strength) && constrain_damping_adj(strength, filter.damping) < 8
        })
}

/// [`cdef_filter_rows`] in an interior layout for windows whose samples
/// minus `base` fit `u8`. Each row pair narrows to one 16-lane `u8` vector,
/// so a tap's constrain is one vector operation per step instead of two.
/// § 7.18.3 reads only tap differences and clamps to tap values, so the
/// result moved back by `base` is the same as the `i16` kernel's.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows_8bit<
    S: SimdToken,
    T: ReconSample,
    const W: usize,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    (pad, h, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA, T>,
    base: u16,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| tap as i16);
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| tap as i16);
    let pri = CdefConstrain8::new(simd, filter.pri_str, filter.damping);
    let sec = CdefConstrain8::new(simd, filter.sec_str, filter.damping);
    let base_u8 = u8x16::splat(simd, base as u8);
    let pair = |view: &[u16; SPAN], row: usize| -> Option<u8x16<S>> {
        let lanes = cdef_row_pair::<S, STRIDE, SPAN>(simd, view, row)?;
        let (low, high) = simd.split_u16x16(lanes);
        Some(simd.narrow_u16x8(low, high) - base_u8)
    };
    for_each_row_pair!([0, 2, 4, 6], ROWS, h, |row| {
        let center = pair(center_view, row)?;
        let mut sum = i16x16::splat(simd, 0);
        let (mut min, mut max) = (center, center);
        macro_rules! add_pair {
            ($first:expr, $second:expr, $constrain:expr, $weight:expr) => {{
                let first = pair($first, row)?;
                let second = pair($second, row)?;
                if PRI && SEC {
                    min = min.min(first).min(second);
                    max = max.max(first).max(second);
                }
                let taps = $constrain.apply(first, center) + $constrain.apply(second, center);
                sum += widen_i8(simd, taps) * $weight;
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
        let filtered = widen_u8(simd, center).bitcast::<i16x16<S>>() + cdef_round(sum);
        let filtered = if PRI && SEC {
            let filtered = filtered
                .max(i16x16::splat(simd, 0))
                .min(i16x16::splat(simd, 255));
            let (low, high) = simd.split_u16x16(filtered.bitcast());
            let clipped = simd.narrow_u16x8(low, high).max(min).min(max);
            widen_u8(simd, clipped) + base
        } else {
            (filtered + base as i16).bitcast()
        };
        cdef_store_row_pair::<S, W, _>(filtered, out, out_stride, row, h)
    });
    Some(())
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_output_row<const W: usize, E>(
    out: &mut [E],
    stride: usize,
    row: usize,
) -> Option<&mut [E; W]> {
    out.get_mut(row.checked_mul(stride)?..)?.first_chunk_mut()
}

/// Runs `kernel` with its `PRI` and `SEC` parameters set to which tap
/// families `filter` makes active, or evaluates `none` when neither is.
macro_rules! cdef_by_families {
    ($filter:expr, $kernel:ident::<$($generic:tt),*>($($arg:expr),*), $none:expr) => {
        match ($filter.pri_str != 0, $filter.sec_str != 0) {
            (true, true) => $kernel::<$($generic),*, true, true>($($arg),*),
            (true, false) => $kernel::<$($generic),*, true, false>($($arg),*),
            (false, true) => $kernel::<$($generic),*, false, true>($($arg),*),
            (false, false) => $none,
        }
    };
}

/// Copies a block's centre samples, the output of a block with no active
/// tap family; into `u8` storage, `W <= 8` centre samples narrow as one
/// zero-padded lane group, and a sample above 255 fails the copy.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_copy_centre_rows<
    S: SimdToken,
    const W: usize,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const AREA: usize,
    T: ReconSample,
>(
    simd: S,
    pad: &[u16; AREA],
    h: usize,
    out: &mut [T],
    out_stride: usize,
) -> Option<()> {
    for row in 0..h.min(ROWS) {
        let start = (2 + row) * STRIDE + CENTER;
        let center = pad.get(start..)?.first_chunk::<W>()?;
        if let Some(out) = T::u8_slice_mut(&mut *out) {
            let mut lanes = [0u16; 8];
            lanes.get_mut(..W)?.copy_from_slice(center); // splot-copy-ok: zero-padded lane group
            let lanes = u16x8::load_array(simd, lanes);
            if simd.any_true_mask16x8(lanes.simd_gt(u16x8::splat(simd, 255))) {
                return None;
            }
            let bytes = simd.narrow_u16x8(lanes, lanes).bitcast::<u8x16<S>>();
            let out = cdef_output_row::<W, u8>(out, out_stride, row)?;
            out.copy_from_slice(bytes.as_slice().get(..W)?); // splot-copy-ok: the group's own lanes
        } else {
            let out = T::u16_slice_mut(&mut *out)?;
            cdef_output_row::<W, u16>(out, out_stride, row)?.copy_from_slice(center); // splot-copy-ok: unfiltered block keeps its centre samples
        }
    }
    Some(())
}

/// Whether [`cdef_filter_rows_diff8`] computes `filter` exactly: strengths
/// below 64, and `strength << dampingAdj` at most 127, so that `constrain`
/// is already zero for every difference that saturates `i8`.
fn cdef_i8_diffs_fit(filter: &CdefBlockFilter) -> bool {
    [filter.pri_str, filter.sec_str]
        .into_iter()
        .all(|strength| {
            (0..64).contains(&strength)
                && strength << constrain_damping_adj(strength, filter.damping) <= 127
        })
}

/// [`cdef_filter_rows`] in an interior layout on tap differences saturated
/// to `i8`, for filters [`cdef_i8_diffs_fit`] admits. A saturated
/// difference constrains to zero like the true one, the rounded sum moves
/// the centre by less than 128, and the `min`/`max` clamp reads the
/// differences, so the result is the same as the `i16` kernel's.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows_diff8<
    S: SimdToken,
    T: ReconSample,
    const W: usize,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    (pad, h, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA, T>,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| tap as i16);
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| tap as i16);
    let pri = CdefConstrain8::new(simd, filter.pri_str, filter.damping);
    let sec = CdefConstrain8::new(simd, filter.sec_str, filter.damping);
    let pair = |view, row| cdef_row_pair_i16::<S, STRIDE, SPAN>(simd, view, row);
    for_each_row_pair!([0, 2, 4, 6], ROWS, h, |row| {
        let center = pair(center_view, row)?;
        let mut sum = i16x16::splat(simd, 0);
        let (mut min, mut max) = (i8x16::splat(simd, 0), i8x16::splat(simd, 0));
        macro_rules! add_pair {
            ($first:expr, $second:expr, $constrain:expr, $weight:expr) => {{
                let first = narrow_i16(simd, pair($first, row)? - center);
                let second = narrow_i16(simd, pair($second, row)? - center);
                if PRI && SEC {
                    min = min.min(first).min(second);
                    max = max.max(first).max(second);
                }
                let taps = $constrain.apply_diff(first) + $constrain.apply_diff(second);
                sum += widen_i8(simd, taps) * $weight;
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
        let offset = cdef_round(sum);
        let filtered = if PRI && SEC {
            let (low, high) = simd.split_i16x16(offset);
            let offset = simd.narrow_i16x8(low, high).max(min).min(max);
            center + widen_i8(simd, offset)
        } else {
            center + offset
        };
        cdef_store_row_pair::<S, W, _>(filtered.bitcast(), out, out_stride, row, h)
    });
    Some(())
}

/// `constrain` passes every tap difference through unchanged.
const CDEF_LINEAR: u8 = 2;
/// `constrain` is `Clip3(-strength, strength, d)`.
const CDEF_CLAMPED: u8 = 1;
/// Neither holds.
const CDEF_GENERAL: u8 = 0;

/// How `constrain` acts on every tap difference of a window whose samples
/// span `range`: `constrain(d)` is `d` when `|d| + (|d| >> dampingAdj)` is
/// at most the strength, and `Clip3(-strength, strength, d)` when
/// `|d| >> dampingAdj` is 0. Both grow with `|d|`, so `range` decides them
/// for the whole window.
#[cfg(test)]
fn cdef_window_mode(filter: &CdefBlockFilter, range: i32) -> u8 {
    let (mut linear, mut clamped) = (true, true);
    for strength in [filter.pri_str, filter.sec_str] {
        if strength != 0 {
            let reduced = shr_or_zero(range, constrain_damping_adj(strength, filter.damping));
            linear &= strength > 0 && range + reduced <= strength;
            clamped &= strength > 0 && reduced == 0;
        }
    }
    if linear {
        CDEF_LINEAR
    } else if clamped {
        CDEF_CLAMPED
    } else {
        CDEF_GENERAL
    }
}

/// `value >> shift` for a non-negative `value`, 0 for a shift past its width.
fn shr_or_zero(value: i32, shift: i32) -> i32 {
    u32::try_from(shift)
        .ok()
        .and_then(|shift| value.checked_shr(shift))
        .unwrap_or(0)
}

/// [`cdef_filter_rows`] in an interior layout for windows that
/// [`cdef_window_mode`] calls clamped, or with `!CLAMP` linear. A constrained difference is then the
/// difference clipped to the strength, `Clip3(c - s, c + s, tap) - c`, or
/// the difference itself, so the weighted sum is the weighted sum of the
/// (clipped) taps minus the centre times the active weight, 12 per tap
/// family.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_rows_clamped<
    S: SimdToken,
    T: ReconSample,
    const W: usize,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const CLAMP: bool,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    (pad, h, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA, T>,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| tap as i16);
    let sec_taps = CDEF_SEC_TAPS[tap_row].map(|tap| tap as i16);
    let weight = -12 * (i16::from(PRI) + i16::from(SEC));
    let (pri_str, sec_str) = (filter.pri_str as i16, filter.sec_str as i16);
    let pair = |view, row| cdef_row_pair_i16::<S, STRIDE, SPAN>(simd, view, row);
    for_each_row_pair!([0, 2, 4, 6], ROWS, h, |row| {
        let center = pair(center_view, row)?;
        let mut sum = center * weight;
        let (mut min, mut max) = (center, center);
        macro_rules! add_pair {
            ($first:expr, $second:expr, $weight:expr, $strength:expr) => {{
                let first = pair($first, row)?;
                let second = pair($second, row)?;
                if PRI && SEC {
                    min = min.min(first).min(second);
                    max = max.max(first).max(second);
                }
                let (first, second) = if CLAMP {
                    let (low, high) = (center - $strength, center + $strength);
                    (first.max(low).min(high), second.max(low).min(high))
                } else {
                    (first, second)
                };
                sum += (first + second) * $weight;
            }};
        }
        if PRI {
            add_pair!(pri_views[0], pri_views[1], pri_taps[0], pri_str);
            add_pair!(pri_views[2], pri_views[3], pri_taps[1], pri_str);
        }
        if SEC {
            add_pair!(sec_views[0], sec_views[1], sec_taps[0], sec_str);
            add_pair!(sec_views[2], sec_views[3], sec_taps[0], sec_str);
            add_pair!(sec_views[4], sec_views[5], sec_taps[1], sec_str);
            add_pair!(sec_views[6], sec_views[7], sec_taps[1], sec_str);
        }
        let mut filtered = center + cdef_round(sum);
        if PRI && SEC {
            filtered = filtered.max(min).min(max);
        }
        cdef_store_row_pair::<S, W, _>(filtered.bitcast(), out, out_stride, row, h)
    });
    Some(())
}

/// Dispatches [`cdef_filter_rows`] on which tap families are active; with
/// neither, the block is its centre samples.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_filter_block_rows<
    S: SimdToken,
    T: ReconSample,
    const W: usize,
    const HAS_UNAVAILABLE: bool,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
>(
    simd: S,
    (pad, h, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA, T>,
) -> Option<()> {
    macro_rules! copy {
        () => {
            cdef_copy_centre_rows::<S, W, STRIDE, CENTER, ROWS, AREA, _>(
                simd, pad, h, out, out_stride,
            )
        };
    }
    let args = (pad, h, filter, starts, &mut *out, out_stride);
    if !HAS_UNAVAILABLE && filter.coeff_shift == 0 && cdef_u8_lanes_fit(filter) {
        return cdef_by_families!(
            filter,
            cdef_filter_rows_8bit::<S, _, W, STRIDE, CENTER, ROWS, SPAN, AREA>(simd, args, 0),
            copy!()
        );
    }
    if !HAS_UNAVAILABLE && filter.coeff_shift > 0 && cdef_i8_diffs_fit(filter) {
        return cdef_by_families!(
            filter,
            cdef_filter_rows_diff8::<S, _, W, STRIDE, CENTER, ROWS, SPAN, AREA>(simd, args),
            copy!()
        );
    }
    cdef_by_families!(
        filter,
        cdef_filter_rows::<S, _, W, HAS_UNAVAILABLE, STRIDE, CENTER, ROWS, SPAN, AREA>(simd, args),
        copy!()
    )
}

/// AV2 § 7.18.3 CDEF filter for one fully-interior block written to a strided output.
///
/// Returns `false` when the output geometry cannot hold the 8- or 4-sample-wide
/// CDEF block. The padded input layout matches [`cdef_direction_padded`].
pub fn cdef_filter_block_interior_to<T: ReconSample>(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [T],
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
pub fn cdef_filter_block_interior_to_valid_stride<T: ReconSample>(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [T],
    out_stride: usize,
) -> bool {
    cdef_filter_block_padded_to_valid_stride::<false, _>(pad, w, h, filter, out, out_stride)
}

/// SIMD CDEF filtering for a padded boundary block.
///
/// Tap slots outside the active filter region must contain [`CDEF_UNAVAILABLE`].
/// Returns `false` for unsupported output geometry.
#[doc(hidden)]
pub fn cdef_filter_block_boundary_to_valid_stride<T: ReconSample>(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [T],
    out_stride: usize,
) -> bool {
    cdef_filter_block_padded_to_valid_stride::<true, _>(pad, w, h, filter, out, out_stride)
}

fn cdef_filter_block_padded_to_valid_stride<const HAS_UNAVAILABLE: bool, T: ReconSample>(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [T],
    out_stride: usize,
) -> bool {
    const SPAN: usize = 7 * CDEF_PADDED_SIDE + 8;
    let args = (
        pad,
        h.min(8),
        filter,
        &CDEF_TAP_STARTS,
        &mut *out,
        out_stride,
    );
    match w.min(8) {
        8 => baseline_simd!(simd => cdef_filter_block_rows::<
            _,
            _,
            8,
            HAS_UNAVAILABLE,
            CDEF_PADDED_SIDE,
            2,
            8,
            SPAN,
            CDEF_PADDED_AREA,
        >(simd, args)),
        4 => baseline_simd!(simd => cdef_filter_block_rows::<
            _,
            _,
            4,
            HAS_UNAVAILABLE,
            CDEF_PADDED_SIDE,
            2,
            8,
            SPAN,
            CDEF_PADDED_AREA,
        >(simd, args)),
        0 => None,
        w => {
            let mut block = [T::default(); 64];
            let h = h.min(8);
            if !cdef_filter_block_padded_to_valid_stride::<HAS_UNAVAILABLE, T>(
                pad, 8, h, filter, &mut block, 8,
            ) {
                return false;
            }
            (0..h).try_for_each(|row| {
                let dst = out.get_mut(row.checked_mul(out_stride)?..)?.get_mut(..w)?;
                dst.copy_from_slice(block.get(8 * row..8 * row + w)?); // splot-copy-ok: keep the block's columns inside the plane
                Some(())
            })
        }
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
    let args = (pad, h, filter, &CDEF_PAIR_TAP_STARTS, &mut out[..], 8);
    baseline_simd!(simd => cdef_filter_block_rows::<
        _,
        _,
        8,
        false,
        CDEF_PAIR_STRIDE,
        4,
        4,
        { 3 * CDEF_PAIR_STRIDE + 8 },
        CDEF_PADDED_AREA,
    >(simd, args))
    .is_some()
}

mod segment;
pub use segment::{
    CdefLumaSegment, CdefPairSegment, CdefSegmentBlock, CdefSegmentOutput, CdefStrengths,
    CdefUnitFilters, cdef_filter_block_chroma_pair_segment, cdef_filter_block_segment,
    cdef_filter_segment, cdef_publish_chroma_pair,
};

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
    fn window_admission_rejects_negative_strengths_and_saturates_the_shift() {
        let filter = |pri_str, sec_str, damping| CdefBlockFilter {
            pri_str,
            sec_str,
            damping,
            dir: 0,
            coeff_shift: 2,
        };
        assert_eq!(cdef_window_mode(&filter(-4, 0, 6), 0), CDEF_GENERAL);
        assert_eq!(cdef_window_mode(&filter(0, -2, 6), 0), CDEF_GENERAL);
        assert_eq!(cdef_window_mode(&filter(4, 2, 200), 70_000), CDEF_CLAMPED);
        assert_eq!(cdef_window_mode(&filter(4, 0, 200), 3), CDEF_LINEAR);
    }

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
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for (i, row) in block.iter().enumerate() {
                for (j, &cell) in row.iter().enumerate() {
                    pad[(i + 2) * CDEF_PADDED_SIDE + j + 2] = (cell + 128) as u16;
                }
            }
            assert_eq!(
                cdef_direction_padded(&pad, 0),
                expected,
                "padded case {case}"
            );
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
                        for (w, h) in [(8usize, 8usize), (4, 8), (4, 4)] {
                            let mut out = [u16::MAX; 104];
                            assert!(cdef_filter_block_interior_to(
                                &pad, w, h, &filter, &mut out, 13,
                            ));
                            for i in 0..h {
                                for j in 0..w {
                                    assert_eq!(
                                        i32::from(out[i * 13 + j]),
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

    /// A block cut by the frame edge to a width other than 4 or 8 filters its
    /// columns like the per-sample filter and leaves the columns past it.
    #[test]
    fn partial_width_boundary_blocks_match_per_sample_filter() {
        for (w, h) in [(1, 8), (2, 3), (3, 8), (5, 6), (6, 8), (7, 1)] {
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for (index, sample) in pad.iter_mut().enumerate() {
                let (row, col) = (index / CDEF_PADDED_SIDE, index % CDEF_PADDED_SIDE);
                *sample = if row >= h + 2 || col >= w + 2 || row == 0 {
                    CDEF_UNAVAILABLE
                } else {
                    ((index * 97 + row * 31) % 1024) as u16
                };
            }
            for dir in 0..8 {
                let filter = CdefBlockFilter {
                    pri_str: 12,
                    sec_str: 4,
                    damping: 5,
                    dir,
                    coeff_shift: 2,
                };
                let mut output = [u16::MAX; 80];
                assert!(cdef_filter_block_boundary_to_valid_stride(
                    &pad,
                    w,
                    h,
                    &filter,
                    &mut output,
                    10
                ));
                for (index, &sample) in output.iter().enumerate() {
                    let (row, col) = (index / 10, index % 10);
                    let want = if row < h && col < w {
                        per_sample_reference(&pad, row, col, &filter)
                    } else {
                        i32::from(u16::MAX)
                    };
                    assert_eq!(i32::from(sample), want, "w={w} h={h} dir={dir} at {index}");
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
    fn segment_blocks_match_single_block_kernels() -> Result<(), &'static str> {
        for coeff_shift in [2u32, 0] {
            let mut state = 0x9e37_79b9u32;
            let mut next = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 13) & ((256 << coeff_shift) - 1)) as u16
            };
            let samples = core::array::from_fn(|_| next());
            let planes = core::array::from_fn(|_| core::array::from_fn(|_| next()));
            let luma = luma_segment(&samples);
            let pair = pair_segment(&planes);
            for block in 0..CDEF_SEGMENT_BLOCKS {
                let luma_block = luma.block(block).ok_or("luma block")?;
                let pair_block = pair.block(block).ok_or("pair block")?;
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
                        let mut want = [0u16; 80];
                        assert!(cdef_filter_block_interior_to_valid_stride(
                            &single, 8, 8, &filter, &mut want, 10
                        ));
                        for use_range in [false, true] {
                            let mut got = [0u16; 80];
                            assert!(cdef_filter_block_segment(
                                &luma, block, &filter, use_range, &mut got, 10
                            ));
                            assert_eq!(got, want, "luma block={block} dir={dir} pri={pri_str}");
                        }
                        let (mut got, mut want) =
                            ([0u16; CDEF_PAIR_OUTPUT], [0u16; CDEF_PAIR_OUTPUT]);
                        assert!(cdef_filter_block_chroma_pair_segment(
                            &pair, block, &filter, &mut got
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
        Ok(())
    }

    const PAIR_PLANE_STRIDE: usize = CDEF_PAIR_SEGMENT_STRIDE / 2;

    /// Gathers a luma segment from 12 rows laid out as its scratch.
    fn luma_segment(samples: &[u16; 12 * CDEF_SEGMENT_STRIDE]) -> CdefLumaSegment {
        let mut segment = CdefLumaSegment::default();
        let stride = CDEF_SEGMENT_STRIDE;
        assert!(segment.gather(samples, stride, stride, (2, 2)));
        segment
    }

    /// Gathers a chroma-pair segment from 8 rows of each plane, so that lane
    /// `2 * col + plane` of a scratch row is column `col` of `planes[plane]`.
    fn pair_segment(planes: &[[u16; 8 * PAIR_PLANE_STRIDE]; 2]) -> CdefPairSegment {
        let mut segment = CdefPairSegment::default();
        let stride = PAIR_PLANE_STRIDE;
        assert!(segment.gather(&planes[0], &planes[1], stride, stride, (2, 2)));
        segment
    }

    /// Marks the tap-family mix of `filter` under the linear (row 0) or the
    /// clamped (row 1) kernel when a window of `range` takes one.
    fn mark_mode(modes: &mut [[bool; 3]; 2], filter: &CdefBlockFilter, range: i32) {
        let family = usize::from(filter.pri_str == 0) + usize::from(filter.sec_str == 0) * 2;
        match cdef_window_mode(filter, range) {
            CDEF_LINEAR => modes[0][family] = true,
            CDEF_CLAMPED => modes[1][family] = true,
            _ => {}
        }
    }

    #[test]
    fn rebased_segment_blocks_match_per_sample_filter() {
        let mut state = 0x51ed_270bu32;
        let mut samples = [0u16; 12 * CDEF_SEGMENT_STRIDE];
        let mut single = [0u16; CDEF_PADDED_AREA];
        let mut mode_families = [[false; 3]; 2];
        for (base, range) in [(0u16, 256u32), (767, 256), (300, 41), (500, 4), (90, 13)] {
            samples.fill_with(|| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                base + ((state >> 13) % range) as u16
            });
            for (row, col) in (0..12).flat_map(|row| (0..12).map(move |col| (row, col))) {
                single[row * CDEF_PADDED_SIDE + col] = samples[row * CDEF_SEGMENT_STRIDE + col];
            }
            let segment = luma_segment(&samples);
            let [min, max] = segment.range(0).unwrap_or([0, u16::MAX]);
            for dir in 0..8 {
                for (pri_str, sec_str) in [(1, 0), (60, 0), (0, 4), (0, 16), (15, 4), (60, 16)] {
                    for damping in 5..=8 {
                        let filter = CdefBlockFilter {
                            pri_str,
                            sec_str,
                            damping,
                            dir,
                            coeff_shift: 2,
                        };
                        mark_mode(&mut mode_families, &filter, i32::from(max - min));
                        let mut got = [0u16; 80];
                        assert!(cdef_filter_block_segment(
                            &segment, 0, &filter, true, &mut got, 10
                        ));
                        for (i, j) in (0..8).flat_map(|i| (0..8).map(move |j| (i, j))) {
                            assert_eq!(
                                i32::from(got[i * 10 + j]),
                                per_sample_reference(&single, i, j, &filter),
                                "base={base} dir={dir} pri={pri_str} sec={sec_str} i={i} j={j}"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(mode_families, [[true; 3]; 2]);
    }

    #[test]
    fn narrow_pair_segment_matches_chroma_pair_kernel() {
        let mut state = 0x6a09_e667u32;
        let mut planes = [[0u16; 8 * PAIR_PLANE_STRIDE]; 2];
        let mut single = [0u16; CDEF_PADDED_AREA];
        let mut mode_families = [[false; 3]; 2];
        for spread in [1u32, 3, 6, 13, 20, 40] {
            for row in 0..8 {
                for lane in 0..16 {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let sample = [100u16, 900][lane % 2] + ((state >> 13) % spread) as u16;
                    planes[lane % 2][row * PAIR_PLANE_STRIDE + lane / 2] = sample;
                    single[row * CDEF_PAIR_STRIDE + lane] = sample;
                }
            }
            let segment = pair_segment(&planes);
            let range = segment.range(0).unwrap_or(u16::MAX);
            assert!(u32::from(range) < spread);
            for dir in 0..8 {
                for (pri_str, sec_str) in [(4, 0), (12, 0), (0, 4), (0, 16), (8, 4), (16, 8)] {
                    for damping in 4..=7 {
                        let filter = CdefBlockFilter {
                            pri_str,
                            sec_str,
                            damping,
                            dir,
                            coeff_shift: 2,
                        };
                        mark_mode(&mut mode_families, &filter, i32::from(range));
                        let (mut got, mut want) =
                            ([0u16; CDEF_PAIR_OUTPUT], [0u16; CDEF_PAIR_OUTPUT]);
                        assert!(cdef_filter_block_chroma_pair_segment(
                            &segment, 0, &filter, &mut got
                        ));
                        assert!(cdef_filter_block_chroma_pair(
                            &single, 4, &filter, &mut want
                        ));
                        assert_eq!(got, want, "spread={spread} dir={dir} pri={pri_str}");
                    }
                }
            }
        }
        assert_eq!(mode_families, [[true; 3]; 2]);
    }

    #[test]
    fn linear_and_clamped_window_bounds_match_per_sample_filter() {
        let mut samples = [0u16; 12 * CDEF_SEGMENT_STRIDE];
        let mut single = [0u16; CDEF_PADDED_AREA];
        for strength in [4, 8, 12, 16, 40, 60] {
            for damping in 5..=8 {
                let adj = constrain_damping_adj(strength, damping);
                let linear = (0..=strength)
                    .filter(|&range| range + (range >> adj) <= strength)
                    .max()
                    .unwrap_or(0);
                let clamped = (1 << adj) - 1;
                for range in [linear, linear + 1, clamped, clamped + 1] {
                    for (row, col) in (0..12).flat_map(|row| (0..12).map(move |col| (row, col))) {
                        let sample = 400 + range as u16 * ((row + col) % 2) as u16;
                        samples[row * CDEF_SEGMENT_STRIDE + col] = sample;
                        single[row * CDEF_PADDED_SIDE + col] = sample;
                    }
                    let segment = luma_segment(&samples);
                    assert_eq!(segment.range(0), Some([400, 400 + range as u16]));
                    for dir in 0..8 {
                        for (pri_str, sec_str) in [(strength, 0), (0, strength), (strength, 16)] {
                            let filter = CdefBlockFilter {
                                pri_str,
                                sec_str,
                                damping,
                                dir,
                                coeff_shift: 2,
                            };
                            let mut got = [0u16; 80];
                            assert!(cdef_filter_block_segment(
                                &segment, 0, &filter, true, &mut got, 10
                            ));
                            for (i, j) in (0..8).flat_map(|i| (0..8).map(move |j| (i, j))) {
                                assert_eq!(
                                    i32::from(got[i * 10 + j]),
                                    per_sample_reference(&single, i, j, &filter),
                                    "range={range} dir={dir} pri={pri_str} sec={sec_str}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn saturated_tap_differences_match_per_sample_filter() {
        let mut state = 0x7f4a_7c15u32;
        let mut pad = [0u16; CDEF_PADDED_AREA];
        for round in 0..6u32 {
            for sample in &mut pad[..CDEF_PADDED_SIDE * CDEF_PADDED_SIDE] {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let step = [0, 126, 127, 128, 129, 260][((state >> 9) % 6) as usize];
                let sign = if (state >> 20) & 1 == 0 { 1 } else { -1 };
                *sample = (512 + sign * step + (round as i32 % 3) - 1) as u16;
            }
            for dir in 0..8 {
                for strength in [1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 60, 63] {
                    for damping in 4..=8 {
                        for (pri_str, sec_str) in [(strength, 0), (0, strength), (strength, 16)] {
                            let filter = CdefBlockFilter {
                                pri_str,
                                sec_str,
                                damping,
                                dir,
                                coeff_shift: 2,
                            };
                            let mut out = [0u16; 64];
                            assert!(cdef_filter_block_interior_to(
                                &pad, 8, 8, &filter, &mut out, 8
                            ));
                            for (i, j) in (0..8).flat_map(|i| (0..8).map(move |j| (i, j))) {
                                assert_eq!(
                                    i32::from(out[i * 8 + j]),
                                    per_sample_reference(&pad, i, j, &filter),
                                    "dir={dir} pri={pri_str} sec={sec_str} damping={damping}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// A single tap family has no window clamp: its weights sum to 12 of 16,
    /// and an unavailable tap constrains to 0, so extreme 10-bit pads with any
    /// header strength and damping stay at most 1023.
    #[test]
    fn single_family_filter_of_extreme_samples_stays_in_range() {
        let mut state = 0x2545_f491u32;
        for boundary in [false, true] {
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for (index, sample) in pad.iter_mut().enumerate() {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let (row, col) = (index / CDEF_PADDED_SIDE, index % CDEF_PADDED_SIDE);
                *sample = if boundary && (row < 2 || col < 2) {
                    CDEF_UNAVAILABLE
                } else if state >> 31 == 0 {
                    0
                } else {
                    1023
                };
            }
            for dir in 0..8 {
                for damping in 5..=8 {
                    for (pri_str, sec_str) in [(60, 0), (4, 0), (0, 16), (0, 4), (60, 16)] {
                        let filter = CdefBlockFilter {
                            pri_str,
                            sec_str,
                            damping,
                            dir,
                            coeff_shift: 2,
                        };
                        let mut out = [u16::MAX; 64];
                        assert!(if boundary {
                            cdef_filter_block_boundary_to_valid_stride(
                                &pad, 8, 8, &filter, &mut out, 8,
                            )
                        } else {
                            cdef_filter_block_interior_to(&pad, 8, 8, &filter, &mut out, 8)
                        });
                        assert!(out.iter().all(|&sample| sample <= 1023), "{filter:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn uv_dir_table_is_the_spec_remap() {
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
