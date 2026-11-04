// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Simd,
    cmp::SimdOrd,
    num::{SimdInt, SimdUint},
    simd_swizzle,
};

use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};

use crate::Result;

use super::{
    GDF_BIAS, GDF_COORDS, GDF_INTRA_REF_DST, GDF_READ_RADIUS, GdfBlock, GdfClass, GdfSource,
    exact_slice, gdf_state_error,
};

pub(super) const TAP_REACH: usize = GDF_READ_RADIUS - 1;
/// Source rows a row pair reads: `TAP_REACH` above the pair to `TAP_REACH` below it.
pub(super) const WINDOW_ROWS: usize = 2 * TAP_REACH + 2;

/// The source rows a row pair reads, each `len` samples from column `origin.0 - TAP_REACH`.
/// Inlined so callers see that every row has length `len`.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn source_rows<'a>(
    source: &GdfSource<'a>,
    origin: (usize, usize),
    len: usize,
) -> Result<[&'a [u16]; WINDOW_ROWS]> {
    let first_row = origin.1.checked_sub(TAP_REACH);
    let first_col = origin.0.checked_sub(TAP_REACH);
    let (Some(first_row), Some(first_col)) = (first_row, first_col) else {
        return Err(gdf_state_error());
    };
    let mut rows = [&source.samples[..0]; WINDOW_ROWS];
    for (index, row) in rows.iter_mut().enumerate() {
        let start = (first_row + index)
            .checked_mul(source.stride)
            .and_then(|start| start.checked_add(first_col))
            .ok_or_else(gdf_state_error)?;
        *row = exact_slice(source.samples, start, len).ok_or_else(gdf_state_error)?;
    }
    Ok(rows)
}

/// The `WIN` samples of every row in `rows` from column `x`.
#[inline]
pub(super) fn window_at<'a, const WIN: usize>(
    rows: &[&'a [u16]; WINDOW_ROWS],
    x: usize,
) -> Option<[&'a [u16; WIN]; WINDOW_ROWS]> {
    let end = x.checked_add(WIN)?;
    if rows.iter().any(|row| row.len() < end) {
        return None;
    }
    Some(rows.map(|row| {
        row.get(x..)
            .and_then(<[u16]>::first_chunk)
            .unwrap_or(const { &[0; WIN] })
    }))
}

/// Expands `$body` once per window row with `$row` bound to the row index as a constant.
macro_rules! for_each_window_row {
    ($row:ident => $body:block) => {
        for_each_window_row!(@ $row $body; 0 1 2 3 4 5 6 7 8 9 10 11 12 13);
    };
    (@ $row:ident $body:block; $($index:literal)*) => {
        $({
            const $row: usize = $index;
            $body
        })*
    };
}

/// Bit `index * 18 + k` is set when weight `index` of tap `k` is zero in every
/// GDF table for every class in the `classes` bit set.
const fn zero_weight_taps(classes: u8) -> u64 {
    let mut mask = 0;
    let mut bit = 0;
    while bit < 3 * GDF_COORDS.len() {
        let (index, k) = (bit / GDF_COORDS.len(), bit % GDF_COORDS.len());
        let mut zero = true;
        let mut table = 0;
        while table < GDF_WEIGHT.len() * GDF_WEIGHT[0].len() {
            let weights = &GDF_WEIGHT[table / GDF_WEIGHT[0].len()][table % GDF_WEIGHT[0].len()];
            let mut class = 0;
            while class < 4 {
                zero &= classes >> class & 1 == 0 || weights[index][k][class] == 0;
                class += 1;
            }
            table += 1;
        }
        if zero {
            mask |= 1 << bit;
        }
        bit += 1;
    }
    mask
}

/// The largest column distance of a tap at each row distance.
const TAP_COLUMN_REACH: [usize; TAP_REACH + 1] = {
    let mut reach = [0; TAP_REACH + 1];
    let mut k = 0;
    while k < GDF_COORDS.len() {
        let (dy, dx) = GDF_COORDS[k];
        if dx.unsigned_abs() > reach[dy as usize] {
            reach[dy as usize] = dx.unsigned_abs();
        }
        k += 1;
    }
    reach
};

