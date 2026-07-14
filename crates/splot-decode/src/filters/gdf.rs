// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.5 guided detail filter application.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_BIAS, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
use splot_parallel::prelude::*;
use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, LoopRestorationSource, LoopRestorationSourceBounds, PlaneId,
    ReconSample, loop_restoration_source_sample,
};

use crate::Result;
use crate::filters::wienerns_lr::wienerns_lr_selectable_transform_record_error_reason;
use crate::support::reusable_scratch::with_reusable_scratch;

const MI_SIZE: usize = 4;
const GDF_TEST_STRIPE_OFF: usize = 8;
const GDF_TEST_STRIPE_SIZE: usize = 64;
const GDF_DIRECTIONS: usize = 4;
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
const GDF_SCRATCH_ALLOCATION_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_gdf_allocation";

#[derive(Default)]
struct GdfScratch {
    source: Vec<u16>,
    gradients: Vec<u16>,
    classes: Vec<GdfClass>,
    base_luma: Vec<u16>,
}

std::thread_local! {
    static GDF_SCRATCH: std::cell::RefCell<GdfScratch> =
        std::cell::RefCell::new(GdfScratch::default());
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GdfBlockGrid {
    block_size: usize,
    rows: usize,
    cols: usize,
    values: Vec<u8>,
}

impl GdfBlockGrid {
    pub(crate) fn new(
        block_size: usize,
        rows: usize,
        cols: usize,
        values: Vec<u8>,
    ) -> core::result::Result<Self, ()> {
        let expected = rows.checked_mul(cols).ok_or(())?;
        if block_size == 0
            || !block_size.is_multiple_of(MI_SIZE)
            || rows == 0
            || cols == 0
            || values.len() != expected
            || values.iter().any(|&value| value > 1)
        {
            return Err(());
        }
        Ok(Self {
            block_size,
            rows,
            cols,
            values,
        })
    }

    fn enabled(&self, stripe_row: usize, x: usize) -> Option<bool> {
        let row = stripe_row.checked_mul(MI_SIZE)? / self.block_size;
        let col = x / self.block_size;
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.values.get(index).map(|&value| value != 0)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    core: &FrameHeaderCore,
    curr_luma: &[u16],
    cdef_luma: &[u16],
    block_grid: Option<&GdfBlockGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
    disable_loopfilters_across_tiles: bool,
    reference: Option<GdfReferenceContext>,
    offset: ByteOffset,
) -> Result<()> {
    let Some(gdf) = core.gdf_params.as_ref().filter(|gdf| gdf.gdf_frame_enable) else {
        return Ok(());
    };
    let Some(per_block) = gdf.gdf_per_block else {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_per_block",
        ));
    };
    if per_block && block_grid.is_none() {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_grid",
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
    let pix_scale = i32::from(gdf.gdf_pic_scale_idx.unwrap_or(0)) + 1;
    let max_sample = i32::from(bit_depth.max_sample());
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

    let band_segments = if disable_loopfilters_across_tiles {
        gdf_band_segments(core, luma_width, offset)?
    } else {
        vec![(0, luma_width)]
    };
    let (luma, _, _) = workspace.as_frame_mut().into_planes();
    let stride = luma.stride_samples();
    if stride != luma_width || luma.samples().len() != expected_samples {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_publish",
        ));
    }
    let band_len = stride
        .checked_mul(MI_SIZE)
        .filter(|&len| len != 0)
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let band_count = luma_height.div_ceil(MI_SIZE);
    let compute_band = |scratch: &mut GdfScratch,
                        band_index: usize,
                        band: &mut [T]|
     -> Result<()> {
        let y = band_index * MI_SIZE;
        let height = MI_SIZE.min(luma_height - y);
        let stripe_row = gdf_stripe_row(core, y, luma_height, offset)?;

        let band_base_luma = &mut scratch.base_luma;
        resize_scratch(band_base_luma, band.len(), offset)?;
        for (dst, src) in band_base_luma.iter_mut().zip(band.iter()) {
            *dst = src.to_u16();
        }

        for &(segment_x, segment_width) in &band_segments {
            let band_block = GdfBlock {
                x: segment_x,
                y,
                width: segment_width,
                height,
                frame_width: luma_width,
                frame_height: luma_height,
                bit_depth,
                qp_idx,
                ref_dst_idx,
                pix_scale,
                max_sample,
            };
            let bounds =
                source_bounds(core, &band_block, disable_loopfilters_across_tiles, offset)?;
            let source = GdfSource::materialize::<T>(
                &mut scratch.source,
                curr_luma,
                cdef_luma,
                &bounds,
                &band_block,
                offset,
            )?;
            let band_origin = source.relative_position(segment_x, y).ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                )
            })?;
            let grad_cols = segment_width + 2;
            band_gradients(
                &source,
                band_origin,
                height + 2,
                grad_cols,
                &mut scratch.gradients,
                offset,
            )?;
            band_classes(
                &scratch.gradients,
                grad_cols,
                &band_block,
                &mut scratch.classes,
                offset,
            )?;
            let segment_end = segment_x.checked_add(segment_width).ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?;
            for x in (segment_x..segment_end).step_by(MI_SIZE) {
                let width = MI_SIZE.min(segment_end - x);
                if width < 2 || height < 2 || !width.is_multiple_of(2) || !height.is_multiple_of(2)
                {
                    return Err(gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                    ));
                }
                let block_enabled = if per_block {
                    block_grid
                        .and_then(|grid| grid.enabled(stripe_row, x))
                        .ok_or_else(|| {
                            gdf_filter_error(
                                offset,
                                "unsupported_wienerns_lr_selectable_transform_records_gdf_grid",
                            )
                        })?
                } else {
                    true
                };
                let mut block = if block_enabled {
                    compute_block(
                        &source,
                        band_base_luma,
                        &scratch.classes,
                        segment_width >> 1,
                        GdfBlock {
                            x,
                            width,
                            ..band_block
                        },
                        offset,
                    )?
                } else {
                    let copied =
                        copy_base_block::<T>(band_base_luma, stride, x, 0, width, height, offset)?;
                    let mut block = [T::default(); MI_SIZE * MI_SIZE];
                    block[..copied.len()].copy_from_slice(&copied);
                    block
                };
                preserve_lossless_luma_samples(
                    lossless_grid,
                    band_base_luma,
                    luma_width,
                    x,
                    y,
                    0,
                    width,
                    height,
                    &mut block,
                    offset,
                )?;
                for row in 0..height {
                    let src = &block[row * width..(row + 1) * width];
                    let dst = &mut band[row * stride + x..row * stride + x + width];
                    dst.copy_from_slice(src);
                }
            }
        }
        Ok(())
    };
    let luma_samples = luma.into_samples();
    if splot_parallel::on_worker_pool() {
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let result = luma_samples
            .par_chunks_mut(band_len)
            .enumerate()
            .try_for_each(|(band_index, band)| {
                tally.note_worker();
                with_reusable_scratch(&GDF_SCRATCH, |scratch| {
                    compute_band(scratch, band_index, band)
                })
            });
        crate::timing::report_detail(
            "gdf_bands",
            timer,
            &format!(
                "units={} threads={} workers_used={}",
                band_count,
                splot_parallel::current_pool_width(),
                tally.workers_used()
            ),
        );
        result
    } else {
        with_reusable_scratch(&GDF_SCRATCH, |scratch| {
            luma_samples
                .chunks_mut(band_len)
                .enumerate()
                .try_for_each(|(band_index, band)| compute_band(scratch, band_index, band))
        })
    }
}

