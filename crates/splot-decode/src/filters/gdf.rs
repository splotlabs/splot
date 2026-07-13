// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.5 guided detail filter application.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_BIAS, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
use splot_parallel::prelude::*;
use splot_recon::math::{clip3, round2_signed};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, LoopRestorationSource, LoopRestorationSourceBounds, PlaneId,
    PlaneRect, ReconSample, loop_restoration_source_sample,
};

use crate::Result;
use crate::filters::wienerns_lr::wienerns_lr_selectable_transform_record_error_reason;

const MI_SIZE: usize = 4;
const GDF_TEST_STRIPE_OFF: usize = 8;
const GDF_TEST_STRIPE_SIZE: usize = 64;
const GDF_DIRECTIONS: usize = 4;
const GDF_GRADIENT_CAPACITY: usize = (MI_SIZE + 2) * (MI_SIZE + 2);
const GDF_CLASS_CAPACITY: usize = (MI_SIZE / 2) * (MI_SIZE / 2);
const GDF_COORDS: [(isize, isize); 18] = [
    (6, 0),
    (5, 0),
    (4, 0),
    (3, 0),
    (2, 1),
    (2, 0),
    (2, -1),
    (1, 2),
    (1, 1),
    (1, 0),
    (1, -1),
    (1, -2),
    (0, 6),
    (0, 5),
    (0, 4),
    (0, 3),
    (0, 2),
    (0, 1),
];
const GDF_READ_RADIUS: usize = 7;
const GDF_INTRA_REF_DST: usize = 0;
const RESTRICTED_ORDER_HINT: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GdfReferenceContext {
    current_order_hint: u32,
    ref_order_hints: [Option<u32>; 2],
}

impl GdfReferenceContext {
    pub(crate) fn from_reference_list(
        current_order_hint: u32,
        ref_frame_idx: &[u32],
        ref_order_hint: &[u32],
    ) -> Self {
        let mut ref_order_hints = [None; 2];
        for (list_ref, &slot) in ref_frame_idx.iter().take(2).enumerate() {
            ref_order_hints[list_ref] = usize::try_from(slot)
                .ok()
                .and_then(|slot| ref_order_hint.get(slot).copied());
        }
        Self {
            current_order_hint,
            ref_order_hints,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    core: &FrameHeaderCore,
    curr_luma: &[u16],
    cdef_luma: &[u16],
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
    reference: Option<GdfReferenceContext>,
    offset: ByteOffset,
) -> Result<()> {
    let Some(gdf) = core.gdf_params.as_ref().filter(|gdf| gdf.gdf_frame_enable) else {
        return Ok(());
    };
    if gdf.gdf_per_block != Some(false) {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_per_block",
        ));
    }
    let quant = core.quantization_params.as_ref().ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_quantization",
        )
    })?;
    let frame_is_intra = core.frame_is_intra.ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_frame_type",
        )
    })?;
    let qp_idx = gdf_qp_idx(
        quant.base_q_idx,
        bit_depth,
        frame_is_intra,
        gdf.gdf_pic_qc_idx,
        offset,
    )?;
    let pix_scale = i64::from(gdf.gdf_pic_scale_idx.unwrap_or(0)) + 1;
    let max_sample = i64::from(bit_depth.max_sample());
    let expected_samples = luma_width.checked_mul(luma_height).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    if curr_luma.len() != expected_samples || cdef_luma.len() != expected_samples {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        ));
    }
    let base_luma: Vec<u16> = workspace
        .samples(PlaneId::Y)
        .map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
            )
        })?
        .iter()
        .map(|sample| sample.to_u16())
        .collect();
    if base_luma.len() != expected_samples {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        ));
    }
    let ref_dst_idx = if frame_is_intra {
        GDF_INTRA_REF_DST
    } else {
        gdf_inter_ref_dst_idx(reference.ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_inter_reference",
            )
        })?)
    };

    let band_rows: Vec<_> = (0..luma_height).step_by(MI_SIZE).collect();
    let compute_band = |y: usize| -> Result<(usize, usize, Vec<T>)> {
        let height = MI_SIZE.min(luma_height - y);
        let mut band = vec![T::default(); luma_width * height];
        let band_block = GdfBlock {
            x: 0,
            y,
            width: luma_width,
            height,
            frame_width: luma_width,
            frame_height: luma_height,
            bit_depth,
            qp_idx,
            ref_dst_idx,
            pix_scale,
            max_sample,
        };
        let bounds = source_bounds(core, &band_block, offset)?;
        let source = GdfSource::materialize(curr_luma, cdef_luma, &bounds, &band_block, offset)?;
        for x in (0..luma_width).step_by(MI_SIZE) {
            let width = MI_SIZE.min(luma_width - x);
            if width < 2 || height < 2 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err(gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                ));
            }
            let mut block = compute_block(
                &source,
                &base_luma,
                GdfBlock {
                    x,
                    width,
                    ..band_block
                },
                offset,
            )?;
            preserve_lossless_luma_samples(
                lossless_grid,
                &base_luma,
                luma_width,
                x,
                y,
                width,
                height,
                &mut block,
                offset,
            )?;
            for row in 0..height {
                let src = &block[row * width..(row + 1) * width];
                let dst = &mut band[row * luma_width + x..row * luma_width + x + width];
                dst.copy_from_slice(src);
            }
        }
        Ok((y, height, band))
    };
    let bands = if splot_parallel::on_worker_pool() {
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let outputs = band_rows
            .par_iter()
            .map(|&y| {
                tally.note_worker();
                compute_band(y)
            })
            .collect::<Result<Vec<_>>>()?;
        crate::timing::report_detail(
            "gdf_bands",
            timer,
            &format!(
                "units={} threads={} workers_used={}",
                band_rows.len(),
                splot_parallel::current_pool_width(),
                tally.workers_used()
            ),
        );
        outputs
    } else {
        band_rows
            .iter()
            .map(|&y| compute_band(y))
            .collect::<Result<Vec<_>>>()?
    };
    for (y, height, band) in bands {
        let rect = PlaneRect::new(0, y, luma_width, height).map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_publish",
            )
        })?;
        workspace
            .write_rect(PlaneId::Y, rect, &band, luma_width)
            .map_err(|_| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_publish",
                )
            })?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct GdfBlock {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    frame_width: usize,
    frame_height: usize,
    bit_depth: BitDepth,
    qp_idx: usize,
    ref_dst_idx: usize,
    pix_scale: i64,
    max_sample: i64,
}