/// The largest minus the smallest sample that the taps of a row pair read
/// from `window`, when it is below `limit`. The two center rows go first:
/// in textured windows they alone usually reach `limit`.
#[inline]
pub(super) fn tap_range<const W: usize, const WIN: usize>(
    window: &[&[u16; WIN]; WINDOW_ROWS],
    limit: u16,
) -> Option<u16> {
    const { assert!(WIN == W + 2 * TAP_REACH && W >= 4 && WINDOW_ROWS == 14) };
    let (mut high, mut low) = ([Simd::<u16, 8>::splat(0); 2], [Simd::splat(u16::MAX); 2]);
    let mut range = 0;
    for center_rows in [true, false] {
        for_each_window_row!(ROW => {
            const REACH: usize = {
                let above = ROW.abs_diff(TAP_REACH);
                let below = ROW.abs_diff(TAP_REACH + 1);
                TAP_COLUMN_REACH[if above < below { above } else { below }]
            };
            if (ROW == TAP_REACH || ROW == TAP_REACH + 1) == center_rows {
                let end = TAP_REACH + W + REACH;
                let mut col = TAP_REACH - REACH;
                while col < end {
                    let at = col.min(end - 8);
                    let samples = Simd::<u16, 8>::from_slice(&window[ROW][at..at + 8]);
                    high[ROW & 1] = high[ROW & 1].simd_max(samples);
                    low[ROW & 1] = low[ROW & 1].simd_min(samples);
                    col += 8;
                }
            }
        });
        range = high[0].simd_max(high[1]).reduce_max() - low[0].simd_min(low[1]).reduce_min();
        if range >= limit {
            return None;
        }
    }
    Some(range)
}

/// Multiplier of the gradient sums: twice the error-index scale, so that each
/// error-table digit is the high half of a biased sum.
pub(super) fn gdf_index_scale(block: &GdfBlock) -> i16 {
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        16
    } else {
        10
    }
}

/// Filters in place a 16-sample row pair whose window has `tap_range` 0:
/// every tap adds zero, so the sums stay at `init`, one lane per two-sample
/// class block, for both rows.
#[inline(never)]
pub(super) fn flat_rows(output: [&mut [u16; 16]; 2], init: &[Simd<i32, 8>; 3], block: &GdfBlock) {
    let residual = simd_swizzle!(
        flat_residual(*init, block),
        [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]
    );
    for output in output {
        *output = add_residual(Simd::from_array(*output), residual, block).to_array();
    }
}

/// The residual that `flat_rows` adds to both samples of each class block.
#[inline]
pub(super) fn flat_residual(init: [Simd<i32, 8>; 3], block: &GdfBlock) -> Simd<i16, 8> {
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        let error = &GDF_INTRA_ERROR[block.qp_idx];
        residual_with_error::<8, 8, 4096>(block, error, init)
    } else {
        let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
        residual_with_error::<8, 5, 1000>(block, error, init)
    }
}

#[inline]
fn add_residual<const W: usize>(
    base: Simd<u16, W>,
    residual: Simd<i16, W>,
    block: &GdfBlock,
) -> Simd<u16, W> {
    (base.cast::<i16>() + residual)
        .simd_max(Simd::splat(0))
        .simd_min(Simd::splat(block.max_sample as i16))
        .cast::<u16>()
}

#[inline]
fn tap_samples<const W: usize, const WIN: usize>(row: &[u16; WIN], col: usize) -> Simd<i16, W> {
    Simd::<u16, W>::from_slice(&row[col..col + W]).cast()
}

/// The residual of biased gradient sums, already multiplied by `2 * SCALE`.
fn residual_with_error<const WIDTH: usize, const SCALE: i32, const ERROR_LEN: usize>(
    block: &GdfBlock,
    error: &[i32; ERROR_LEN],
    sums: [Simd<i32, WIDTH>; 3],
) -> Simd<i16, WIDTH> {
    let digit_offset = Simd::splat((1 << 15) + (SCALE << 16));
    let [first, second, third] = sums.map(|sum| {
        ((sum + (sum >> 31) + digit_offset) >> 16)
            .cast::<i16>()
            .simd_clamp(Simd::splat(0), Simd::splat(2 * SCALE as i16 - 1))
    });
    let radix = Simd::splat(2 * SCALE as i16);
    let pos = (first * radix + second) * radix + third;
    let error = Simd::gather_or_default(error, pos.cast::<usize>()).cast::<i16>();
    let scaled_error = error * Simd::splat(block.pix_scale as i16);
    let rounding = 12 - i16::from(block.bit_depth.bits());
    if rounding == 0 {
        scaled_error
    } else {
        (scaled_error + Simd::splat(1 << (rounding - 1)) + (scaled_error >> 15)) >> rounding
    }
}