fn gdf_stripe_row(
    core: &FrameHeaderCore,
    y: usize,
    luma_height: usize,
    offset: ByteOffset,
) -> Result<usize> {
    let frame_end_y = luma_height.checked_sub(1).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let tile_end_y = core.tile_info.as_ref().map_or(Ok(frame_end_y), |tile| {
        tile_axis_bounds(&tile.mi_row_starts, y, luma_height, offset).map(|(_, end)| end)
    })?;
    gdf_stripe_row_for_tile_end(y, tile_end_y, offset)
}

fn gdf_stripe_row_for_tile_end(y: usize, tile_end_y: usize, offset: ByteOffset) -> Result<usize> {
    let row = y / MI_SIZE;
    let stripe_row = row
        .checked_add(2)
        .map(|value| (value >> 4) << 4)
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    Ok(stripe_row.min(tile_end_y / MI_SIZE))
}

#[allow(clippy::too_many_arguments)]
fn copy_base_block<T: ReconSample>(
    base_luma: &[u16],
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    offset: ByteOffset,
) -> Result<Vec<T>> {
    let capacity = width.checked_mul(height).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let mut output = Vec::with_capacity(capacity);
    for row in 0..height {
        let start = y
            .checked_add(row)
            .and_then(|sample_y| sample_y.checked_mul(stride))
            .and_then(|row_start| row_start.checked_add(x))
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?;
        let end = start.checked_add(width).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        let samples = base_luma.get(start..end).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
            )
        })?;
        for &sample in samples {
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
    pix_scale: i32,
    max_sample: i32,
}

fn compute_block<T: ReconSample>(
    source: &GdfSource<'_>,
    base_luma: &[u16],
    classes: &[GdfClass],
    class_cols: usize,
    block: GdfBlock,
    offset: ByteOffset,
) -> Result<[T; MI_SIZE * MI_SIZE]> {
    let source_origin = source.relative_position(block.x, block.y).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    })?;
    if source_origin.0 < GDF_READ_RADIUS || source_origin.1 < GDF_READ_RADIUS {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        ));
    }
    let stride = source.stride;
    let tap_offsets = gdf_tap_offsets(stride, offset)?;
    let last_col = source_origin
        .0
        .checked_add(block.width)
        .and_then(|value| value.checked_add(GDF_READ_RADIUS - 1))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let window_end = source_origin
        .1
        .checked_add(block.height)
        .and_then(|value| value.checked_add(GDF_READ_RADIUS - 1))
        .and_then(|last_row| last_row.checked_mul(stride))
        .and_then(|prefix| prefix.checked_add(last_col))
        .and_then(|last_index| last_index.checked_add(1))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    if last_col >= stride || window_end > source.samples.len() {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        ));
    }
    let class_col_base = (source_origin.0 - GDF_READ_RADIUS) >> 1;
    let mut output = [T::default(); MI_SIZE * MI_SIZE];
    if block.width == MI_SIZE {
        for row in 0..block.height {
            let class_base = (row >> 1) * class_cols + class_col_base;
            let samples = gdf_width4_row(
                base_luma,
                source,
                &tap_offsets,
                [classes[class_base], classes[class_base + 1]],
                &block,
                row,
                source_origin,
                offset,
            )?;
            for (col, sample) in samples.into_iter().enumerate() {
                output[row * MI_SIZE + col] = T::try_from_u16(sample).map_err(|_| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_sample",
                    )
                })?;
            }
        }
        return Ok(output);
    }
    for row in 0..block.height {
        for col in 0..block.width {
            let class = &classes[(row >> 1) * class_cols + class_col_base + (col >> 1)];
            let sample = gdf_sample(
                base_luma,
                source,
                &tap_offsets,
                &block,
                row,
                col,
                (source_origin.0 + col, source_origin.1 + row),
                class,
            );
            output[row * block.width + col] = T::try_from_u16(sample).map_err(|_| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_sample",
                )
            })?;
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
    y_frame: usize,
    y_band: usize,
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
            let sample_y_frame = y_frame + row;
            if !lossless_grid.plane_sample_lossless(PlaneId::Y, sample_x, sample_y_frame, 0, 0) {
                continue;
            }
            let sample_y_band = y_band + row;
            let src = sample_y_band
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