fn compute_block<T: ReconSample>(
    source: &GdfSource<T>,
    base_luma: &[u16],
    block: GdfBlock,
    offset: ByteOffset,
) -> Result<Vec<T>> {
    let grad = gradients(source, &block);
    let classes = classes(&grad, &block);
    let mut output = Vec::with_capacity(block.width * block.height);
    for row in 0..block.height {
        for col in 0..block.width {
            let class = &classes[(row >> 1) * (block.width >> 1) + (col >> 1)];
            let sample = gdf_sample(base_luma, source, &block, row, col, class);
            output.push(T::try_from_u16(sample).map_err(|_| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_sample",
                )
            })?);
        }
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn preserve_lossless_luma_samples<T: ReconSample>(
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    base_luma: &[u16],
    luma_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    output: &mut [T],
    offset: ByteOffset,
) -> Result<()> {
    let Some(lossless_grid) = lossless_grid else {
        return Ok(());
    };
    for row in 0..height {
        for col in 0..width {
            let sample_x = x + col;
            let sample_y = y + row;
            if !lossless_grid.plane_sample_lossless(PlaneId::Y, sample_x, sample_y, 0, 0) {
                continue;
            }
            let src = sample_y
                .checked_mul(luma_width)
                .and_then(|start| start.checked_add(sample_x))
                .and_then(|index| base_luma.get(index).copied())
                .ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                    )
                })?;
            let dst = row
                .checked_mul(width)
                .and_then(|start| start.checked_add(col))
                .and_then(|index| output.get_mut(index))
                .ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                    )
                })?;
            *dst = T::try_from_u16(src).map_err(|_| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_sample",
                )
            })?;
        }
    }
    Ok(())
}