/// The tap that class `class` reads in place of class-0 tap `k`: class bit 0
/// transposes the tap and bit 1 mirrors it horizontally. A tap and its
/// negation are the same tap. `GDF_COORDS.len()` marks a missing tap.
const fn class_tap(k: usize, class: usize) -> usize {
    let (mut dy, mut dx) = GDF_COORDS[k];
    if class & 1 != 0 {
        (dy, dx) = (dx, dy);
    }
    if class & 2 != 0 {
        dx = -dx;
    }
    if dy < 0 || (dy == 0 && dx < 0) {
        (dy, dx) = (-dy, -dx);
    }
    let mut tap = 0;
    while tap < GDF_COORDS.len() && (GDF_COORDS[tap].0 != dy || GDF_COORDS[tap].1 != dx) {
        tap += 1;
    }
    tap
}

/// Every table's weights and clip bounds of class `c` are those of class 0
/// with the taps moved by `class_tap`, so one class-0 table serves all lanes.
const _: () = {
    let mut index = 0;
    while index < 6 * 6 * 4 * GDF_COORDS.len() {
        let (reference, qp) = (
            index / (6 * 4 * GDF_COORDS.len()),
            index / (4 * GDF_COORDS.len()) % 6,
        );
        let (class, k) = (index / GDF_COORDS.len() % 4, index % GDF_COORDS.len());
        let tap = class_tap(k, class);
        assert!(tap < GDF_COORDS.len());
        assert!(GDF_ALPHA[reference][qp][tap][class] == GDF_ALPHA[reference][qp][k][0]);
        let mut sum = 0;
        while sum < 3 {
            let weights = &GDF_WEIGHT[reference][qp][sum];
            assert!(weights[tap][class] == weights[k][0]);
            sum += 1;
        }
        index += 1;
    }
};

/// Class-0 tap pairs and the sum that each pair's multiply-add feeds.
const SYM_PAIRS: [(usize, usize, usize); 15] = [
    (0, 1, 0),
    (12, 13, 1),
    (2, 3, 0),
    (14, 15, 1),
    (3, 15, 2),
    (4, 6, 0),
    (7, 11, 1),
    (8, 10, 0),
    (8, 10, 1),
    (9, 17, 0),
    (9, 17, 1),
    (9, 17, 2),
    (5, 16, 0),
    (5, 16, 1),
    (5, 16, 2),
];
const CLASS0_ZERO_WEIGHTS: u64 = zero_weight_taps(0b0001);

/// `SYM_PAIRS` adds every class-0 weight that some table makes nonzero once.
const _: () = {
    let mut bit = 0;
    while bit < 3 * GDF_COORDS.len() {
        let (sum, k) = (bit / GDF_COORDS.len(), bit % GDF_COORDS.len());
        let mut uses = 0;
        let mut pair = 0;
        while pair < SYM_PAIRS.len() {
            let (a, b, target) = SYM_PAIRS[pair];
            if target == sum && (a == k || b == k) {
                uses += 1;
            }
            pair += 1;
        }
        assert!(uses == 1 || (uses == 0 && CLASS0_ZERO_WEIGHTS >> bit & 1 != 0));
        bit += 1;
    }
};

/// Class-0 clip bounds and weights of one GDF table, the weights scaled by
/// `gdf_index_scale` and packed per `SYM_PAIRS` entry as two i16 lanes.
pub(super) struct GdfSymParams {
    /// Each tap's clip bounds `[-alpha, alpha]` as whole vectors, so the
    /// clamps read them from memory instead of rebuilding them per row.
    bounds: [[Simd<i16, 16>; 2]; GDF_COORDS.len()],
    pair_weights: [Simd<i16, 16>; SYM_PAIRS.len()],
    bias: [i32; 3],
    scale: i32,
    /// Whether a flat window with gradient bias 0 keeps its samples.
    keeps_flat: bool,
}