fn gdf_band_segments(
    core: &FrameHeaderCore,
    luma_width: usize,
    offset: ByteOffset,
) -> Result<Vec<(usize, usize)>> {
    let Some(tile) = core.tile_info.as_ref() else {
        return Ok(vec![(0, luma_width)]);
    };
    let tile_cols = usize::try_from(tile.tile_cols).map_err(|_| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    if tile.mi_col_starts.len() != tile_cols.saturating_add(1) || luma_width == 0 {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    let mut segments: Vec<(usize, usize)> = Vec::with_capacity(tile_cols);
    for window in tile.mi_col_starts.windows(2) {
        let start = usize::try_from(window[0])
            .ok()
            .and_then(|value| value.checked_mul(MI_SIZE))
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?
            .min(luma_width);
        let end = usize::try_from(window[1])
            .ok()
            .and_then(|value| value.checked_mul(MI_SIZE))
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?
            .min(luma_width);
        let width = end
            .checked_sub(start)
            .filter(|&width| width != 0)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?;
        if segments
            .last()
            .is_some_and(|&(previous_x, previous_width)| {
                previous_x.checked_add(previous_width) != Some(start)
            })
        {
            return Err(gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            ));
        }
        segments.push((start, width));
    }
    if segments.first().map(|&(x, _)| x) != Some(0)
        || segments.last().and_then(|&(x, width)| x.checked_add(width)) != Some(luma_width)
    {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    Ok(segments)
}

fn tile_axis_bounds(
    starts: &[u32],
    position: usize,
    frame_extent: usize,
    offset: ByteOffset,
) -> Result<(usize, usize)> {
    if position >= frame_extent {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    let mi = position / MI_SIZE;
    let window = starts
        .windows(2)
        .find(|window| {
            usize::try_from(window[0]).is_ok_and(|start| start <= mi)
                && usize::try_from(window[1]).is_ok_and(|end| mi < end)
        })
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let start = usize::try_from(window[0])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let end = usize::try_from(window[1])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .map(|end| end.min(frame_extent))
        .and_then(|end| end.checked_sub(1))
        .filter(|&end| position >= start && position <= end)
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    Ok((start, end))
}

fn source_bounds(
    core: &FrameHeaderCore,
    block: &GdfBlock,
    disable_loopfilters_across_tiles: bool,
    offset: ByteOffset,
) -> Result<LoopRestorationSourceBounds> {
    let frame_end_x = block.frame_width.checked_sub(1).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let frame_end_y = block.frame_height.checked_sub(1).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let (tile_start_x, tile_end_x, tile_start_y, tile_end_y) =
        if let Some(tile) = core.tile_info.as_ref() {
            let (start_x, end_x) =
                tile_axis_bounds(&tile.mi_col_starts, block.x, block.frame_width, offset)?;
            let (start_y, end_y) =
                tile_axis_bounds(&tile.mi_row_starts, block.y, block.frame_height, offset)?;
            (start_x, end_x, start_y, end_y)
        } else {
            (0, frame_end_x, 0, frame_end_y)
        };
    let local_y = block.y.checked_sub(tile_start_y).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let stripe_num = local_y
        .checked_add(GDF_TEST_STRIPE_OFF)
        .map(|value| value / GDF_TEST_STRIPE_SIZE)
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let stripe_base = stripe_num
        .checked_mul(GDF_TEST_STRIPE_SIZE)
        .and_then(|start| tile_start_y.checked_add(start))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let stripe_start = stripe_base.saturating_sub(GDF_TEST_STRIPE_OFF);
    let stripe_end = stripe_base
        .checked_add(GDF_TEST_STRIPE_SIZE - GDF_TEST_STRIPE_OFF - 1)
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let (luma_start_x, luma_end_x, luma_start_y, luma_end_y) = if disable_loopfilters_across_tiles {
        (tile_start_x, tile_end_x, tile_start_y, tile_end_y)
    } else {
        (0, frame_end_x, 0, frame_end_y)
    };
    Ok(LoopRestorationSourceBounds {
        luma_start_x,
        luma_end_x,
        luma_start_y,
        luma_end_y,
        luma_stripe_start_y: stripe_start.max(luma_start_y).min(luma_end_y),
        luma_stripe_end_y: stripe_end.min(luma_end_y),
        subsampling_x: 0,
        subsampling_y: 0,
    })
}

fn resize_scratch<T: Clone + Default>(
    buffer: &mut Vec<T>,
    len: usize,
    offset: ByteOffset,
) -> Result<()> {
    buffer.clear();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| gdf_filter_error(offset, GDF_SCRATCH_ALLOCATION_REASON))?;
    buffer.resize(len, T::default());
    Ok(())
}

struct GdfSource<'a> {
    samples: &'a [u16],
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

impl<'a> GdfSource<'a> {
    fn materialize<T: ReconSample>(
        samples: &'a mut Vec<u16>,
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
        let sample_count = width.checked_mul(height).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
            )
        })?;
        resize_scratch(samples, sample_count, offset)?;
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
            let left_clamped_count = (left.x as isize - origin_x).max(0) as usize;
            let middle_count = ((right.x as isize - origin_x).min(width as isize)
                - left_clamped_count as isize)
                .max(0) as usize;
            let right_clamped_count = width - left_clamped_count - middle_count;

            let left_val = source_row.get(left.x).copied().ok_or_else(source_error)?;
            let left_val_t = T::try_from_u16(left_val)
                .map_err(|_| source_error())?
                .to_u16();

            let right_val = source_row.get(right.x).copied().ok_or_else(source_error)?;
            let right_val_t = T::try_from_u16(right_val)
                .map_err(|_| source_error())?
                .to_u16();

            let row_offset = row * width;

            if left_clamped_count > 0 {
                samples[row_offset..row_offset + left_clamped_count].fill(left_val_t);
            }

            if middle_count > 0 {
                let src_start = (origin_x + left_clamped_count as isize) as usize;
                let src_slice = source_row
                    .get(src_start..src_start + middle_count)
                    .ok_or_else(source_error)?;
                let dst_slice = &mut samples[row_offset + left_clamped_count
                    ..row_offset + left_clamped_count + middle_count];
                for (dst, &src) in dst_slice.iter_mut().zip(src_slice.iter()) {
                    *dst = T::try_from_u16(src).map_err(|_| source_error())?.to_u16();
                }
            }

            if right_clamped_count > 0 {
                samples[row_offset + left_clamped_count + middle_count..row_offset + width]
                    .fill(right_val_t);
            }
        }
        Ok(Self {
            samples: samples.as_slice(),
            stride: width,
            origin_x,
            origin_y,
        })
    }

    #[cfg(test)]
    fn get(&self, x: isize, y: isize) -> i32 {
        let Some((col, row)) = self.relative_position_signed(x, y) else {
            return 0;
        };
        if col >= self.stride {
            return 0;
        }
        self.samples
            .get(row * self.stride + col)
            .map_or(0, |&sample| i32::from(sample))
    }

    fn relative_position(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        self.relative_position_signed(isize::try_from(x).ok()?, isize::try_from(y).ok()?)
    }

    fn relative_position_signed(&self, x: isize, y: isize) -> Option<(usize, usize)> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        Some((col, row))
    }
}