fn source_bounds(
    core: &FrameHeaderCore,
    block: &GdfBlock,
    offset: ByteOffset,
) -> Result<LoopRestorationSourceBounds> {
    let mi_rows = block.frame_height.div_ceil(MI_SIZE);
    let mi_cols = block.frame_width.div_ceil(MI_SIZE);
    let row = block.y / MI_SIZE;
    let luma_y = row * MI_SIZE;
    let stripe_num = (luma_y + GDF_TEST_STRIPE_OFF) / GDF_TEST_STRIPE_SIZE;
    let stripe_start = stripe_num
        .checked_mul(GDF_TEST_STRIPE_SIZE)
        .and_then(|start| start.checked_sub(GDF_TEST_STRIPE_OFF))
        .unwrap_or(0);
    let stripe_end = stripe_num
        .checked_mul(GDF_TEST_STRIPE_SIZE)
        .and_then(|start| start.checked_add(GDF_TEST_STRIPE_SIZE - GDF_TEST_STRIPE_OFF - 1))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let luma_end_x = mi_cols
        .checked_mul(MI_SIZE)
        .and_then(|end| end.checked_sub(1))
        .map(|end| end.min(block.frame_width.saturating_sub(1)))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let luma_end_y = mi_rows
        .checked_mul(MI_SIZE)
        .and_then(|end| end.checked_sub(1))
        .map(|end| end.min(block.frame_height.saturating_sub(1)))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    if core
        .tile_info
        .as_ref()
        .is_some_and(|tile| tile.tile_cols != 1 || tile.tile_rows != 1)
    {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_multitile",
        ));
    }
    Ok(LoopRestorationSourceBounds {
        luma_start_x: 0,
        luma_end_x,
        luma_start_y: 0,
        luma_end_y,
        luma_stripe_start_y: stripe_start.min(luma_end_y),
        luma_stripe_end_y: stripe_end.min(luma_end_y),
        subsampling_x: 0,
        subsampling_y: 0,
    })
}

struct GdfSource<T> {
    samples: Vec<T>,
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

impl<T: ReconSample> GdfSource<T> {
    fn materialize(
        curr_luma: &[u16],
        cdef_luma: &[u16],
        bounds: &LoopRestorationSourceBounds,
        block: &GdfBlock,
        offset: ByteOffset,
    ) -> Result<Self> {
        let width = block
            .width
            .checked_add(GDF_READ_RADIUS * 2)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                )
            })?;
        let height = block
            .height
            .checked_add(GDF_READ_RADIUS * 2)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                )
            })?;
        let radius = isize::try_from(GDF_READ_RADIUS).map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
            )
        })?;
        let origin_x = isize::try_from(block.x).map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
            )
        })? - radius;
        let origin_y = isize::try_from(block.y).map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
            )
        })? - radius;
        let source_error = || {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
            )
        };
        let mut samples = Vec::with_capacity(width * height);
        for row in 0..height {
            let y = origin_y
                .checked_add(isize::try_from(row).map_err(|_| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                    )
                })?)
                .ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                    )
                })?;
            let left = loop_restoration_source_sample(PlaneId::Y, isize::MIN, y, bounds)
                .map_err(|_| source_error())?;
            let right = loop_restoration_source_sample(PlaneId::Y, isize::MAX, y, bounds)
                .map_err(|_| source_error())?;
            if right.x >= block.frame_width || left.y >= block.frame_height {
                return Err(source_error());
            }
            let source = match left.source {
                LoopRestorationSource::CurrFrame => curr_luma,
                LoopRestorationSource::CdefFrame => cdef_luma,
            };
            let row_start = left
                .y
                .checked_mul(block.frame_width)
                .ok_or_else(source_error)?;
            let row_end = row_start
                .checked_add(block.frame_width)
                .ok_or_else(source_error)?;
            let source_row = source.get(row_start..row_end).ok_or_else(source_error)?;
            for col in 0..width {
                let x = origin_x
                    .checked_add(isize::try_from(col).map_err(|_| {
                        gdf_filter_error(
                            offset,
                            "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                        )
                    })?)
                    .ok_or_else(|| {
                        gdf_filter_error(
                            offset,
                            "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                        )
                    })?;
                let x = x.clamp(left.x as isize, right.x as isize) as usize;
                let sample = source_row.get(x).copied().ok_or_else(source_error)?;
                samples.push(T::try_from_u16(sample).map_err(|_| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                    )
                })?);
            }
        }
        Ok(Self {
            samples,
            stride: width,
            origin_x,
            origin_y,
        })
    }

    fn get(&self, x: isize, y: isize) -> i64 {
        let col = x.saturating_sub(self.origin_x);
        let row = y.saturating_sub(self.origin_y);
        if col < 0 || row < 0 || col as usize >= self.stride {
            return 0;
        }
        self.samples
            .get((row as usize) * self.stride + col as usize)
            .map_or(0, |sample| i64::from(sample.to_u16()))
    }
}