impl GdfSymParams {
    pub(super) fn new(block: &GdfBlock) -> Self {
        let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
        let table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
        let scale = gdf_index_scale(block);
        let bias = GDF_BIAS[block.ref_dst_idx][block.qp_idx].map(|bias| bias * i32::from(scale));
        Self {
            bounds: core::array::from_fn(|k| {
                let alpha = alpha[k][0] as i16;
                [Simd::splat(-alpha), Simd::splat(alpha)]
            }),
            pair_weights: SYM_PAIRS.map(|(a, b, sum)| {
                let weight = |k: usize| table[sum][k][0] * scale;
                Simd::from_array(core::array::from_fn(|lane| {
                    weight(if lane & 1 == 0 { a } else { b })
                }))
            }),
            bias,
            scale: i32::from(scale),
            keeps_flat: flat_residual(bias.map(Simd::splat), block) == Simd::splat(0),
        }
    }
}

/// Lanes `0..4, 8..12` and `4..8, 12..16` of two vectors interleaved, as an
/// in-lane AVX2 unpack gives them.
#[inline]
fn interleave_pairs(a: Simd<i16, 16>, b: Simd<i16, 16>) -> [Simd<i16, 16>; 2] {
    [
        simd_swizzle!(
            a,
            b,
            [0, 16, 1, 17, 2, 18, 3, 19, 8, 24, 9, 25, 10, 26, 11, 27]
        ),
        simd_swizzle!(
            a,
            b,
            [4, 20, 5, 21, 6, 22, 7, 23, 12, 28, 13, 29, 14, 30, 15, 31]
        ),
    ]
}

/// Sums of adjacent products. Written as a product and a pairwise add so that
/// LLVM still emits one `pmaddwd` after folding the interleave into it.
#[inline]
fn multiply_add_pairs(x: Simd<i16, 16>, w: Simd<i16, 16>) -> Simd<i32, 8> {
    let product = x.cast::<i32>() * w.cast::<i32>();
    simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15])
}

/// The samples of tap `TAP` for row `ROW` minus the center samples: the tap's
/// two mirrored reads.
#[allow(clippy::inline_always)]
#[inline(always)]
fn tap_diffs<const TAP: usize, const ROW: usize>(
    window: &[&[u16; 28]; WINDOW_ROWS],
    center: Simd<i16, 16>,
) -> (Simd<i16, 16>, Simd<i16, 16>) {
    let (dy, dx) = GDF_COORDS[TAP];
    let row = TAP_REACH + ROW;
    (
        tap_samples::<16, 28>(
            window[row - dy as usize],
            (TAP_REACH as isize - dx) as usize,
        ) - center,
        tap_samples::<16, 28>(
            window[row + dy as usize],
            (TAP_REACH as isize + dx) as usize,
        ) - center,
    )
}

/// The clipped sum of `above` and `below`, each within tap `TAP`'s bounds.
#[allow(clippy::inline_always)]
#[inline(always)]
fn clipped_sum(
    above: Simd<i16, 16>,
    below: Simd<i16, 16>,
    bounds: &[Simd<i16, 16>; 2],
) -> Simd<i16, 16> {
    let low = bounds[0];
    let high = bounds[1];
    (above.simd_max(low).simd_min(high) + below.simd_max(low).simd_min(high))
        .simd_max(Simd::splat(-512))
        .simd_min(Simd::splat(511))
}