fn band_gradients(
    source: &GdfSource<'_>,
    source_origin: (usize, usize),
    rows: usize,
    cols: usize,
    grad: &mut Vec<u16>,
    offset: ByteOffset,
) -> Result<()> {
    let plane = rows.checked_mul(cols).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let len = GDF_DIRECTIONS.checked_mul(plane).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    resize_scratch(grad, len, offset)?;

    let stride = source.stride;
    let col_start_idx = source_origin.0 - 2;
    let len_to_slice = cols + 2;

    let (g0, g_rest) = grad.split_at_mut(plane);
    let (g1, g_rest) = g_rest.split_at_mut(plane);
    let (g2, g3) = g_rest.split_at_mut(plane);

    for i in 0..rows {
        let sample_row = source_origin.1 - 1 + i;

        let prev_start = (sample_row - 1) * stride + col_start_idx;
        let curr_start = sample_row * stride + col_start_idx;
        let next_start = (sample_row + 1) * stride + col_start_idx;

        let prev_slice = source
            .samples
            .get(prev_start..prev_start + len_to_slice)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                )
            })?;
        let curr_slice = source
            .samples
            .get(curr_start..curr_start + len_to_slice)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                )
            })?;
        let next_slice = source
            .samples
            .get(next_start..next_start + len_to_slice)
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                )
            })?;

        let start_idx = i * cols;
        let end_idx = start_idx + cols;
        let r0 = g0.get_mut(start_idx..end_idx).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        let r1 = g1.get_mut(start_idx..end_idx).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        let r2 = g2.get_mut(start_idx..end_idx).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        let r3 = g3.get_mut(start_idx..end_idx).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;

        let prev = &prev_slice[..len_to_slice];
        let curr = &curr_slice[..len_to_slice];
        let next = &next_slice[..len_to_slice];
        let d0 = &mut r0[..cols];
        let d1 = &mut r1[..cols];
        let d2 = &mut r2[..cols];
        let d3 = &mut r3[..cols];

        for j in 0..cols {
            let center = i32::from(curr[j + 1]);

            let before_p1 = i32::from(prev[j + 1]);
            let after_n1 = i32::from(next[j + 1]);
            d0[j] = (center * 2 - before_p1 - after_n1).unsigned_abs() as u16;

            let before_c0 = i32::from(curr[j]);
            let after_c2 = i32::from(curr[j + 2]);
            d1[j] = (center * 2 - before_c0 - after_c2).unsigned_abs() as u16;

            let before_p0 = i32::from(prev[j]);
            let after_n2 = i32::from(next[j + 2]);
            d2[j] = (center * 2 - before_p0 - after_n2).unsigned_abs() as u16;

            let before_n0 = i32::from(next[j]);
            let after_p2 = i32::from(prev[j + 2]);
            d3[j] = (center * 2 - before_n0 - after_p2).unsigned_abs() as u16;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct GdfClass {
    index: u8,
    strength_contribution: [i32; 3],
}

fn band_classes(
    grad: &[u16],
    grad_cols: usize,
    block: &GdfBlock,
    classes: &mut Vec<GdfClass>,
    offset: ByteOffset,
) -> Result<()> {
    let class_cols = block.width >> 1;
    let class_rows = block.height >> 1;
    let plane = block
        .height
        .checked_add(2)
        .and_then(|rows| rows.checked_mul(grad_cols))
        .ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
    let grad_len = GDF_DIRECTIONS.checked_mul(plane).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    if grad.len() != grad_len {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let strength_shift = if block.bit_depth == BitDepth::Eight {
        2
    } else {
        4
    };
    let len = class_rows.checked_mul(class_cols).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    resize_scratch(classes, len, offset)?;

    let (g0, g_rest) = grad.split_at(plane);
    let (g1, g_rest) = g_rest.split_at(plane);
    let (g2, g3) = g_rest.split_at(plane);

    let classes_slice = classes.get_mut(..len).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let mut classes_iter = classes_slice.iter_mut();

    for i in 0..class_rows {
        for j in 0..class_cols {
            let mut strengths = [0_u32; GDF_DIRECTIONS];
            strengths[0] = grad_sum_4x4(g0, grad_cols, i * 2, j * 2);
            strengths[1] = grad_sum_4x4(g1, grad_cols, i * 2, j * 2);
            strengths[2] = grad_sum_4x4(g2, grad_cols, i * 2, j * 2);
            strengths[3] = grad_sum_4x4(g3, grad_cols, i * 2, j * 2);

            let index = u8::from(strengths[0] <= strengths[1])
                | (u8::from(strengths[2] <= strengths[3]) << 1);
            let cls = usize::from(index);
            let mut strength_contribution = [0_i32; 3];
            for (direction, strength) in strengths.into_iter().enumerate() {
                let k = GDF_COORDS.len() + direction;
                let alpha = alpha_table[k][cls];
                let comb = ((strength >> strength_shift) as i32).min(alpha);
                for (idx, total) in strength_contribution.iter_mut().enumerate() {
                    *total += comb * weight_table[idx][k][cls];
                }
            }
            if let Some(dest) = classes_iter.next() {
                *dest = GdfClass {
                    index,
                    strength_contribution,
                };
            }
        }
    }
    Ok(())
}

fn gdf_tap_offsets(stride: usize, offset: ByteOffset) -> Result<[usize; GDF_COORDS.len()]> {
    let stride = isize::try_from(stride).map_err(|_| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
        )
    })?;
    let mut offsets = [0_usize; GDF_COORDS.len()];
    for (slot, &(dy, dx)) in offsets.iter_mut().zip(GDF_COORDS.iter()) {
        let val = dy * stride + dx;
        *slot = usize::try_from(val).map_err(|_| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
            )
        })?;
    }
    Ok(offsets)
}

