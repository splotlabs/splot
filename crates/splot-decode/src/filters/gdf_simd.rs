// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Simd,
    cmp::{SimdOrd, SimdPartialEq},
    num::{SimdInt, SimdUint},
};

use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};

use crate::Result;

use super::{
    GDF_BIAS, GDF_COORDS, GDF_INTRA_REF_DST, GdfBlock, GdfClass, GdfSource, MI_SIZE, exact_slice,
    gdf_state_error,
};

#[inline]
pub(super) fn uniform_gdf_class<const LANES: usize>(classes: &[GdfClass; LANES]) -> Option<u8> {
    let indices = Simd::<i32, LANES>::from_array(classes.map(|class| class.0)) & Simd::splat(3);
    let first = indices[0];
    indices
        .simd_eq(Simd::splat(first))
        .all()
        .then_some(first as u8)
}

pub(super) fn gdf_width8_rows<const ROWS: usize>(
    base_values: [[u16; 8]; ROWS],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    classes: [GdfClass; 4],
    block: &GdfBlock,
    source_origin: (usize, usize),
) -> Result<[[u16; 8]; ROWS]> {
    let source_error = gdf_state_error;
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    let class_indices = classes.map(|class| usize::from(class.index()));
    let gradient_bias = Simd::from_array(core::array::from_fn(|lane| {
        classes[lane >> 1].gradient_bias()
    }));
    let mut bases = [0usize; ROWS];
    let mut centers = [Simd::<i16, 8>::splat(0); ROWS];
    let mut gdf_indices = [[Simd::<i32, 8>::splat(0); 3]; ROWS];
    let mut output = [[0; 8]; ROWS];
    for row_offset in 0..ROWS {
        let base = (source_origin.1 + row_offset) * source.stride + source_origin.0;
        bases[row_offset] = base;
        centers[row_offset] = Simd::<u16, 8>::from_slice(
            exact_slice(source.samples, base, 8).ok_or_else(source_error)?,
        )
        .cast::<i16>();
        gdf_indices[row_offset][2] = gradient_bias;
    }
    for (k, &tap) in tap_offsets.iter().enumerate() {
        let alpha = Simd::from_array(core::array::from_fn(|lane| {
            alpha_table[k][class_indices[lane >> 1]] as i16
        }));
        let low = -alpha;
        for row_offset in 0..ROWS {
            let base = bases[row_offset];
            let negative = Simd::<u16, 8>::from_slice(
                exact_slice(source.samples, base - tap, 8).ok_or_else(source_error)?,
            )
            .cast::<i16>();
            let positive = Simd::<u16, 8>::from_slice(
                exact_slice(source.samples, base + tap, 8).ok_or_else(source_error)?,
            )
            .cast::<i16>();
            let above = ((negative - centers[row_offset]) << shift as i16).simd_clamp(low, alpha);
            let below = ((positive - centers[row_offset]) << shift as i16).simd_clamp(low, alpha);
            let comb = (above + below)
                .simd_clamp(Simd::splat(-512), Simd::splat(511))
                .cast::<i32>();
            for (index, weights) in gdf_indices[row_offset].iter_mut().zip(weight_table) {
                *index += comb
                    * Simd::from_array(core::array::from_fn(|lane| {
                        i32::from(weights[k][class_indices[lane >> 1]])
                    }));
            }
        }
    }
    for (row_offset, gdf_idx) in gdf_indices.into_iter().enumerate() {
        output[row_offset] = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            finish_gdf_width_simd::<8, 8, 4096>(
                Simd::from_array(base_values[row_offset]).cast::<i32>(),
                block,
                error,
                gdf_idx,
            )
            .to_array()
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            finish_gdf_width_simd::<8, 5, 1000>(
                Simd::from_array(base_values[row_offset]).cast::<i32>(),
                block,
                error,
                gdf_idx,
            )
            .to_array()
        };
    }
    Ok(output)
}