/// The clipped sums of taps `K` and `T`, whose candidate samples coincide:
/// each lane of one reads the tap its class moves `K` to, so the two combs
/// select between the same two diff pairs. `select` is the class mask that
/// sends `K`'s lanes to `T` (`transposed`, or `mirrored` when `K`'s transpose
/// is itself and its mirror is `T`).
#[allow(clippy::inline_always)]
#[inline(always)]
fn paired_combs<const K: usize, const T: usize, const ROW: usize>(
    window: &[&[u16; 28]; WINDOW_ROWS],
    center: Simd<i16, 16>,
    select: Simd<i16, 16>,
    params: &GdfSymParams,
) -> [Simd<i16, 16>; 2] {
    let (above, below) = tap_diffs::<K, ROW>(window, center);
    let (t_above, t_below) = tap_diffs::<T, ROW>(window, center);
    let pick_k_above = above ^ ((above ^ t_above) & select);
    let pick_t_above = t_above ^ ((above ^ t_above) & select);
    let pick_k_below = below ^ ((below ^ t_below) & select);
    let pick_t_below = t_below ^ ((below ^ t_below) & select);
    [
        clipped_sum(pick_k_above, pick_k_below, &params.bounds[K]),
        clipped_sum(pick_t_above, pick_t_below, &params.bounds[T]),
    ]
}

/// The clipped sums of the tap cycle `A`, `B`, `C`, `D`, whose four combs
/// select among the same four diff pairs: mirroring swaps `A`/`B` and
/// `C`/`D`, transposing swaps the pairs.
#[allow(clippy::inline_always)]
#[inline(always)]
fn cycle_combs<const A: usize, const B: usize, const C: usize, const D: usize, const ROW: usize>(
    window: &[&[u16; 28]; WINDOW_ROWS],
    center: Simd<i16, 16>,
    (transposed, mirrored): (Simd<i16, 16>, Simd<i16, 16>),
    params: &GdfSymParams,
) -> [Simd<i16, 16>; 4] {
    let (a_above, a_below) = tap_diffs::<A, ROW>(window, center);
    let (b_above, b_below) = tap_diffs::<B, ROW>(window, center);
    let (c_above, c_below) = tap_diffs::<C, ROW>(window, center);
    let (d_above, d_below) = tap_diffs::<D, ROW>(window, center);
    let select4 = |a: Simd<i16, 16>, b: Simd<i16, 16>, c: Simd<i16, 16>, d: Simd<i16, 16>| {
        let pick_ab = a ^ ((a ^ b) & mirrored);
        let pick_ba = b ^ ((a ^ b) & mirrored);
        let pick_cd = c ^ ((c ^ d) & mirrored);
        let pick_dc = d ^ ((c ^ d) & mirrored);
        let pick_a = pick_ab ^ ((pick_ab ^ pick_cd) & transposed);
        let pick_c = pick_cd ^ ((pick_ab ^ pick_cd) & transposed);
        let pick_b = pick_ba ^ ((pick_ba ^ pick_dc) & transposed);
        let pick_d = pick_dc ^ ((pick_ba ^ pick_dc) & transposed);
        [pick_a, pick_b, pick_c, pick_d]
    };
    let above = select4(a_above, b_above, c_above, d_above);
    let below = select4(a_below, b_below, c_below, d_below);
    [
        clipped_sum(above[0], below[0], &params.bounds[A]),
        clipped_sum(above[1], below[1], &params.bounds[B]),
        clipped_sum(above[2], below[2], &params.bounds[C]),
        clipped_sum(above[3], below[3], &params.bounds[D]),
    ]
}

/// Adds the weighted comb pair `a`/`b` of `SYM_PAIRS` entry `P` to its sum.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_pair<const P: usize>(
    sums: &mut [[Simd<i32, 8>; 2]; 3],
    a: Simd<i16, 16>,
    b: Simd<i16, 16>,
    params: &GdfSymParams,
) {
    let weights = params.pair_weights[P];
    for (sum, half) in sums[SYM_PAIRS[P].2].iter_mut().zip(interleave_pairs(a, b)) {
        *sum += multiply_add_pairs(half, weights);
    }
}