fn gradients<T: ReconSample>(
    source: &GdfSource<T>,
    block: &GdfBlock,
) -> [[i64; GDF_GRADIENT_CAPACITY]; GDF_DIRECTIONS] {
    let rows = block.height + 2;
    let cols = block.width + 2;
    let mut grad = [[0_i64; GDF_GRADIENT_CAPACITY]; GDF_DIRECTIONS];
    for i in 0..rows {
        for j in 0..cols {
            let sample_x = block.x as isize - 1 + j as isize;
            let sample_y = block.y as isize - 1 + i as isize;
            for (direction, (delta_y, delta_x)) in
                [(1, 0), (0, 1), (1, 1), (-1, 1)].into_iter().enumerate()
            {
                let before = source.get(sample_x - delta_x, sample_y - delta_y);
                let center = source.get(sample_x, sample_y);
                let after = source.get(sample_x + delta_x, sample_y + delta_y);
                grad[direction][i * cols + j] = (center * 2 - before - after).abs();
            }
        }
    }
    grad
}

#[derive(Clone, Copy, Default)]
struct GdfClass {
    index: u8,
    strength_contribution: [i64; 3],
}

fn classes(
    grad: &[[i64; GDF_GRADIENT_CAPACITY]; GDF_DIRECTIONS],
    block: &GdfBlock,
) -> [GdfClass; GDF_CLASS_CAPACITY] {
    let class_cols = block.width >> 1;
    let class_rows = block.height >> 1;
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let strength_shift = if block.bit_depth == BitDepth::Eight {
        2
    } else {
        4
    };
    let mut classes = [GdfClass::default(); GDF_CLASS_CAPACITY];
    for i in (0..class_rows).rev() {
        for j in 0..class_cols {
            let mut strengths = [0_i64; GDF_DIRECTIONS];
            for direction in 0..GDF_DIRECTIONS {
                strengths[direction] =
                    grad_sum(&grad[direction], block.width + 2, i * 2, j * 2, 4, 4);
            }
            let index = u8::from(strengths[0] <= strengths[1])
                | (u8::from(strengths[2] <= strengths[3]) << 1);
            let cls = usize::from(index);
            let mut strength_contribution = [0_i64; 3];
            for (direction, strength) in strengths.into_iter().enumerate() {
                let k = GDF_COORDS.len() + direction;
                let alpha = i64::from(alpha_table[k][cls]);
                let comb = (strength >> strength_shift).min(alpha);
                for (idx, total) in strength_contribution.iter_mut().enumerate() {
                    *total += comb * i64::from(weight_table[idx][k][cls]);
                }
            }
            classes[i * class_cols + j] = GdfClass {
                index,
                strength_contribution,
            };
        }
    }
    classes
}

fn gdf_sample<T: ReconSample>(
    base_luma: &[u16],
    source: &GdfSource<T>,
    block: &GdfBlock,
    row: usize,
    col: usize,
    class: &GdfClass,
) -> u16 {
    let x = block.x as isize + col as isize;
    let y = block.y as isize + row as isize;
    let sample2 = source.get(x, y);
    let cls = usize::from(class.index);
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let mut gdf_idx = class.strength_contribution;
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    for (k, &(dy, dx)) in GDF_COORDS.iter().enumerate() {
        let alpha = i64::from(alpha_table[k][cls]);
        let sample3 = source.get(x - dx, y - dy);
        let sample4 = source.get(x + dx, y + dy);
        let above = clip3(-alpha, alpha, (sample3 - sample2) << shift);
        let below = clip3(-alpha, alpha, (sample4 - sample2) << shift);
        let comb = clip3(-512, 511, above + below);
        for (idx, total) in gdf_idx.iter_mut().enumerate() {
            *total += comb * i64::from(weight_table[idx][k][cls]);
        }
    }

    let scale = if block.ref_dst_idx == GDF_INTRA_REF_DST {
        8_i64
    } else {
        5_i64
    };
    let mut pos = 0_usize;
    for (idx, value) in gdf_idx.iter().enumerate() {
        let biased = (*value + i64::from(GDF_BIAS[block.ref_dst_idx][block.qp_idx][idx])) * scale;
        let v = round2_signed(biased, 15);
        let digit = clip3(-scale, scale - 1, v) + scale;
        pos = pos * (scale as usize * 2) + usize::try_from(digit).unwrap_or_default();
    }
    let err = if block.ref_dst_idx == GDF_INTRA_REF_DST {
        i64::from(GDF_INTRA_ERROR[block.qp_idx][pos])
    } else {
        i64::from(GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx][pos])
    };
    let residual = round2_signed(
        err * block.pix_scale,
        12 - u32::from(block.bit_depth.bits()),
    );
    let base_index = (block.y + row) * block.frame_width + block.x + col;
    let base = base_luma
        .get(base_index)
        .copied()
        .map(i64::from)
        .unwrap_or_default();
    clip3(0, block.max_sample, base + residual) as u16
}

