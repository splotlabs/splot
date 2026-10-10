// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Simd, SimdElement, ToBytes,
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

/// Clip bound and weights of one tap per lane.
pub(super) struct GdfTapWeights<const W: usize> {
    pub(super) alpha: Simd<i16, W>,
    pub(super) weights: [Simd<i16, W>; 3],
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

/// Filters in place a row pair of `W` samples whose class changes every two samples.
pub(super) fn mixed_class_rows<const W: usize, const WIN: usize>(
    window: &[&[u16; WIN]; WINDOW_ROWS],
    output: [&mut [u16; W]; 2],
    classes: &[GdfClass],
    block: &GdfBlock,
) {
    let weights = class_tap_weights::<W>(classes, block);
    gdf_rows::<W, WIN, 2, { zero_weight_taps(0b1111) }>(
        window,
        0,
        output,
        class_bias(classes),
        block,
        weights,
    );
}

/// Per-tap weights for `W` lanes whose class changes every two lanes.
fn class_tap_weights<const W: usize>(
    classes: &[GdfClass],
    block: &GdfBlock,
) -> impl Fn(usize) -> GdfTapWeights<W> {
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let indices = Simd::<i32, 4>::from_array(core::array::from_fn(|lane| {
        classes.get(lane).map_or(0, |class| class.0)
    }));
    let byte_offsets = Simd::from_array(core::array::from_fn(|byte| (byte & 1) as u8));
    let lane_bytes =
        ((indices & Simd::splat(3)) * Simd::splat(0x0202_0202)).to_ne_bytes() + byte_offsets;
    let lane_bytes = [lane_bytes, lane_bytes + Simd::splat(8)];
    let per_class = move |table: Simd<i16, 8>, k: usize| {
        let bytes = table.to_ne_bytes().swizzle_dyn(lane_bytes[k & 1]);
        Simd::<i16, 8>::from_ne_bytes(bytes).resize::<W>(0)
    };
    move |k| GdfTapWeights {
        alpha: per_class(tap_pair(alpha_table, k).cast(), k),
        weights: core::array::from_fn(|index| per_class(tap_pair(&weight_table[index], k), k)),
    }
}

/// Filters in place the `ROWS` rows of `W` samples in `output`, from row
/// `first_row` of the row pair whose source rows are `rows`; weights marked in
/// `ZERO_WEIGHTS` are skipped.
#[inline(never)]
pub(super) fn gdf_rows<
    const W: usize,
    const WIN: usize,
    const ROWS: usize,
    const ZERO_WEIGHTS: u64,
>(
    rows: &[&[u16; WIN]; WINDOW_ROWS],
    first_row: usize,
    output: [&mut [u16; W]; ROWS],
    class_bias: Simd<i32, W>,
    block: &GdfBlock,
    tap_weights: impl Fn(usize) -> GdfTapWeights<W>,
) {
    const { assert!(WIN == W + 2 * TAP_REACH && ROWS <= 2) };
    let first_row = first_row.min(2 - ROWS);
    let bias = &GDF_BIAS[block.ref_dst_idx][block.qp_idx];
    let gradient_bias = class_bias + Simd::splat(bias[2]);
    let centers: [Simd<i16, W>; ROWS] =
        core::array::from_fn(|row| tap_samples(rows[TAP_REACH + first_row + row], TAP_REACH));
    let mut sums = [[Simd::splat(bias[0]), Simd::splat(bias[1]), gradient_bias]; ROWS];
    for_each_gdf_tap!(K => {
        let (dy, dx) = GDF_COORDS[K];
        let tap = tap_weights(K);
        let low = -tap.alpha;
        let left = (TAP_REACH as isize - dx) as usize;
        let right = (TAP_REACH as isize + dx) as usize;
        for (row, center) in centers.iter().enumerate() {
            let above = tap_samples(rows[TAP_REACH + first_row + row - dy as usize], left);
            let below = tap_samples(rows[TAP_REACH + first_row + row + dy as usize], right);
            let above = (above - center).simd_max(low).simd_min(tap.alpha);
            let below = (below - center).simd_max(low).simd_min(tap.alpha);
            let comb = (above + below)
                .simd_max(Simd::splat(-512))
                .simd_min(Simd::splat(511))
                .cast::<i32>();
            for (index, (sum, weight)) in sums[row].iter_mut().zip(tap.weights).enumerate() {
                if ZERO_WEIGHTS >> (index * GDF_COORDS.len() + K) & 1 == 0 {
                    *sum += comb * weight.cast::<i32>();
                }
            }
        }
    });
    for (output, sums) in output.into_iter().zip(sums) {
        let base = Simd::from_array(*output);
        *output = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            finish_gdf_width_simd::<W, 8, 4096>(base, block, error, sums)
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            finish_gdf_width_simd::<W, 5, 1000>(base, block, error, sums)
        }
        .to_array();
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

/// Maps the biased gradient sums to the filtered sample. Round2Signed(v * 8, 15)
/// equals Round2Signed(v, 12), so the intra scale of 8 needs no multiply.
fn finish_gdf_width_simd<const WIDTH: usize, const SCALE: i32, const ERROR_LEN: usize>(
    base: Simd<u16, WIDTH>,
    block: &GdfBlock,
    error: &[i32; ERROR_LEN],
    gdf_idx: [Simd<i32, WIDTH>; 3],
) -> Simd<u16, WIDTH> {
    let shift = if SCALE == 8 { 12 } else { 15 };
    let digit_offset = Simd::splat((1 << (shift - 1)) + (SCALE << shift));
    let mut pos = Simd::<i16, WIDTH>::splat(0);
    for value in gdf_idx {
        let scaled = if SCALE == 8 {
            value
        } else {
            value * Simd::splat(SCALE)
        };
        let digit = ((scaled + digit_offset + (scaled >> 31)) >> shift)
            .cast::<i16>()
            .simd_clamp(Simd::splat(0), Simd::splat(2 * SCALE as i16 - 1));
        pos = pos * Simd::splat(2 * SCALE as i16) + digit;
    }
    let error = Simd::gather_or_default(error, pos.cast::<usize>()).cast::<i16>();
    let scaled_error = error * Simd::splat(block.pix_scale as i16);
    let rounding = 12 - i16::from(block.bit_depth.bits());
    let residual = if rounding == 0 {
        scaled_error
    } else {
        (scaled_error + Simd::splat(1 << (rounding - 1)) + (scaled_error >> 15)) >> rounding
    };
    (base.cast::<i16>() + residual)
        .simd_clamp(Simd::splat(0), Simd::splat(block.max_sample as i16))
        .cast::<u16>()
}