/// Filters in place row `ROW` of 16 samples of any class mix. Not inlined:
/// across both rows LLVM would hoist the widened weights out of the
/// multiply-adds and spill them. Taps are grouped so that each group's combs
/// share their candidate samples and selects: a tap and its transpose read
/// mirrored offsets, and taps `4`, `6`, `7`, `11` form a cycle.
#[inline(never)]
fn sym_row<const ROW: usize>(
    window: &[&[u16; 28]; WINDOW_ROWS],
    output: &mut [u16; 16],
    masks: (Simd<i16, 16>, Simd<i16, 16>),
    gradient: Simd<i32, 16>,
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let center = tap_samples::<16, 28>(window[TAP_REACH + ROW], TAP_REACH);
    let split = |v: Simd<i32, 16>| {
        [
            simd_swizzle!(v, [0, 1, 2, 3, 8, 9, 10, 11]),
            simd_swizzle!(v, [4, 5, 6, 7, 12, 13, 14, 15]),
        ]
    };
    let mut sums = [
        split(Simd::splat(params.bias[0])),
        split(Simd::splat(params.bias[1])),
        split(gradient),
    ];
    let (transposed, mirrored) = masks;
    let [tap0, tap12] = paired_combs::<0, 12, ROW>(window, center, transposed, params);
    let [tap1, tap13] = paired_combs::<1, 13, ROW>(window, center, transposed, params);
    add_pair::<0>(&mut sums, tap0, tap1, params);
    add_pair::<1>(&mut sums, tap12, tap13, params);
    let [tap2, tap14] = paired_combs::<2, 14, ROW>(window, center, transposed, params);
    let [tap3, tap15] = paired_combs::<3, 15, ROW>(window, center, transposed, params);
    add_pair::<2>(&mut sums, tap2, tap3, params);
    add_pair::<3>(&mut sums, tap14, tap15, params);
    add_pair::<4>(&mut sums, tap3, tap15, params);
    let [tap4, tap6, tap7, tap11] = cycle_combs::<4, 6, 7, 11, ROW>(window, center, masks, params);
    add_pair::<5>(&mut sums, tap4, tap6, params);
    add_pair::<6>(&mut sums, tap7, tap11, params);
    let [tap8, tap10] = paired_combs::<8, 10, ROW>(window, center, mirrored, params);
    add_pair::<7>(&mut sums, tap8, tap10, params);
    add_pair::<8>(&mut sums, tap8, tap10, params);
    let [tap9, tap17] = paired_combs::<9, 17, ROW>(window, center, transposed, params);
    add_pair::<9>(&mut sums, tap9, tap17, params);
    add_pair::<10>(&mut sums, tap9, tap17, params);
    add_pair::<11>(&mut sums, tap9, tap17, params);
    let [tap5, tap16] = paired_combs::<5, 16, ROW>(window, center, transposed, params);
    add_pair::<12>(&mut sums, tap5, tap16, params);
    add_pair::<13>(&mut sums, tap5, tap16, params);
    add_pair::<14>(&mut sums, tap5, tap16, params);
    let base = Simd::from_array(*output);
    let residual = if block.ref_dst_idx == GDF_INTRA_REF_DST {
        sym_residual::<8, 4096>(&GDF_INTRA_ERROR_I16[block.qp_idx], sums, block)
    } else {
        sym_residual::<5, 1024>(
            &GDF_INTER_ERROR_I16[block.ref_dst_idx - 1][block.qp_idx],
            sums,
            block,
        )
    };
    *output = add_residual(base, residual, block).to_array();
}

/// The error tables as `i16`, the inter ones padded to a power of two so
/// that a masked index needs no bounds check.
static GDF_INTRA_ERROR_I16: [[i16; 4096]; 6] = {
    let mut table = [[0; 4096]; 6];
    let mut index = 0;
    while index < 6 * 4096 {
        let value = GDF_INTRA_ERROR[index / 4096][index % 4096];
        assert!(value == value as i16 as i32);
        table[index / 4096][index % 4096] = value as i16;
        index += 1;
    }
    table
};
static GDF_INTER_ERROR_I16: [[[i16; 1024]; 6]; 5] = {
    let mut table = [[[0; 1024]; 6]; 5];
    let mut index = 0;
    while index < 5 * 6 * 1000 {
        let (reference, qp, entry) = (index / 6000, index / 1000 % 6, index % 1000);
        let value = GDF_INTER_ERROR[reference][qp][entry];
        assert!(value == value as i16 as i32);
        table[reference][qp][entry] = value as i16;
        index += 1;
    }
    table
};