fn grad_sum(
    grad: &[i64],
    stride: usize,
    row: usize,
    col: usize,
    down: usize,
    across: usize,
) -> i64 {
    let mut total = 0_i64;
    for y in row..row + down {
        for x in col..col + across {
            total += grad[y * stride + x];
        }
    }
    total
}

fn gdf_qp_idx(
    base_q_idx: u32,
    bit_depth: BitDepth,
    frame_is_intra: bool,
    pic_qc_idx: Option<u8>,
    offset: ByteOffset,
) -> Result<usize> {
    let qp_base = if frame_is_intra { 85_i32 } else { 110_i32 };
    let qp_diff = i32::try_from(base_q_idx).map_err(|_| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_quantization",
        )
    })? - qp_base
        - 24 * (i32::from(bit_depth.bits()) - 8);
    let qp_bucket = clip3(0, 2, i64::from((qp_diff - 37) / 25));
    let qc_idx = i64::from(pic_qc_idx.unwrap_or(0));
    usize::try_from(qp_bucket + qc_idx).map_err(|_| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_quantization",
        )
    })
}

fn gdf_inter_ref_dst_idx(reference: GdfReferenceContext) -> usize {
    let current = i32::try_from(reference.current_order_hint).unwrap_or(i32::MAX);
    let mut max_dist = 0usize;
    for raw_hint in reference.ref_order_hints.into_iter().flatten() {
        if raw_hint == RESTRICTED_ORDER_HINT {
            continue;
        }
        let hint = i32::try_from(raw_hint).unwrap_or(i32::MAX);
        let dist =
            usize::try_from(get_relative_dist(current, hint).unsigned_abs()).unwrap_or(usize::MAX);
        max_dist = max_dist.max(dist);
    }
    gdf_inter_ref_dst_idx_from_max_dist(max_dist)
}

const fn gdf_inter_ref_dst_idx_from_max_dist(max_dist: usize) -> usize {
    if max_dist == 0 {
        5
    } else if max_dist < 2 {
        1
    } else if max_dist < 3 {
        2
    } else if max_dist < 6 {
        3
    } else if max_dist < 11 {
        4
    } else {
        5
    }
}

fn get_relative_dist(a: i32, b: i32) -> i32 {
    (a - b).clamp(-127, 127)
}

fn gdf_filter_error(offset: ByteOffset, reason: &'static str) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(offset, reason)
}

#[cfg(test)]
mod tests {
    use super::{
        GDF_READ_RADIUS, GdfBlock, GdfReferenceContext, GdfSource, RESTRICTED_ORDER_HINT,
        gdf_inter_ref_dst_idx, gdf_inter_ref_dst_idx_from_max_dist, preserve_lossless_luma_samples,
    };
    use crate::filters::deblock::DeblockBlock;
    use crate::filters::lossless::LosslessBlockGrid;
    use splot_core::span::ByteOffset;
    use splot_recon::{BitDepth, LoopRestorationSourceBounds};