pub(super) fn gdf_width4_rows<const ROWS: usize>(
    base_values: [[u16; MI_SIZE]; ROWS],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    classes: [GdfClass; 2],
    block: &GdfBlock,
    row: usize,
    source_origin: (usize, usize),
) -> Result<[[u16; MI_SIZE]; ROWS]> {
    let source_error = gdf_state_error;
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    let [class0, class1] = classes.map(|class| usize::from(class.index()));
    let [bias0, bias1] = classes.map(GdfClass::gradient_bias);
    let gradient_bias = Simd::from_array([bias0, bias0, bias1, bias1]);
    let mut bases = [0usize; ROWS];
    let mut centers = [Simd::<i32, MI_SIZE>::splat(0); ROWS];
    let mut gdf_indices = [[Simd::<i32, MI_SIZE>::splat(0); 3]; ROWS];
    let mut output = [[0; MI_SIZE]; ROWS];
    for row_offset in 0..ROWS {
        let base = (source_origin.1 + row + row_offset) * source.stride + source_origin.0;
        bases[row_offset] = base;
        centers[row_offset] = Simd::<u16, MI_SIZE>::from_slice(
            exact_slice(source.samples, base, MI_SIZE).ok_or_else(source_error)?,
        )
        .cast::<i32>();
        gdf_indices[row_offset][2] = gradient_bias;
    }
    for (k, &tap) in tap_offsets.iter().enumerate() {
        let alpha = Simd::from_array([
            i32::from(alpha_table[k][class0]),
            i32::from(alpha_table[k][class0]),
            i32::from(alpha_table[k][class1]),
            i32::from(alpha_table[k][class1]),
        ]);
        let per_class = |table: &[[i16; 4]; 22]| {
            Simd::from_array([
                i32::from(table[k][class0]),
                i32::from(table[k][class0]),
                i32::from(table[k][class1]),
                i32::from(table[k][class1]),
            ])
        };
        let weights: [Simd<i32, MI_SIZE>; 3] =
            core::array::from_fn(|index| per_class(&weight_table[index]));
        for row_offset in 0..ROWS {
            let base = bases[row_offset];
            let negative = Simd::<u16, MI_SIZE>::from_slice(
                exact_slice(source.samples, base - tap, MI_SIZE).ok_or_else(source_error)?,
            )
            .cast::<i32>();
            let positive = Simd::<u16, MI_SIZE>::from_slice(
                exact_slice(source.samples, base + tap, MI_SIZE).ok_or_else(source_error)?,
            )
            .cast::<i32>();
            let above =
                ((negative - centers[row_offset]) << shift as i32).simd_clamp(-alpha, alpha);
            let below =
                ((positive - centers[row_offset]) << shift as i32).simd_clamp(-alpha, alpha);
            let comb = (above + below).simd_clamp(Simd::splat(-512), Simd::splat(511));
            for (index, weight) in gdf_indices[row_offset].iter_mut().zip(weights) {
                *index += comb * weight;
            }
        }
    }
    for (row_offset, gdf_idx) in gdf_indices.into_iter().enumerate() {
        output[row_offset] = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            finish_gdf_width_simd::<MI_SIZE, 8, 4096>(
                Simd::from_array(base_values[row_offset]).cast::<i32>(),
                block,
                error,
                gdf_idx,
            )
            .to_array()
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            finish_gdf_width_simd::<MI_SIZE, 5, 1000>(
                Simd::from_array(base_values[row_offset]).cast::<i32>(),
                block,
                error,
                gdf_idx,
            )
            .to_array()
        };
    }
    Ok(output)
}

pub(super) fn finish_gdf_width_simd<
    const WIDTH: usize,
    const SCALE: i32,
    const ERROR_LEN: usize,
>(
    base: Simd<i32, WIDTH>,
    block: &GdfBlock,
    error: &[i32; ERROR_LEN],
    gdf_idx: [Simd<i32, WIDTH>; 3],
) -> Simd<u16, WIDTH> {
    let mut pos = Simd::<u16, WIDTH>::splat(0);
    for (idx, value) in gdf_idx.into_iter().enumerate() {
        let biased = (value + Simd::splat(GDF_BIAS[block.ref_dst_idx][block.qp_idx][idx]))
            * Simd::splat(SCALE);
        let digit = round2_signed_simd(biased, 15)
            .simd_clamp(Simd::splat(-SCALE), Simd::splat(SCALE - 1))
            + Simd::splat(SCALE);
        pos = pos * Simd::splat((SCALE * 2) as u16) + digit.cast::<u16>();
    }
    let scaled_error =
        Simd::gather_or_default(error, pos.cast::<usize>()) * Simd::splat(block.pix_scale);
    let residual = round2_signed_simd(scaled_error, 12 - u32::from(block.bit_depth.bits()));
    (base + residual)
        .simd_clamp(Simd::splat(0), Simd::splat(block.max_sample))
        .cast::<u16>()
}

fn round2_signed_simd<const WIDTH: usize>(value: Simd<i32, WIDTH>, shift: u32) -> Simd<i32, WIDTH> {
    if shift == 0 {
        return value;
    }
    (value + Simd::splat(1 << (shift - 1)) + (value >> Simd::splat(31)))
        >> Simd::splat(shift as i32)
}
