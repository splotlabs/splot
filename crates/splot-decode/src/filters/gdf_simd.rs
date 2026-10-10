// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Mask, Select, Simd, SimdElement, ToBytes,
    cmp::{SimdOrd, SimdPartialEq},
    num::{SimdInt, SimdUint},
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

/// Per-lane gradient bias of `W` lanes whose class changes every two lanes.
#[inline]
pub(super) fn class_bias<const W: usize>(classes: &[GdfClass]) -> Simd<i32, W> {
    Simd::from_array(core::array::from_fn(|lane| {
        classes
            .get(lane >> 1)
            .map_or(0, |class| class.gradient_bias())
    }))
}

/// Expands `$body` once per GDF tap with `$k` bound to the tap index as a constant.
macro_rules! for_each_gdf_tap {
    ($k:ident => $body:block) => {
        for_each_gdf_tap!(@ $k $body; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17);
    };
    (@ $k:ident $body:block; $($tap:literal)*) => {
        $({
            const $k: usize = $tap;
            $body
        })*
    };
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

/// Weights that are zero for classes 0 and 2.
pub(super) const EVEN_CLASS_ZERO_WEIGHTS: u64 = zero_weight_taps(0b0101);
/// Weights that are zero for classes 1 and 3.
pub(super) const ODD_CLASS_ZERO_WEIGHTS: u64 = zero_weight_taps(0b1010);

/// Clip bound, its negation and weights of one tap per lane.
pub(super) struct GdfTapWeights<const W: usize> {
    pub(super) alpha: Simd<i16, W>,
    pub(super) low: Simd<i16, W>,
    pub(super) weights: [Simd<i16, W>; 3],
}

/// Most taps that a low-range window may still clip.
const MAX_LOW_RANGE_CLIPS: usize = 4;
/// The only kernel width that takes a `GdfLowRange`: narrower windows of
/// mixed classes are too rarely low-range to pay for measuring the range.
const LOW_RANGE_LANES: usize = 16;

/// The taps with the smallest clip bounds of one class of one GDF table, in
/// bound order, with the bound capped at 256.
#[derive(Clone, Copy)]
pub(super) struct GdfClipTaps {
    taps: [(i16, u8); MAX_LOW_RANGE_CLIPS],
    /// A window range below this clips no tap outside `taps` and needs no
    /// [-512, 511] clamp.
    pub(super) limit: u16,
}

/// `GdfClipTaps` per reference distance, qp and class.
pub(super) static GDF_CLIP_TAPS: [[[GdfClipTaps; 4]; 6]; 6] = {
    let empty = GdfClipTaps {
        taps: [(0, 0); MAX_LOW_RANGE_CLIPS],
        limit: 0,
    };
    let mut table = [[[empty; 4]; 6]; 6];
    let mut index = 0;
    while index < 6 * 6 * 4 {
        let (reference, qp, class) = (index / (6 * 4), index / 4 % 6, index % 4);
        let alpha = &GDF_ALPHA[reference][qp];
        let mut order = [(0_i16, 0_u8); GDF_COORDS.len()];
        let mut k = 0;
        while k < GDF_COORDS.len() {
            let bound = if alpha[k][class] < 256 {
                alpha[k][class] as i16
            } else {
                256
            };
            let mut at = k;
            while at > 0 && order[at - 1].0 > bound {
                order[at] = order[at - 1];
                at -= 1;
            }
            order[at] = (bound, k as u8);
            k += 1;
        }
        let clips = &mut table[reference][qp][class];
        let mut at = 0;
        while at < MAX_LOW_RANGE_CLIPS {
            clips.taps[at] = order[at];
            at += 1;
        }
        clips.limit = order[MAX_LOW_RANGE_CLIPS].0 as u16;
        index += 1;
    }
    table
};

/// Sum of each unscaled weight over all taps per reference distance, qp and
/// class.
pub(super) static GDF_WEIGHT_SUMS: [[[[i16; 3]; 4]; 6]; 6] = {
    let mut table = [[[[0; 3]; 4]; 6]; 6];
    let mut index = 0;
    while index < 6 * 6 * 4 * 3 {
        let (reference, qp) = (index / (6 * 4 * 3), index / (4 * 3) % 6);
        let (class, weight) = (index / 3 % 4, index % 3);
        let mut k = 0;
        while k < GDF_COORDS.len() {
            table[reference][qp][class][weight] += GDF_WEIGHT[reference][qp][weight][k][class];
            k += 1;
        }
        index += 1;
    }
    table
};

/// A window whose samples differ by at most `range` (below its taps'
/// `GdfClipTaps::limit`): a tap whose clip bound is at least `range` clips
/// nothing.
pub(super) struct GdfLowRange<const W: usize> {
    pub(super) range: i16,
    /// Per-lane sum of each unscaled weight over all taps.
    pub(super) weight_sums: [Simd<i16, W>; 3],
    pub(super) scale: i32,
    pub(super) clip_taps: GdfClipTaps,
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

#[inline]
pub(super) fn uniform_gdf_class<const LANES: usize>(classes: &[GdfClass; LANES]) -> Option<u8> {
    let indices = Simd::<i32, LANES>::from_array(classes.map(|class| class.0)) & Simd::splat(3);
    let first = indices[0];
    indices
        .simd_eq(Simd::splat(first))
        .all()
        .then_some(first as u8)
}

/// Weight row `index` of a tap is zero for every table and class of one parity
/// exactly where weight row `index ^ 1` is zero for the other parity, so the
/// mixed-class kernel accumulates an odd class's first two sums swapped. This
/// marks the taps whose swapped weights are zero for both parities.
const MIXED_ZERO_WEIGHTS: u64 = {
    let (even, odd) = (EVEN_CLASS_ZERO_WEIGHTS, ODD_CLASS_ZERO_WEIGHTS);
    let taps = GDF_COORDS.len();
    let mask = (1 << taps) - 1;
    let first = even & (odd >> taps) & mask;
    let second = (even >> taps) & odd & mask;
    first | (second << taps) | (zero_weight_taps(0b1111) >> (2 * taps) << (2 * taps))
};

/// Per-class weights of one GDF table for the mixed-class kernel, scaled by
/// `gdf_index_scale`; an odd class has its first two weight rows swapped.
pub(super) struct GdfMixedParams {
    alpha: &'static [[u16; 4]; 22],
    weights: [[[i16; 4]; GDF_COORDS.len()]; 3],
    bias: [i32; 3],
    scale: i32,
}

impl GdfMixedParams {
    pub(super) fn new(block: &GdfBlock) -> Self {
        let table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
        let scale = gdf_index_scale(block);
        let bias = &GDF_BIAS[block.ref_dst_idx][block.qp_idx];
        Self {
            alpha: &GDF_ALPHA[block.ref_dst_idx][block.qp_idx],
            weights: core::array::from_fn(|index| {
                core::array::from_fn(|tap| {
                    core::array::from_fn(|class| {
                        let row = if index < 2 {
                            index ^ (class & 1)
                        } else {
                            index
                        };
                        table[row][tap][class] * scale
                    })
                })
            }),
            bias: bias.map(|bias| bias * i32::from(scale)),
            scale: i32::from(scale),
        }
    }
}

/// Filters in place a row pair of `W` samples whose class changes every two samples.
pub(super) fn mixed_class_rows<const W: usize, const WIN: usize>(
    window: &[&[u16; WIN]; WINDOW_ROWS],
    output: [&mut [u16; W]; 2],
    classes: &[GdfClass],
    block: &GdfBlock,
    params: &GdfMixedParams,
) {
    let indices = Simd::<i32, 4>::from_array(core::array::from_fn(|lane| {
        classes.get(lane).map_or(0, |class| class.0)
    }));
    let byte_offsets = Simd::from_array(core::array::from_fn(|byte| (byte & 1) as u8));
    let lane_bytes =
        ((indices & Simd::splat(3)) * Simd::splat(0x0202_0202)).to_ne_bytes() + byte_offsets;
    let odd = (Simd::<u16, 8>::from_ne_bytes(lane_bytes) & Simd::splat(2))
        .simd_ne(Simd::splat(0))
        .resize::<W>(false);
    let lane_bytes = [lane_bytes, lane_bytes + Simd::splat(8)];
    let per_class = move |table: Simd<i16, 8>, k: usize| {
        let bytes = table.to_ne_bytes().swizzle_dyn(lane_bytes[k & 1]);
        Simd::<i16, 8>::from_ne_bytes(bytes).resize::<W>(0)
    };
    let weights = |k| {
        let alpha = per_class(tap_pair(params.alpha, k).cast(), k);
        GdfTapWeights {
            alpha,
            low: -alpha,
            weights: core::array::from_fn(|index| {
                per_class(tap_pair(&params.weights[index], k), k)
            }),
        }
    };
    let [first, second, gradient] = params.bias.map(Simd::splat);
    let odd_sums = odd.cast::<i32>();
    let init = [
        odd_sums.select(second, first),
        odd_sums.select(first, second),
        class_bias(classes) * Simd::splat(params.scale) + gradient,
    ];
    let filter = gdf_rows::<W, WIN, 2, MIXED_ZERO_WEIGHTS, true>;
    filter(window, 0, output, init, odd, block, weights, None);
}

/// Filters in place the `ROWS` rows of `W` samples in `output`, from row
/// `first_row` of the row pair whose source rows are `rows`. The three sums
/// start at `init`, scaled by `gdf_index_scale` like the weights; weights
/// marked in `ZERO_WEIGHTS` are skipped. With `SWAPPED`, the lanes in `odd`
/// hold their first two sums swapped.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn gdf_rows<
    const W: usize,
    const WIN: usize,
    const ROWS: usize,
    const ZERO_WEIGHTS: u64,
    const SWAPPED: bool,
>(
    rows: &[&[u16; WIN]; WINDOW_ROWS],
    first_row: usize,
    output: [&mut [u16; W]; ROWS],
    init: [Simd<i32, W>; 3],
    odd: Mask<i16, W>,
    block: &GdfBlock,
    tap_weights: impl Fn(usize) -> GdfTapWeights<W>,
    low: Option<GdfLowRange<W>>,
) {
    const { assert!(WIN == W + 2 * TAP_REACH && ROWS <= 2) };
    let first_row = first_row.min(2 - ROWS);
    let centers: [Simd<i16, W>; ROWS] =
        core::array::from_fn(|row| tap_samples(rows[TAP_REACH + first_row + row], TAP_REACH));
    let mut sums = [init; ROWS];
    if let Some(low) = low.filter(|_| W == LOW_RANGE_LANES) {
        let add = add_taps::<W, WIN, ROWS, ZERO_WEIGHTS, false>;
        add(rows, first_row, &centers, &mut sums, &tap_weights);
        add_low_range_clips(rows, first_row, &centers, &mut sums, &tap_weights, &low);
    } else {
        let add = add_taps::<W, WIN, ROWS, ZERO_WEIGHTS, true>;
        add(rows, first_row, &centers, &mut sums, &tap_weights);
    }
    let swap = SWAPPED.then_some(odd);
    for (output, sums) in output.into_iter().zip(sums) {
        let base = Simd::from_array(*output);
        *output = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            finish_gdf_width_simd::<W, 8, 4096>(base, block, error, sums, swap)
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            finish_gdf_width_simd::<W, 5, 1000>(base, block, error, sums, swap)
        }
        .to_array();
    }
}

/// Adds every tap of the `ROWS` rows from row `first_row`. With `CLIP`, a tap
/// adds its clipped and clamped differences to the center; without it, the
/// tap adds `above + below`, and the caller subtracts the doubled center.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_taps<
    const W: usize,
    const WIN: usize,
    const ROWS: usize,
    const ZERO_WEIGHTS: u64,
    const CLIP: bool,
>(
    rows: &[&[u16; WIN]; WINDOW_ROWS],
    first_row: usize,
    centers: &[Simd<i16, W>; ROWS],
    sums: &mut [[Simd<i32, W>; 3]; ROWS],
    tap_weights: &impl Fn(usize) -> GdfTapWeights<W>,
) {
    for_each_gdf_tap!(K => {
        let (dy, dx) = GDF_COORDS[K];
        let tap = tap_weights(K);
        let left = (TAP_REACH as isize - dx) as usize;
        let right = (TAP_REACH as isize + dx) as usize;
        for (row, (sums, center)) in sums.iter_mut().zip(centers).enumerate() {
            let above = tap_samples(rows[TAP_REACH + first_row + row - dy as usize], left);
            let below = tap_samples(rows[TAP_REACH + first_row + row + dy as usize], right);
            let comb = if CLIP {
                let above = (above - center).simd_max(tap.low).simd_min(tap.alpha);
                let below = (below - center).simd_max(tap.low).simd_min(tap.alpha);
                (above + below)
                    .simd_max(Simd::splat(-512))
                    .simd_min(Simd::splat(511))
            } else {
                above + below
            }
            .cast::<i32>();
            for (index, (sum, weight)) in sums.iter_mut().zip(tap.weights).enumerate() {
                if ZERO_WEIGHTS >> (index * GDF_COORDS.len() + K) & 1 == 0 {
                    *sum += comb * weight.cast::<i32>();
                }
            }
        }
    });
}

/// Completes `add_taps` without `CLIP` for a low-range window: subtracts the
/// doubled center times the weight sums, and adds the change that the clip
/// makes for each tap whose bound is below the range.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_low_range_clips<const W: usize, const WIN: usize, const ROWS: usize>(
    rows: &[&[u16; WIN]; WINDOW_ROWS],
    first_row: usize,
    centers: &[Simd<i16, W>; ROWS],
    sums: &mut [[Simd<i32, W>; 3]; ROWS],
    tap_weights: &impl Fn(usize) -> GdfTapWeights<W>,
    low: &GdfLowRange<W>,
) {
    for (sums, center) in sums.iter_mut().zip(centers) {
        let scaled_center = (center + center).cast::<i32>() * Simd::splat(low.scale);
        for (sum, weight_sum) in sums.iter_mut().zip(low.weight_sums) {
            *sum -= scaled_center * weight_sum.cast::<i32>();
        }
    }
    for (alpha, k) in low.clip_taps.taps {
        if alpha >= low.range {
            break;
        }
        let k = usize::from(k).min(GDF_COORDS.len() - 1);
        let tap = tap_weights(k);
        let (dy, dx) = GDF_COORDS[k];
        let column = |dx: isize| ((TAP_REACH as isize + dx) as usize).min(WIN - W);
        for (row, center) in centers.iter().enumerate() {
            let center_row = TAP_REACH + first_row + row;
            let (Some(above), Some(below)) = (
                rows.get(center_row.wrapping_sub(dy as usize)),
                rows.get(center_row + dy as usize),
            ) else {
                continue;
            };
            let clip_change = |samples: Simd<i16, W>| {
                let diff = samples - center;
                diff.simd_max(tap.low).simd_min(tap.alpha) - diff
            };
            let change = clip_change(tap_samples(above, column(-dx)))
                + clip_change(tap_samples(below, column(dx)));
            for (sum, weight) in sums[row].iter_mut().zip(tap.weights) {
                *sum += change.cast::<i32>() * weight.cast::<i32>();
            }
        }
    }
}