/// The residual of `sym_row` from its sums in `interleave_pairs` halves,
/// already multiplied by `2 * SCALE`: each sum gives one error-table digit.
#[allow(clippy::inline_always)]
#[inline(always)]
fn sym_residual<const SCALE: i32, const LEN: usize>(
    error: &[i16; LEN],
    sums: [[Simd<i32, 8>; 2]; 3],
    block: &GdfBlock,
) -> Simd<i16, 16> {
    const { assert!(LEN.is_power_of_two() && LEN >= (2 * SCALE * 2 * SCALE * 2 * SCALE) as usize) };
    let digit_offset = Simd::splat((1 << 15) + (SCALE << 16));
    let [first, second, third] = sums.map(|halves| {
        let [low, high] = halves.map(|sum| {
            ((sum + (sum >> 31) + digit_offset) >> 16)
                .simd_max(Simd::splat(0))
                .simd_min(Simd::splat(2 * SCALE - 1))
        });
        simd_swizzle!(
            low,
            high,
            [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
        )
        .cast::<i16>()
    });
    let radix = Simd::splat(2 * SCALE as i16);
    let pos = ((first * radix + second) * radix + third).to_array();
    let error = Simd::from_array(core::array::from_fn(|lane| {
        error[pos[lane] as usize & (LEN - 1)]
    }));
    let scaled_error = error * Simd::splat(block.pix_scale as i16);
    let rounding = 12 - i16::from(block.bit_depth.bits());
    if rounding == 0 {
        scaled_error
    } else {
        (scaled_error + Simd::splat(1 << (rounding - 1)) + (scaled_error >> 15)) >> rounding
    }
}

/// Filters in place a row pair of 16 samples whose class changes every two
/// samples, reading class-0 weights and moving each lane's taps by its class.
#[inline(never)]
pub(super) fn sym_rows(
    window: &[&[u16; 28]; WINDOW_ROWS],
    [top, bottom]: [&mut [u16; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let bias = Simd::from_array(classes.map(GdfClass::gradient_bias));
    if tap_range::<16, 28>(window, 1) == Some(0) {
        if params.keeps_flat && bias == Simd::splat(0) {
            return;
        }
        let [first, second, gradient] = params.bias.map(Simd::splat);
        let init = [first, second, bias * Simd::splat(params.scale) + gradient];
        flat_rows([top, bottom], &init, block);
        return;
    }
    let index = (Simd::from_array(classes.map(|class| class.0)) & Simd::splat(3)).cast::<i16>();
    let lanes: Simd<i16, 16> =
        simd_swizzle!(index, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]);
    let masks = (-(lanes & Simd::splat(1)), -((lanes >> 1) & Simd::splat(1)));
    let gradient = bias * Simd::splat(params.scale) + Simd::splat(params.bias[2]);
    let gradient = simd_swizzle!(gradient, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]);
    sym_row::<0>(window, top, masks, gradient, block, params);
    sym_row::<1>(window, bottom, masks, gradient, block, params);
}

/// Filters in place `width` samples (at most 16) from column `x` of a row
/// pair through `sym_rows`, on copies padded to 16 lanes.
pub(super) fn sym_rows_narrow(
    rows: &[&[u16]; WINDOW_ROWS],
    x: usize,
    width: usize,
    [top, bottom]: [&mut [u16]; 2],
    classes: &[GdfClass],
    block: &GdfBlock,
    params: &GdfSymParams,
) -> Option<()> {
    let mut window = [[0; 28]; WINDOW_ROWS];
    for (padded, row) in window.iter_mut().zip(rows) {
        let samples = row.get(x..x.checked_add(width + 2 * TAP_REACH)?)?;
        padded.get_mut(..samples.len())?.copy_from_slice(samples);
    }
    let mut lane_classes = [GdfClass::default(); 8];
    let class_count = width.div_ceil(2);
    lane_classes
        .get_mut(..class_count)?
        .copy_from_slice(classes.get(..class_count)?);
    let (top, bottom) = (top.get_mut(..width)?, bottom.get_mut(..width)?);
    let mut output = [[0; 16]; 2];
    output[0][..width].copy_from_slice(top);
    output[1][..width].copy_from_slice(bottom);
    let [first, second] = &mut output;
    sym_rows(
        &window.each_ref(),
        [first, second],
        &lane_classes,
        block,
        params,
    );
    top.copy_from_slice(&output[0][..width]);
    bottom.copy_from_slice(&output[1][..width]);
    Some(())
}
