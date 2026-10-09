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

const TAP_REACH: usize = GDF_READ_RADIUS - 1;

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

/// Filters a row pair of `W` samples whose class changes every two samples.
pub(super) fn mixed_class_rows<const W: usize>(
    base_values: [[u16; W]; 2],
    source: &GdfSource<'_>,
    classes: &[GdfClass],
    block: &GdfBlock,
    origin: (usize, usize),
) -> Result<[[u16; W]; 2]> {
    let weights = class_tap_weights::<W>(classes, block);
    gdf_rows::<W, { zero_weight_taps(0b1111) }>(
        base_values,
        source,
        classes,
        block,
        origin,
        weights,
    )
}

/// Per-tap weights for `W` lanes whose class changes every two lanes.
fn class_tap_weights<const W: usize>(
    classes: &[GdfClass],
    block: &GdfBlock,
) -> impl Fn(usize) -> GdfTapWeights<W> {
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let lane_bytes = Simd::<u8, 16>::from_array(core::array::from_fn(|byte| {
        classes.get(byte >> 2).map_or(0, |class| class.index() * 2) + (byte & 1) as u8
    }));
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

/// Filters two rows of `W` samples starting at `origin` in `source`; weights
/// marked in `ZERO_WEIGHTS` are skipped.
#[inline(never)]
pub(super) fn gdf_rows<const W: usize, const ZERO_WEIGHTS: u64>(
    base_values: [[u16; W]; 2],
    source: &GdfSource<'_>,
    classes: &[GdfClass],
    block: &GdfBlock,
    origin: (usize, usize),
    tap_weights: impl Fn(usize) -> GdfTapWeights<W>,
) -> Result<[[u16; W]; 2]> {
    let first_row = origin.1.checked_sub(TAP_REACH);
    let first_col = origin.0.checked_sub(TAP_REACH);
    let (Some(first_row), Some(first_col)) = (first_row, first_col) else {
        return Err(gdf_state_error());
    };
    if classes.len() < W / 2 {
        return Err(gdf_state_error());
    }
    let mut rows = [&source.samples[..0]; 2 * TAP_REACH + 2];
    let window = W + 2 * TAP_REACH;
    let first = first_row
        .checked_mul(source.stride)
        .and_then(|start| start.checked_add(first_col))
        .filter(|&first| {
            (rows.len() - 1)
                .checked_mul(source.stride)
                .and_then(|offset| offset.checked_add(first + window))
                .is_some_and(|end| end <= source.samples.len())
        })
        .ok_or_else(gdf_state_error)?;
    for (index, row) in rows.iter_mut().enumerate() {
        let start = first + index * source.stride;
        *row = exact_slice(source.samples, start, window).ok_or_else(gdf_state_error)?;
    }
    let bias = &GDF_BIAS[block.ref_dst_idx][block.qp_idx];
    let gradient_bias = Simd::from_array(core::array::from_fn(|lane| {
        classes[lane >> 1].gradient_bias() + bias[2]
    }));
    let centers: [Simd<i16, W>; 2] =
        core::array::from_fn(|row| tap_samples(rows[TAP_REACH + row], TAP_REACH));
    let mut sums = [[Simd::splat(bias[0]), Simd::splat(bias[1]), gradient_bias]; 2];
    for_each_gdf_tap!(K => {
        let (dy, dx) = GDF_COORDS[K];
        let tap = tap_weights(K);
        let low = -tap.alpha;
        let left = (TAP_REACH as isize - dx) as usize;
        let right = (TAP_REACH as isize + dx) as usize;
        for (row, center) in centers.iter().enumerate() {
            let above = tap_samples(rows[TAP_REACH + row - dy as usize], left);
            let below = tap_samples(rows[TAP_REACH + row + dy as usize], right);
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
    let mut output = [[0; W]; 2];
    for ((output, base), sums) in output.iter_mut().zip(base_values).zip(sums) {
        let base = Simd::from_array(base);
        *output = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            finish_gdf_width_simd::<W, 8, 4096>(base, block, error, sums)
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            finish_gdf_width_simd::<W, 5, 1000>(base, block, error, sums)
        }
        .to_array();
    }
    Ok(output)
}

/// Loads the class rows of taps `k` and `k ^ 1`, so the two taps share one load.
#[inline]
fn tap_pair<T: SimdElement>(table: &[[T; 4]], k: usize) -> Simd<T, 8> {
    Simd::from_slice(table[k & !1..(k & !1) + 2].as_flattened())
}

#[inline]
fn tap_samples<const W: usize>(row: &[u16], col: usize) -> Simd<i16, W> {
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