    #[test]
    fn band_source_matches_per_block_windows() {
        let frame_width = 16;
        let frame_height = 16;
        let curr: Vec<u16> = (0..frame_width * frame_height)
            .map(|index| index as u16)
            .collect();
        let cdef: Vec<u16> = curr.iter().map(|&sample| sample + 1_000).collect();
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: frame_width - 1,
            luma_start_y: 0,
            luma_end_y: frame_height - 1,
            luma_stripe_start_y: 4,
            luma_stripe_end_y: 11,
            subsampling_x: 0,
            subsampling_y: 0,
        };
        let band_block = GdfBlock {
            x: 0,
            y: 4,
            width: frame_width,
            height: 4,
            frame_width,
            frame_height,
            bit_depth: BitDepth::Ten,
            qp_idx: 0,
            ref_dst_idx: 0,
            pix_scale: 1,
            max_sample: 1_023,
        };
        let band_result =
            GdfSource::<u16>::materialize(&curr, &cdef, &bounds, &band_block, ByteOffset::new(0));
        assert!(band_result.is_ok());
        let Ok(band_source) = band_result else {
            return;
        };

        for x in (0..frame_width).step_by(4) {
            let block = GdfBlock {
                x,
                width: 4,
                ..band_block
            };
            let local_result =
                GdfSource::<u16>::materialize(&curr, &cdef, &bounds, &block, ByteOffset::new(0));
            assert!(local_result.is_ok());
            let Ok(local_source) = local_result else {
                return;
            };
            let radius = GDF_READ_RADIUS as isize;
            for y in block.y as isize - radius..block.y as isize + block.height as isize + radius {
                for x in block.x as isize - radius..block.x as isize + block.width as isize + radius
                {
                    assert_eq!(band_source.get(x, y), local_source.get(x, y));
                }
            }
        }
    }

    #[test]
    fn inter_ref_dst_idx_uses_first_two_reference_list_entries() {
        let context = GdfReferenceContext::from_reference_list(10, &[2, 4, 1], &[0, 6, 7, 8, 3]);

        assert_eq!(gdf_inter_ref_dst_idx(context), 4);
    }

    #[test]
    fn inter_ref_dst_idx_maps_zero_or_restricted_distance_to_far_bucket() {
        let same = GdfReferenceContext::from_reference_list(10, &[0], &[10, RESTRICTED_ORDER_HINT]);
        let restricted =
            GdfReferenceContext::from_reference_list(10, &[1], &[10, RESTRICTED_ORDER_HINT]);

        assert_eq!(gdf_inter_ref_dst_idx(same), 5);
        assert_eq!(gdf_inter_ref_dst_idx(restricted), 5);
    }

    #[test]
    fn inter_ref_dst_idx_bucket_boundaries_match_spec() {
        let cases = [
            (0, 5),
            (1, 1),
            (2, 2),
            (3, 3),
            (5, 3),
            (6, 4),
            (10, 4),
            (11, 5),
        ];
        for (case, (max_dist, expected)) in cases.into_iter().enumerate() {
            let actual = gdf_inter_ref_dst_idx_from_max_dist(max_dist);
            assert!(
                actual == expected,
                "case {case}: distance {max_dist} mapped to {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn preserve_lossless_luma_samples_keeps_only_lossless_cells() {
        let lossless = [DeblockBlock {
            r: 1,
            c: 1,
            luma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: 1,
                base_c: 1,
                default_sub_pu_tx: 0,
            },
            chroma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: 1,
                base_c: 1,
                default_sub_pu_tx: 0,
            },
            chroma_base_r: 1,
            chroma_base_c: 1,
            n4w: 1,
            n4h: 1,
            luma_tx: 0,
            chroma_tx: Some(0),
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex: 0,
            skip: false,
            lossless: true,
        }];
        let grid = LosslessBlockGrid::from_deblock_blocks(4, 4, &lossless, [&[], &[]]);
        assert!(grid.is_ok());
        let Ok(grid) = grid else {
            return;
        };
        let base: Vec<u16> = (0..64).collect();
        let mut output = vec![200_u8; 32];

        let result = preserve_lossless_luma_samples(
            Some(&grid),
            &base,
            8,
            4,
            4,
            8,
            4,
            &mut output,
            ByteOffset::new(0),
        );
        assert!(result.is_ok());

        for row in 0..4 {
            for col in 0..8 {
                let expected = if col < 4 {
                    base[(4 + row) * 8 + 4 + col] as u8
                } else {
                    200
                };
                assert_eq!(output[row * 8 + col], expected, "row {row} col {col}");
            }
        }
    }
}