#[allow(clippy::too_many_arguments)]
fn gdf_width4_row(
    base_luma: &[u16],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    classes: [GdfClass; 2],
    block: &GdfBlock,
    row: usize,
    source_origin: (usize, usize),
    offset: ByteOffset,
) -> Result<[u16; MI_SIZE]> {
    let source_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    };
    let base = (source_origin.1 + row) * source.stride + source_origin.0;

    let max_tap = *tap_offsets.iter().max().unwrap_or(&0);
    let samples = source.samples;
    if base < max_tap || base + max_tap + MI_SIZE > samples.len() {
        return Err(source_error());
    }

    let centers: &[u16; 4] = (&samples[base..base + MI_SIZE])
        .try_into()
        .map_err(|_| source_error())?;
    let center_values: [i32; MI_SIZE] = [
        i32::from(centers[0]),
        i32::from(centers[1]),
        i32::from(centers[2]),
        i32::from(centers[3]),
    ];
    let mut gdf_idx: [[i32; 3]; MI_SIZE] = [
        classes[0].strength_contribution,
        classes[0].strength_contribution,
        classes[1].strength_contribution,
        classes[1].strength_contribution,
    ];
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));

    let cls0 = usize::from(classes[0].index);
    let cls1 = usize::from(classes[1].index);

    let wt0 = &weight_table[0];
    let wt1 = &weight_table[1];
    let wt2 = &weight_table[2];

    for (k, &tap) in tap_offsets.iter().enumerate() {
        let neg: &[u16; 4] = samples[base - tap..base - tap + 4]
            .try_into()
            .map_err(|_| source_error())?;
        let pos: &[u16; 4] = samples[base + tap..base + tap + 4]
            .try_into()
            .map_err(|_| source_error())?;

        let neg0 = i32::from(neg[0]);
        let neg1 = i32::from(neg[1]);
        let neg2 = i32::from(neg[2]);
        let neg3 = i32::from(neg[3]);

        let pos0 = i32::from(pos[0]);
        let pos1 = i32::from(pos[1]);
        let pos2 = i32::from(pos[2]);
        let pos3 = i32::from(pos[3]);

        let alpha0 = alpha_table[k][cls0];
        let alpha1 = alpha_table[k][cls1];

        let wt0_k = &wt0[k];
        let wt1_k = &wt1[k];
        let wt2_k = &wt2[k];

        {
            let above = ((neg0 - center_values[0]) << shift).clamp(-alpha0, alpha0);
            let below = ((pos0 - center_values[0]) << shift).clamp(-alpha0, alpha0);
            let comb = (above + below).clamp(-512, 511);
            gdf_idx[0][0] += comb * wt0_k[cls0];
            gdf_idx[0][1] += comb * wt1_k[cls0];
            gdf_idx[0][2] += comb * wt2_k[cls0];
        }
        {
            let above = ((neg1 - center_values[1]) << shift).clamp(-alpha0, alpha0);
            let below = ((pos1 - center_values[1]) << shift).clamp(-alpha0, alpha0);
            let comb = (above + below).clamp(-512, 511);
            gdf_idx[1][0] += comb * wt0_k[cls0];
            gdf_idx[1][1] += comb * wt1_k[cls0];
            gdf_idx[1][2] += comb * wt2_k[cls0];
        }
        {
            let above = ((neg2 - center_values[2]) << shift).clamp(-alpha1, alpha1);
            let below = ((pos2 - center_values[2]) << shift).clamp(-alpha1, alpha1);
            let comb = (above + below).clamp(-512, 511);
            gdf_idx[2][0] += comb * wt0_k[cls1];
            gdf_idx[2][1] += comb * wt1_k[cls1];
            gdf_idx[2][2] += comb * wt2_k[cls1];
        }
        {
            let above = ((neg3 - center_values[3]) << shift).clamp(-alpha1, alpha1);
            let below = ((pos3 - center_values[3]) << shift).clamp(-alpha1, alpha1);
            let comb = (above + below).clamp(-512, 511);
            gdf_idx[3][0] += comb * wt0_k[cls1];
            gdf_idx[3][1] += comb * wt1_k[cls1];
            gdf_idx[3][2] += comb * wt2_k[cls1];
        }
    }
    Ok(core::array::from_fn(|col| {
        finish_gdf_sample(base_luma, block, row, col, &gdf_idx[col])
    }))
}