/// Loads the class rows of taps `k` and `k ^ 1`, so the two taps share one load.
#[inline]
fn tap_pair<T: SimdElement>(table: &[[T; 4]], k: usize) -> Simd<T, 8> {
    Simd::from_slice(table[k & !1..(k & !1) + 2].as_flattened())
}

#[inline]
fn tap_samples<const W: usize, const WIN: usize>(row: &[u16; WIN], col: usize) -> Simd<i16, W> {
    Simd::<u16, W>::from_slice(&row[col..col + W]).cast()
}

/// Maps the biased gradient sums, already multiplied by `2 * SCALE`, to the
/// filtered sample; `swap` marks lanes whose first two sums are swapped.
fn finish_gdf_width_simd<const WIDTH: usize, const SCALE: i32, const ERROR_LEN: usize>(
    base: Simd<u16, WIDTH>,
    block: &GdfBlock,
    error: &[i32; ERROR_LEN],
    sums: [Simd<i32, WIDTH>; 3],
    swap: Option<Mask<i16, WIDTH>>,
) -> Simd<u16, WIDTH> {
    let digit_offset = Simd::splat((1 << 15) + (SCALE << 16));
    let [first, second, third] = sums.map(|sum| {
        ((sum + (sum >> 31) + digit_offset) >> 16)
            .cast::<i16>()
            .simd_clamp(Simd::splat(0), Simd::splat(2 * SCALE as i16 - 1))
    });
    let (first, second) = swap.map_or((first, second), |odd| {
        (odd.select(second, first), odd.select(first, second))
    });
    let radix = Simd::splat(2 * SCALE as i16);
    let pos = (first * radix + second) * radix + third;
    let error = Simd::gather_or_default(error, pos.cast::<usize>()).cast::<i16>();
    let scaled_error = error * Simd::splat(block.pix_scale as i16);
    let rounding = 12 - i16::from(block.bit_depth.bits());
    let residual = if rounding == 0 {
        scaled_error
    } else {
        (scaled_error + Simd::splat(1 << (rounding - 1)) + (scaled_error >> 15)) >> rounding
    };
    (base.cast::<i16>() + residual)
        .simd_max(Simd::splat(0))
        .simd_min(Simd::splat(block.max_sample as i16))
        .cast::<u16>()
}