#[allow(clippy::too_many_arguments)]
fn gdf_sample(
    base_luma: &[u16],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    block: &GdfBlock,
    row: usize,
    col: usize,
    source_position: (usize, usize),
    class: &GdfClass,
) -> u16 {
    let (source_col, source_row) = source_position;
    let samples = source.samples;
    let base = source_row * source.stride + source_col;
    let sample2 = i32::from(samples[base]);
    let cls = usize::from(class.index);
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let mut gdf_idx = class.strength_contribution;
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    for (k, &tap) in tap_offsets.iter().enumerate() {
        let alpha = alpha_table[k][cls];
        let sample3 = i32::from(samples[base - tap]);
        let sample4 = i32::from(samples[base + tap]);
        let above = ((sample3 - sample2) << shift).clamp(-alpha, alpha);
        let below = ((sample4 - sample2) << shift).clamp(-alpha, alpha);
        let comb = (above + below).clamp(-512, 511);
        for (idx, total) in gdf_idx.iter_mut().enumerate() {
            *total += comb * weight_table[idx][k][cls];
        }
    }

    finish_gdf_sample(base_luma, block, row, col, &gdf_idx)
}

fn finish_gdf_sample(
    base_luma: &[u16],
    block: &GdfBlock,
    row: usize,
    col: usize,
    gdf_idx: &[i32; 3],
) -> u16 {
    let scale = if block.ref_dst_idx == GDF_INTRA_REF_DST {
        8_i32
    } else {
        5_i32
    };

    let bias = &GDF_BIAS[block.ref_dst_idx][block.qp_idx];

    // idx = 0
    let biased0 = (gdf_idx[0] + bias[0]) * scale;
    let v0 = round2_signed_i32(biased0, 15);
    let digit0 = (v0.clamp(-scale, scale - 1) + scale) as usize;

    // idx = 1
    let biased1 = (gdf_idx[1] + bias[1]) * scale;
    let v1 = round2_signed_i32(biased1, 15);
    let digit1 = (v1.clamp(-scale, scale - 1) + scale) as usize;

    // idx = 2
    let biased2 = (gdf_idx[2] + bias[2]) * scale;
    let v2 = round2_signed_i32(biased2, 15);
    let digit2 = (v2.clamp(-scale, scale - 1) + scale) as usize;

    let scale_u = scale as usize * 2;
    let pos = (digit0 * scale_u + digit1) * scale_u + digit2;

    let err = if block.ref_dst_idx == GDF_INTRA_REF_DST {
        GDF_INTRA_ERROR[block.qp_idx][pos]
    } else {
        GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx][pos]
    };
    let residual = round2_signed_i32(
        err * block.pix_scale,
        12 - u32::from(block.bit_depth.bits()),
    );
    let base_index = row * block.frame_width + block.x + col;
    let base = base_luma
        .get(base_index)
        .copied()
        .map(i32::from)
        .unwrap_or_default();
    (base + residual).clamp(0, block.max_sample) as u16
}

#[inline]
fn grad_sum_4x4(grad: &[u16], stride: usize, row: usize, col: usize) -> u32 {
    let offset0 = row * stride + col;
    let offset1 = (row + 1) * stride + col;
    let offset2 = (row + 2) * stride + col;
    let offset3 = (row + 3) * stride + col;

    let r0: &[u16; 4] = (&grad[offset0..offset0 + 4]).try_into().unwrap_or(&[0; 4]);
    let r1: &[u16; 4] = (&grad[offset1..offset1 + 4]).try_into().unwrap_or(&[0; 4]);
    let r2: &[u16; 4] = (&grad[offset2..offset2 + 4]).try_into().unwrap_or(&[0; 4]);
    let r3: &[u16; 4] = (&grad[offset3..offset3 + 4]).try_into().unwrap_or(&[0; 4]);

    u32::from(r0[0])
        + u32::from(r0[1])
        + u32::from(r0[2])
        + u32::from(r0[3])
        + u32::from(r1[0])
        + u32::from(r1[1])
        + u32::from(r1[2])
        + u32::from(r1[3])
        + u32::from(r2[0])
        + u32::from(r2[1])
        + u32::from(r2[2])
        + u32::from(r2[3])
        + u32::from(r3[0])
        + u32::from(r3[1])
        + u32::from(r3[2])
        + u32::from(r3[3])
}

#[cfg(test)]
fn grad_sum(
    grad: &[u16],
    stride: usize,
    row: usize,
    col: usize,
    down: usize,
    across: usize,
) -> u32 {
    let mut total = 0_u32;
    for y in row..row + down {
        for x in col..col + across {
            total += u32::from(grad[y * stride + x]);
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
    let qp_bucket = ((qp_diff - 37) / 25).clamp(0, 2);
    let qc_idx = i32::from(pic_qc_idx.unwrap_or(0));
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
        GDF_ALPHA, GDF_READ_RADIUS, GDF_SCRATCH_ALLOCATION_REASON, GdfBlock, GdfBlockGrid,
        GdfClass, GdfReferenceContext, GdfSource, RESTRICTED_ORDER_HINT, band_classes,
        band_gradients, compute_block, copy_base_block, gdf_inter_ref_dst_idx,
        gdf_inter_ref_dst_idx_from_max_dist, gdf_sample, gdf_stripe_row_for_tile_end,
        gdf_tap_offsets, gdf_width4_row, grad_sum, preserve_lossless_luma_samples, resize_scratch,
        tile_axis_bounds,
    };
    use crate::error::DecodeError;
    use crate::filters::deblock::DeblockBlock;
    use crate::filters::lossless::LosslessBlockGrid;
    use splot_core::span::ByteOffset;
    use splot_recon::{BitDepth, LoopRestorationSourceBounds};

    #[test]
    fn per_block_grid_selects_units_by_restoration_stripe_row() {
        let result = GdfBlockGrid::new(64, 2, 2, vec![1, 1, 0, 1]);
        assert!(result.is_ok());
        let Ok(grid) = result else {
            return;
        };

        assert_eq!(grid.enabled(0, 0), Some(true));
        assert_eq!(grid.enabled(0, 64), Some(true));
        assert_eq!(grid.enabled(16, 0), Some(false));
        assert_eq!(grid.enabled(16, 64), Some(true));
    }

    #[test]
    fn per_block_grid_rejects_invalid_shape_and_values() {
        assert!(GdfBlockGrid::new(0, 1, 1, vec![1]).is_err());
        assert!(GdfBlockGrid::new(62, 1, 1, vec![1]).is_err());
        assert!(GdfBlockGrid::new(64, 2, 2, vec![1, 0, 1]).is_err());
        assert!(GdfBlockGrid::new(64, 1, 1, vec![2]).is_err());
    }

    #[test]
    fn stripe_row_switches_before_the_next_sixty_four_pixel_band() {
        let offset = ByteOffset::new(0);

        assert_eq!(gdf_stripe_row_for_tile_end(52, 127, offset).ok(), Some(0));
        assert_eq!(gdf_stripe_row_for_tile_end(56, 127, offset).ok(), Some(16));
        assert_eq!(gdf_stripe_row_for_tile_end(124, 127, offset).ok(), Some(31));
        assert_eq!(gdf_stripe_row_for_tile_end(60, 63, offset).ok(), Some(15));
        assert_eq!(gdf_stripe_row_for_tile_end(64, 127, offset).ok(), Some(16));
    }

    #[test]
    fn tile_axis_bounds_select_the_containing_tile() {
        let starts = [0, 16, 32];
        let offset = ByteOffset::new(0);

        assert_eq!(
            tile_axis_bounds(&starts, 0, 128, offset).ok(),
            Some((0, 63))
        );
        assert_eq!(
            tile_axis_bounds(&starts, 63, 128, offset).ok(),
            Some((0, 63))
        );
        assert_eq!(
            tile_axis_bounds(&starts, 64, 128, offset).ok(),
            Some((64, 127))
        );
        assert_eq!(
            tile_axis_bounds(&starts, 127, 128, offset).ok(),
            Some((64, 127))
        );
        assert!(tile_axis_bounds(&starts, 128, 128, offset).is_err());
        assert!(tile_axis_bounds(&[0, 16], 64, 128, offset).is_err());
    }

    #[test]
    fn disabled_gdf_unit_copies_base_samples() {
        let base: Vec<u16> = (0..64).collect();

        let result = copy_base_block::<u8>(&base, 8, 4, 2, 4, 4, ByteOffset::new(0));
        assert!(result.is_ok());
        let Ok(copied) = result else {
            return;
        };

        assert_eq!(
            copied,
            vec![
                20, 21, 22, 23, 28, 29, 30, 31, 36, 37, 38, 39, 44, 45, 46, 47
            ]
        );
        assert!(copy_base_block::<u8>(&base[..60], 8, 4, 6, 4, 4, ByteOffset::new(0)).is_err());
    }

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
        let mut band_samples = Vec::new();
        let band_result = GdfSource::materialize::<u16>(
            &mut band_samples,
            &curr,
            &cdef,
            &bounds,
            &band_block,
            ByteOffset::new(0),
        );
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
            let mut local_samples = Vec::new();
            let local_result = GdfSource::materialize::<u16>(
                &mut local_samples,
                &curr,
                &cdef,
                &bounds,
                &block,
                ByteOffset::new(0),
            );
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

    #[test]
    fn maximum_ten_bit_gradient_and_filter_fit_narrow_state() {
        let frame_width = 16;
        let frame_height = 16;
        let curr: Vec<u16> = (0..frame_width * frame_height)
            .map(|index| if index & 1 == 0 { 1023 } else { 0 })
            .collect();
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: frame_width - 1,
            luma_start_y: 0,
            luma_end_y: frame_height - 1,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: frame_height - 1,
            subsampling_x: 0,
            subsampling_y: 0,
        };
        let block = GdfBlock {
            x: 4,
            y: 4,
            width: 4,
            height: 4,
            frame_width,
            frame_height,
            bit_depth: BitDepth::Ten,
            qp_idx: 0,
            ref_dst_idx: 0,
            pix_scale: 4,
            max_sample: 1023,
        };
        let mut source_samples = Vec::new();
        let source_result = GdfSource::materialize::<u16>(
            &mut source_samples,
            &curr,
            &curr,
            &bounds,
            &block,
            ByteOffset::new(0),
        );
        assert!(source_result.is_ok());
        let Ok(source) = source_result else {
            return;
        };
        let Some(origin) = source.relative_position(block.x, block.y) else {
            return;
        };
        let grad_cols = block.width + 2;
        let plane = (block.height + 2) * grad_cols;
        let mut grad = Vec::new();
        let grad_result = band_gradients(
            &source,
            origin,
            block.height + 2,
            grad_cols,
            &mut grad,
            ByteOffset::new(0),
        );
        assert!(grad_result.is_ok());
        assert!(grad.iter().all(|&value| value <= 2046));
        assert!(
            grad.chunks_exact(plane)
                .all(|direction| grad_sum(direction, grad_cols, 0, 0, 4, 4) <= 32_736)
        );

        let mut classes = Vec::new();
        let classes_result =
            band_classes(&grad, grad_cols, &block, &mut classes, ByteOffset::new(0));
        assert!(classes_result.is_ok());
        let filtered = compute_block::<u16>(
            &source,
            &curr,
            &classes,
            block.width >> 1,
            block,
            ByteOffset::new(0),
        );
        assert!(filtered.is_ok());
        assert!(filtered.is_ok_and(|samples| samples.into_iter().all(|sample| sample <= 1023)));
    }

    #[test]
    fn scratch_resize_reuses_storage_and_clears_old_values() {
        let mut scratch = Vec::new();
        assert!(resize_scratch(&mut scratch, 8, ByteOffset::new(0)).is_ok());
        scratch.fill(9_u16);
        let capacity = scratch.capacity();

        assert!(resize_scratch(&mut scratch, 4, ByteOffset::new(0)).is_ok());
        assert_eq!(scratch, vec![0; 4]);
        assert_eq!(scratch.capacity(), capacity);
    }

    #[test]
    fn scratch_capacity_overflow_is_typed() {
        let mut scratch = vec![1_u8];
        let result = resize_scratch(&mut scratch, usize::MAX, ByteOffset::new(17));

        assert!(matches!(
            result,
            Err(DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == GDF_SCRATCH_ALLOCATION_REASON
        ));
        assert!(scratch.is_empty());
    }

    #[test]
    fn width_four_row_matches_legacy_samples_for_all_tables_and_classes() {
        let source_origin = (GDF_READ_RADIUS, GDF_READ_RADIUS);
        let stride = 4 + GDF_READ_RADIUS * 2;
        let source_len = stride * (1 + GDF_READ_RADIUS * 2);
        let offset = ByteOffset::new(0);
        let tap_offsets_result = gdf_tap_offsets(stride, offset);
        assert!(tap_offsets_result.is_ok());
        let Ok(tap_offsets) = tap_offsets_result else {
            return;
        };

        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max_sample = bit_depth.max_sample();
            for source_case in 0..4 {
                let samples: Vec<u16> = (0..source_len)
                    .map(|index| match source_case {
                        0 => 0,
                        1 => max_sample,
                        2 => {
                            if index.is_multiple_of(2) {
                                0
                            } else {
                                max_sample
                            }
                        }
                        _ => {
                            ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1))
                                as u16
                        }
                    })
                    .collect();
                let source = GdfSource {
                    samples: &samples,
                    stride,
                    origin_x: 0,
                    origin_y: 0,
                };
                let base_luma = [0, max_sample, max_sample / 2, max_sample];
                for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
                    for qp_idx in 0..alpha_by_qp.len() {
                        let block = GdfBlock {
                            x: 0,
                            y: 0,
                            width: 4,
                            height: 1,
                            frame_width: 4,
                            frame_height: 1,
                            bit_depth,
                            qp_idx,
                            ref_dst_idx,
                            pix_scale: source_case + 1,
                            max_sample: i32::from(max_sample),
                        };
                        for class_index in 0..4_u8 {
                            let class_delta = i32::from(class_index);
                            let classes = [
                                GdfClass {
                                    index: class_index,
                                    strength_contribution: [
                                        -511 + class_delta,
                                        class_delta,
                                        511 - class_delta,
                                    ],
                                },
                                GdfClass {
                                    index: 3 - class_index,
                                    strength_contribution: [1_024, -1_024, 256],
                                },
                            ];
                            let row_result = gdf_width4_row(
                                &base_luma,
                                &source,
                                &tap_offsets,
                                classes,
                                &block,
                                0,
                                source_origin,
                                offset,
                            );
                            assert!(row_result.is_ok());
                            let Ok(actual) = row_result else {
                                return;
                            };
                            let expected = core::array::from_fn(|col| {
                                gdf_sample(
                                    &base_luma,
                                    &source,
                                    &tap_offsets,
                                    &block,
                                    0,
                                    col,
                                    (source_origin.0 + col, source_origin.1),
                                    &classes[col >> 1],
                                )
                            });
                            assert_eq!(
                                actual, expected,
                                "bit depth {bit_depth:?}, source {source_case}, reference \
                                 {ref_dst_idx}, qp {qp_idx}, class {class_index}"
                            );
                        }
                    }
                }
            }
        }
    }
}
