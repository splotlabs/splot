// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.5 guided detail filter application.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_BIAS, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BitDepth, LoopRestorationSource, LoopRestorationSourceBounds, PlaneId, ReconSample,
    loop_restoration_source_sample,
};

use crate::Result;
use crate::filters::source::{FramePlane, StripePlane};
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

fn gdf_stripe_end_for_tile(y: usize, tile_start: usize, tile_end: usize) -> Option<usize> {
    let local_y = y.checked_sub(tile_start)?;
    let stripe = local_y.checked_add(GDF_TEST_STRIPE_OFF)? / GDF_TEST_STRIPE_SIZE;
    let stripe_base = stripe
        .checked_mul(GDF_TEST_STRIPE_SIZE)?
        .checked_add(tile_start)?;
    let end = stripe_base
        .checked_add(GDF_TEST_STRIPE_SIZE - GDF_TEST_STRIPE_OFF)?
        .min(tile_end);
    (end > y).then_some(end)
}

pub(crate) fn stripe_ranges(
    core: &FrameHeaderCore,
    luma_height: usize,
    offset: ByteOffset,
) -> Result<Vec<(usize, usize)>> {
    if luma_height == 0 {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    let mut ranges = Vec::new();
    let mut y = 0;
    while y < luma_height {
        let (tile_start, tile_end) = core
            .tile_info
            .as_ref()
            .map_or(Ok((0, luma_height - 1)), |tile| {
                tile_axis_bounds(&tile.mi_row_starts, y, luma_height, offset)
            })?;
        let tile_end = tile_end.checked_add(1).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        let end = gdf_stripe_end_for_tile(y, tile_start, tile_end).ok_or_else(|| {
            gdf_filter_error(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
            )
        })?;
        ranges.push((y, end));
        y = end;
    }
    Ok(ranges)
}

#[derive(Default)]
struct GdfScratch {
    source: Vec<u16>,
    classes: Vec<GdfClass>,
    gradient_pairs: [Vec<[u16; GDF_DIRECTIONS]>; 2],
    gradient_tmp: Vec<[u16; GDF_DIRECTIONS]>,
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

#[derive(Clone, Copy)]
struct GdfConfig {
    per_block: bool,
    qp_idx: usize,
    ref_dst_idx: usize,
    pix_scale: i32,
    max_sample: i32,
}

fn gdf_config(
    core: &FrameHeaderCore,
    block_grid: Option<&GdfBlockGrid>,
    bit_depth: BitDepth,
    reference: Option<GdfReferenceContext>,
    offset: ByteOffset,
) -> Result<Option<GdfConfig>> {
    let Some(gdf) = core.gdf_params.as_ref().filter(|gdf| gdf.gdf_frame_enable) else {
        return Ok(None);
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
    Ok(Some(GdfConfig {
        per_block,
        qp_idx,
        ref_dst_idx,
        pix_scale: i32::from(gdf.gdf_pic_scale_idx.unwrap_or(0)) + 1,
        max_sample: i32::from(bit_depth.max_sample()),
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_stripe<T: ReconSample>(
    core: &FrameHeaderCore,
    deblocked_luma: FramePlane<'_, T>,
    separate_cdef_luma: Option<&StripePlane>,
    post_lr_luma: &mut StripePlane,
    block_grid: Option<&GdfBlockGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    bit_depth: BitDepth,
    disable_loopfilters_across_tiles: bool,
    reference: Option<GdfReferenceContext>,
    offset: ByteOffset,
) -> Result<()> {
    let Some(config) = gdf_config(core, block_grid, bit_depth, reference, offset)? else {
        return Ok(());
    };
    let width = post_lr_luma.width();
    let frame_height = post_lr_luma.frame_height();
    let y = post_lr_luma.origin_y();
    let sample_count = post_lr_luma.samples().len();
    let height = sample_count.checked_div(width).filter(|&height| {
        height >= 2 && height.is_multiple_of(2) && height.checked_mul(width) == Some(sample_count)
    });
    let Some(height) = height else {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    };
    let end_y = y.checked_add(height).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    let (tile_start, tile_end) = core
        .tile_info
        .as_ref()
        .map_or(Ok((0, frame_height - 1)), |tile| {
            tile_axis_bounds(&tile.mi_row_starts, y, frame_height, offset)
        })?;
    let tile_end = tile_end.checked_add(1).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    if gdf_stripe_end_for_tile(y, tile_start, tile_end) != Some(end_y)
        || deblocked_luma.width() != width
        || deblocked_luma.frame_height() != frame_height
        || separate_cdef_luma.is_some_and(|cdef_luma| {
            cdef_luma.width() != width
                || cdef_luma.frame_height() != frame_height
                || cdef_luma.origin_y() != y
                || cdef_luma.samples().len() != sample_count
        })
    {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        ));
    }
    let band_segments = if disable_loopfilters_across_tiles {
        gdf_band_segments(core, width, offset)?
    } else {
        vec![(0, width)]
    };
    let stripe_row = gdf_stripe_row(core, y, frame_height, offset)?;
    with_reusable_scratch(&GDF_SCRATCH, |scratch| {
        for &(segment_x, segment_width) in &band_segments {
            let stripe_block = GdfBlock {
                x: segment_x,
                y,
                width: segment_width,
                height,
                frame_width: width,
                frame_height,
                base_origin_y: y,
                bit_depth,
                qp_idx: config.qp_idx,
                ref_dst_idx: config.ref_dst_idx,
                pix_scale: config.pix_scale,
                max_sample: config.max_sample,
            };
            let bounds = source_bounds(
                core,
                &stripe_block,
                disable_loopfilters_across_tiles,
                offset,
            )?;
            let cdef_luma = separate_cdef_luma.unwrap_or(post_lr_luma);
            let source = GdfSource::materialize_stripe(
                &mut scratch.source,
                deblocked_luma,
                cdef_luma,
                &bounds,
                &stripe_block,
                offset,
            )?;
            let band_origin = source.relative_position(segment_x, y).ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                )
            })?;
            band_classes_from_source(
                &source,
                band_origin,
                &stripe_block,
                &mut scratch.classes,
                &mut scratch.gradient_pairs,
                &mut scratch.gradient_tmp,
                offset,
            )?;
            if !config.per_block
                && lossless_grid.is_none()
                && segment_width.is_multiple_of(MI_SIZE)
                && height.is_multiple_of(2)
            {
                compute_enabled_segment(
                    &source,
                    post_lr_luma.samples_mut(),
                    &scratch.classes,
                    &stripe_block,
                    band_origin,
                    offset,
                )?;
                continue;
            }
            let segment_end = segment_x.checked_add(segment_width).ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                )
            })?;
            let class_cols = segment_width >> 1;
            for local_y in (0..height).step_by(MI_SIZE) {
                let block_y = y + local_y;
                let block_height = MI_SIZE.min(height - local_y);
                let class_start = (local_y >> 1).checked_mul(class_cols).ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                    )
                })?;
                let classes = scratch.classes.get(class_start..).ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                    )
                })?;
                for x in (segment_x..segment_end).step_by(MI_SIZE) {
                    let block_width = MI_SIZE.min(segment_end - x);
                    if block_width < 2
                        || block_height < 2
                        || !block_width.is_multiple_of(2)
                        || !block_height.is_multiple_of(2)
                    {
                        return Err(gdf_filter_error(
                            offset,
                            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
                        ));
                    }
                    let block_enabled = if config.per_block {
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
                    let block = GdfBlock {
                        x,
                        y: block_y,
                        width: block_width,
                        height: block_height,
                        ..stripe_block
                    };
                    let mut output = if block_enabled {
                        compute_block::<u16>(
                            &source,
                            post_lr_luma.samples(),
                            classes,
                            class_cols,
                            block,
                            offset,
                        )?
                    } else {
                        let copied = copy_base_block::<u16>(
                            post_lr_luma.samples(),
                            width,
                            y,
                            x,
                            block_y,
                            block_width,
                            block_height,
                            offset,
                        )?;
                        let mut block = [0; MI_SIZE * MI_SIZE];
                        block[..copied.len()].copy_from_slice(&copied);
                        block
                    };
                    preserve_lossless_luma_samples(
                        lossless_grid,
                        post_lr_luma.samples(),
                        width,
                        y,
                        x,
                        block_y,
                        block_width,
                        block_height,
                        &mut output,
                        offset,
                    )?;
                    for row in 0..block_height {
                        let src = &output[row * block_width..(row + 1) * block_width];
                        let start = (local_y + row) * width + x;
                        post_lr_luma.samples_mut()[start..start + block_width].copy_from_slice(src);
                    }
                }
            }
        }
        Ok(())
    })
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
    origin_y: usize,
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
            .and_then(|sample_y| sample_y.checked_sub(origin_y))
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
    base_origin_y: usize,
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
            let base_start = block
                .y
                .checked_add(row)
                .and_then(|y| y.checked_sub(block.base_origin_y))
                .and_then(|y| y.checked_mul(block.frame_width))
                .and_then(|index| index.checked_add(block.x))
                .ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                    )
                })?;
            let base_values = exact_slice(base_luma, base_start, MI_SIZE)
                .and_then(|samples| <&[u16; MI_SIZE]>::try_from(samples).ok())
                .copied()
                .ok_or_else(|| {
                    gdf_filter_error(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
                    )
                })?;
            let samples = gdf_width4_rows(
                [base_values],
                source,
                &tap_offsets,
                [classes[class_base], classes[class_base + 1]],
                &block,
                row,
                source_origin,
                offset,
            )?[0];
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
            let class = classes[(row >> 1) * class_cols + class_col_base + (col >> 1)];
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

fn compute_enabled_segment(
    source: &GdfSource<'_>,
    base_luma: &mut [u16],
    classes: &[GdfClass],
    block: &GdfBlock,
    source_origin: (usize, usize),
    offset: ByteOffset,
) -> Result<()> {
    let geometry_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    };
    let source_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    };
    if !block.height.is_multiple_of(2) {
        return Err(geometry_error());
    }
    let class_cols = block.width >> 1;
    let tap_offsets = gdf_tap_offsets(source.stride, offset)?;
    for row in (0..block.height).step_by(2) {
        let class_row = (row >> 1)
            .checked_mul(class_cols)
            .ok_or_else(geometry_error)?;
        let output_row = row
            .checked_mul(block.frame_width)
            .and_then(|row| row.checked_add(block.x))
            .ok_or_else(geometry_error)?;
        let mut local_x = 0;
        while local_x < block.width {
            let output_start = output_row.checked_add(local_x).ok_or_else(geometry_error)?;
            let next_output_start = output_start
                .checked_add(block.frame_width)
                .ok_or_else(geometry_error)?;
            let class_start = class_row
                .checked_add(local_x >> 1)
                .ok_or_else(geometry_error)?;
            if block.width - local_x >= 16 {
                let wide_classes = classes
                    .get(class_start..class_start + 8)
                    .and_then(|classes| <&[GdfClass; 8]>::try_from(classes).ok());
                if let Some(wide_classes) =
                    wide_classes.filter(|classes| classes.iter().all(|class| class.index == 3))
                {
                    let base_values = exact_slice(base_luma, output_start, 16)
                        .and_then(|samples| <&[u16; 16]>::try_from(samples).ok())
                        .copied()
                        .ok_or_else(source_error)?;
                    let next_base_values = exact_slice(base_luma, next_output_start, 16)
                        .and_then(|samples| <&[u16; 16]>::try_from(samples).ok())
                        .copied()
                        .ok_or_else(source_error)?;
                    let filtered = gdf_uniform_width_rows::<16, 3, 2>(
                        [base_values, next_base_values],
                        source,
                        &tap_offsets,
                        wide_classes,
                        block,
                        (source_origin.0 + local_x, source_origin.1 + row),
                        offset,
                    )?;
                    base_luma[output_start..output_start + 16].copy_from_slice(&filtered[0]);
                    base_luma[next_output_start..next_output_start + 16]
                        .copy_from_slice(&filtered[1]);
                    local_x += 16;
                    continue;
                }
            }
            if block.width - local_x >= 8 {
                let uniform_classes = classes
                    .get(class_start..class_start + 4)
                    .and_then(|classes| <&[GdfClass; 4]>::try_from(classes).ok())
                    .filter(|classes| classes.iter().all(|class| class.index == classes[0].index));
                if let Some(uniform_classes) = uniform_classes {
                    let base_values = exact_slice(base_luma, output_start, 8)
                        .and_then(|samples| <&[u16; 8]>::try_from(samples).ok())
                        .copied()
                        .ok_or_else(source_error)?;
                    let next_base_values = exact_slice(base_luma, next_output_start, 8)
                        .and_then(|samples| <&[u16; 8]>::try_from(samples).ok())
                        .copied()
                        .ok_or_else(source_error)?;
                    let filtered = match uniform_classes[0].index {
                        0 => gdf_uniform_width_rows::<8, 0, 2>(
                            [base_values, next_base_values],
                            source,
                            &tap_offsets,
                            uniform_classes,
                            block,
                            (source_origin.0 + local_x, source_origin.1 + row),
                            offset,
                        ),
                        1 => gdf_uniform_width_rows::<8, 1, 2>(
                            [base_values, next_base_values],
                            source,
                            &tap_offsets,
                            uniform_classes,
                            block,
                            (source_origin.0 + local_x, source_origin.1 + row),
                            offset,
                        ),
                        2 => gdf_uniform_width_rows::<8, 2, 2>(
                            [base_values, next_base_values],
                            source,
                            &tap_offsets,
                            uniform_classes,
                            block,
                            (source_origin.0 + local_x, source_origin.1 + row),
                            offset,
                        ),
                        _ => gdf_uniform_width_rows::<8, 3, 2>(
                            [base_values, next_base_values],
                            source,
                            &tap_offsets,
                            uniform_classes,
                            block,
                            (source_origin.0 + local_x, source_origin.1 + row),
                            offset,
                        ),
                    }?;
                    base_luma[output_start..output_start + 8].copy_from_slice(&filtered[0]);
                    base_luma[next_output_start..next_output_start + 8]
                        .copy_from_slice(&filtered[1]);
                    local_x += 8;
                    continue;
                }
            }
            let base_values = exact_slice(base_luma, output_start, MI_SIZE)
                .and_then(|samples| <&[u16; MI_SIZE]>::try_from(samples).ok())
                .copied()
                .ok_or_else(source_error)?;
            let next_base_values = exact_slice(base_luma, next_output_start, MI_SIZE)
                .and_then(|samples| <&[u16; MI_SIZE]>::try_from(samples).ok())
                .copied()
                .ok_or_else(source_error)?;
            let classes = classes
                .get(class_start..class_start + 2)
                .ok_or_else(geometry_error)?;
            let filtered = gdf_width4_rows(
                [base_values, next_base_values],
                source,
                &tap_offsets,
                [classes[0], classes[1]],
                block,
                0,
                (source_origin.0 + local_x, source_origin.1 + row),
                offset,
            )?;
            base_luma[output_start..output_start + MI_SIZE].copy_from_slice(&filtered[0]);
            base_luma[next_output_start..next_output_start + MI_SIZE].copy_from_slice(&filtered[1]);
            local_x += MI_SIZE;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn preserve_lossless_luma_samples<T: ReconSample>(
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    base_luma: &[u16],
    luma_width: usize,
    origin_y: usize,
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
                .checked_sub(origin_y)
                .and_then(|row| row.checked_mul(luma_width))
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

enum GdfSourceRow<'a, T> {
    Frame(&'a [T]),
    Stripe(&'a [u16]),
}

impl<T: ReconSample> GdfSourceRow<'_, T> {
    fn len(&self) -> usize {
        match self {
            Self::Frame(row) => row.len(),
            Self::Stripe(row) => row.len(),
        }
    }

    fn get(&self, index: usize) -> Option<u16> {
        match self {
            Self::Frame(row) => row.get(index).map(|sample| sample.to_u16()),
            Self::Stripe(row) => row.get(index).copied(),
        }
    }

    fn copy_range_as<U: ReconSample>(&self, start: usize, dst: &mut [u16]) -> Option<()> {
        let end = start.checked_add(dst.len())?;
        match self {
            Self::Frame(row) => {
                for (dst, sample) in dst.iter_mut().zip(row.get(start..end)?) {
                    *dst = U::try_from_u16(sample.to_u16()).ok()?.to_u16();
                }
            }
            Self::Stripe(row) => {
                for (dst, &sample) in dst.iter_mut().zip(row.get(start..end)?) {
                    *dst = U::try_from_u16(sample).ok()?.to_u16();
                }
            }
        }
        Some(())
    }
}

impl<'a> GdfSource<'a> {
    #[cfg(test)]
    fn materialize<T: ReconSample>(
        samples: &'a mut Vec<u16>,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        bounds: &LoopRestorationSourceBounds,
        block: &GdfBlock,
        offset: ByteOffset,
    ) -> Result<Self> {
        Self::materialize_rows::<u16, T>(samples, bounds, block, offset, |source, y| {
            let plane = match source {
                LoopRestorationSource::CurrFrame => curr_luma,
                LoopRestorationSource::CdefFrame => cdef_luma,
            };
            let start = y.checked_mul(block.frame_width)?;
            plane
                .get(start..start.checked_add(block.frame_width)?)
                .map(GdfSourceRow::Stripe)
        })
    }

    fn materialize_stripe<T: ReconSample>(
        samples: &'a mut Vec<u16>,
        deblocked_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane,
        bounds: &LoopRestorationSourceBounds,
        block: &GdfBlock,
        offset: ByteOffset,
    ) -> Result<Self> {
        Self::materialize_rows::<T, T>(samples, bounds, block, offset, |source, y| match source {
            LoopRestorationSource::CurrFrame => deblocked_luma.row(y).map(GdfSourceRow::Frame),
            LoopRestorationSource::CdefFrame => cdef_luma.row(y).map(GdfSourceRow::Stripe),
        })
    }

    fn materialize_rows<'b, S: ReconSample, T: ReconSample>(
        samples: &'a mut Vec<u16>,
        bounds: &LoopRestorationSourceBounds,
        block: &GdfBlock,
        offset: ByteOffset,
        source_row: impl Fn(LoopRestorationSource, usize) -> Option<GdfSourceRow<'b, S>>,
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
            let source_row = source_row(left.source, left.y)
                .filter(|row| row.len() == block.frame_width)
                .ok_or_else(source_error)?;
            let width_i = isize::try_from(width).map_err(|_| source_error())?;
            let pre = isize::try_from(left.x)
                .ok()
                .and_then(|left| left.checked_sub(origin_x))
                .ok_or_else(source_error)?
                .clamp(0, width_i) as usize;
            let post = origin_x
                .checked_add(width_i)
                .and_then(|end| end.checked_sub(1))
                .and_then(|last| last.checked_sub(isize::try_from(right.x).ok()?))
                .ok_or_else(source_error)?
                .clamp(0, isize::try_from(width - pre).map_err(|_| source_error())?)
                as usize;
            let mid = width - pre - post;
            let row_start = row.checked_mul(width).ok_or_else(source_error)?;
            let dst = samples
                .get_mut(row_start..row_start + width)
                .ok_or_else(source_error)?;
            let left_value = source_row.get(left.x).ok_or_else(source_error)?;
            dst[..pre].fill(
                T::try_from_u16(left_value)
                    .map_err(|_| source_error())?
                    .to_u16(),
            );
            if mid != 0 {
                let mid_start = usize::try_from(
                    origin_x
                        .checked_add(isize::try_from(pre).map_err(|_| source_error())?)
                        .ok_or_else(source_error)?,
                )
                .map_err(|_| source_error())?;
                source_row
                    .copy_range_as::<T>(mid_start, &mut dst[pre..pre + mid])
                    .ok_or_else(source_error)?;
            }
            let right_value = source_row.get(right.x).ok_or_else(source_error)?;
            dst[pre + mid..].fill(
                T::try_from_u16(right_value)
                    .map_err(|_| source_error())?
                    .to_u16(),
            );
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
        self.get_at(col, row)
    }

    fn relative_position(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        self.relative_position_signed(isize::try_from(x).ok()?, isize::try_from(y).ok()?)
    }

    fn relative_position_signed(&self, x: isize, y: isize) -> Option<(usize, usize)> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        Some((col, row))
    }

    #[cfg(test)]
    fn get_at(&self, col: usize, row: usize) -> i32 {
        debug_assert!(col < self.stride);
        self.samples
            .get(row * self.stride + col)
            .map_or(0, |&sample| i32::from(sample))
    }
}

#[cfg(test)]
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
    for i in 0..rows {
        for j in 0..cols {
            let sample_col = source_origin.0 - 1 + j;
            let sample_row = source_origin.1 - 1 + i;
            for (direction, (delta_y, delta_x)) in [(1_isize, 0_isize), (0, 1), (1, 1), (-1, 1)]
                .into_iter()
                .enumerate()
            {
                let before = source.get_at(
                    sample_col.wrapping_add_signed(-delta_x),
                    sample_row.wrapping_add_signed(-delta_y),
                );
                let center = source.get_at(sample_col, sample_row);
                let after = source.get_at(
                    sample_col.wrapping_add_signed(delta_x),
                    sample_row.wrapping_add_signed(delta_y),
                );
                grad[direction * plane + i * cols + j] =
                    (center * 2 - before - after).unsigned_abs() as u16;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GdfClass {
    index: u8,
    gradient_bias: i32,
}

#[cfg(test)]
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
    for i in 0..class_rows {
        for j in 0..class_cols {
            let mut strengths = [0_u32; GDF_DIRECTIONS];
            for direction in 0..GDF_DIRECTIONS {
                let plane_slice = &grad[direction * plane..direction * plane + plane];
                strengths[direction] = grad_sum(plane_slice, grad_cols, i * 2, j * 2, 4, 4);
            }
            let index = u8::from(strengths[0] <= strengths[1])
                | (u8::from(strengths[2] <= strengths[3]) << 1);
            let cls = usize::from(index);
            let mut gradient_bias = 0_i32;
            for (direction, strength) in strengths.into_iter().enumerate() {
                let k = GDF_COORDS.len() + direction;
                let alpha = alpha_table[k][cls];
                let comb = ((strength >> strength_shift) as i32).min(i32::from(alpha));
                gradient_bias += comb * i32::from(weight_table[2][k][cls]);
            }
            classes[i * class_cols + j] = GdfClass {
                index,
                gradient_bias,
            };
        }
    }
    Ok(())
}

fn gradient_pair_row(
    source: &GdfSource<'_>,
    source_origin: (usize, usize),
    row_pair: usize,
    class_cols: usize,
    output: &mut Vec<[u16; GDF_DIRECTIONS]>,
    tmp: &mut Vec<[u16; GDF_DIRECTIONS]>,
    offset: ByteOffset,
) -> Result<()> {
    resize_scratch(tmp, class_cols + 1, offset)?;
    resize_scratch(output, class_cols, offset)?;
    let base_row = source_origin.1 - 1 + row_pair * 2;
    let base_col = source_origin.0 - 1;
    let source_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    };
    let window_col = base_col.checked_sub(1).ok_or_else(source_error)?;
    let window_len = class_cols
        .checked_mul(2)
        .and_then(|width| width.checked_add(4))
        .ok_or_else(source_error)?;
    let first_row = base_row.checked_sub(1).ok_or_else(source_error)?;
    let row_window = |row: usize| {
        row.checked_mul(source.stride)
            .and_then(|start| start.checked_add(window_col))
            .and_then(|start| exact_slice(source.samples, start, window_len))
            .ok_or_else(source_error)
    };
    let rows = [
        row_window(first_row)?,
        row_window(first_row + 1)?,
        row_window(first_row + 2)?,
        row_window(first_row + 3)?,
    ];
    let row0 = gradient_pair_chunks(rows[0]).ok_or_else(source_error)?;
    let row1 = gradient_pair_chunks(rows[1]).ok_or_else(source_error)?;
    let row2 = gradient_pair_chunks(rows[2]).ok_or_else(source_error)?;
    let row3 = gradient_pair_chunks(rows[3]).ok_or_else(source_error)?;
    let row_pairs = row0
        .0
        .iter()
        .zip(row0.1)
        .zip(row1.0.iter().zip(row1.1))
        .zip(row2.0.iter().zip(row2.1))
        .zip(row3.0.iter().zip(row3.1));
    for (
        sums,
        (
            (((row0_left, row0_right), (row1_left, row1_right)), (row2_left, row2_right)),
            (row3_left, row3_right),
        ),
    ) in tmp.iter_mut().zip(row_pairs)
    {
        *sums = [
            gdf_gradient(row0_left[1], row1_left[1], row2_left[1])
                + gdf_gradient(row0_right[0], row1_right[0], row2_right[0])
                + gdf_gradient(row1_left[1], row2_left[1], row3_left[1])
                + gdf_gradient(row1_right[0], row2_right[0], row3_right[0]),
            gdf_gradient(row1_left[0], row1_left[1], row1_right[0])
                + gdf_gradient(row1_left[1], row1_right[0], row1_right[1])
                + gdf_gradient(row2_left[0], row2_left[1], row2_right[0])
                + gdf_gradient(row2_left[1], row2_right[0], row2_right[1]),
            gdf_gradient(row0_left[0], row1_left[1], row2_right[0])
                + gdf_gradient(row0_left[1], row1_right[0], row2_right[1])
                + gdf_gradient(row1_left[0], row2_left[1], row3_right[0])
                + gdf_gradient(row1_left[1], row2_right[0], row3_right[1]),
            gdf_gradient(row2_left[0], row1_left[1], row0_right[0])
                + gdf_gradient(row2_left[1], row1_right[0], row0_right[1])
                + gdf_gradient(row3_left[0], row2_left[1], row1_right[0])
                + gdf_gradient(row3_left[1], row2_right[0], row1_right[1]),
        ];
    }
    for (col, pair) in output.iter_mut().enumerate() {
        for direction in 0..GDF_DIRECTIONS {
            pair[direction] = tmp[col][direction] + tmp[col + 1][direction];
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn gradient_pair_chunks(row: &[u16]) -> Option<(&[[u16; 2]], &[[u16; 2]])> {
    let left_end = row.len().checked_sub(2)?;
    let (left, left_remainder) = row.get(..left_end)?.as_chunks::<2>();
    let (right, right_remainder) = row.get(2..)?.as_chunks::<2>();
    (left_remainder.is_empty() && right_remainder.is_empty()).then_some((left, right))
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn gdf_gradient(before: u16, center: u16, after: u16) -> u16 {
    (i32::from(center) * 2 - i32::from(before) - i32::from(after)).unsigned_abs() as u16
}

fn band_classes_from_source(
    source: &GdfSource<'_>,
    source_origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut Vec<GdfClass>,
    gradient_pairs: &mut [Vec<[u16; GDF_DIRECTIONS]>; 2],
    gradient_tmp: &mut Vec<[u16; GDF_DIRECTIONS]>,
    offset: ByteOffset,
) -> Result<()> {
    if source_origin.0 == 0
        || source_origin.1 == 0
        || !block.width.is_multiple_of(2)
        || !block.height.is_multiple_of(2)
    {
        return Err(gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        ));
    }
    let class_cols = block.width >> 1;
    let class_rows = block.height >> 1;
    let len = class_rows.checked_mul(class_cols).ok_or_else(|| {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_geometry",
        )
    })?;
    resize_scratch(classes, len, offset)?;
    let [previous, current] = gradient_pairs;
    gradient_pair_row(
        source,
        source_origin,
        0,
        class_cols,
        previous,
        gradient_tmp,
        offset,
    )?;
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let strength_shift = if block.bit_depth == BitDepth::Eight {
        2
    } else {
        4
    };
    for row in 0..class_rows {
        gradient_pair_row(
            source,
            source_origin,
            row + 1,
            class_cols,
            current,
            gradient_tmp,
            offset,
        )?;
        for col in 0..class_cols {
            let mut strengths = [0u32; GDF_DIRECTIONS];
            for (direction, strength) in strengths.iter_mut().enumerate() {
                *strength =
                    u32::from(previous[col][direction]) + u32::from(current[col][direction]);
            }
            let index = u8::from(strengths[0] <= strengths[1])
                | (u8::from(strengths[2] <= strengths[3]) << 1);
            let cls = usize::from(index);
            let mut gradient_bias = 0_i32;
            for (direction, strength) in strengths.into_iter().enumerate() {
                let k = GDF_COORDS.len() + direction;
                let alpha = alpha_table[k][cls];
                let comb = ((strength >> strength_shift) as i32).min(i32::from(alpha));
                gradient_bias += comb * i32::from(weight_table[2][k][cls]);
            }
            classes[row * class_cols + col] = GdfClass {
                index,
                gradient_bias,
            };
        }
        core::mem::swap(previous, current);
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
        *slot = dy
            .checked_mul(stride)
            .and_then(|row| row.checked_add(dx))
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| {
                gdf_filter_error(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_gdf_window",
                )
            })?;
    }
    Ok(offsets)
}

fn exact_slice<T>(samples: &[T], start: usize, len: usize) -> Option<&[T]> {
    samples.get(start..)?.get(..len)
}

#[allow(clippy::too_many_arguments)]
fn gdf_width4_rows<const ROWS: usize>(
    base_values: [[u16; MI_SIZE]; ROWS],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    classes: [GdfClass; 2],
    block: &GdfBlock,
    row: usize,
    source_origin: (usize, usize),
    offset: ByteOffset,
) -> Result<[[u16; MI_SIZE]; ROWS]> {
    let source_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    };
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    let mut output = [[0; MI_SIZE]; ROWS];
    let params = GdfFinishParams::from_block(block);
    for row_offset in 0..ROWS {
        let base = (source_origin.1 + row + row_offset) * source.stride + source_origin.0;
        let centers = exact_slice(source.samples, base, MI_SIZE).ok_or_else(source_error)?;
        let mut center_values = [0i32; MI_SIZE];
        for (col, value) in center_values.iter_mut().enumerate() {
            *value = i32::from(centers[col]);
        }
        let mut gdf_idx = [[0i32; 3]; MI_SIZE];
        for (col, cell) in gdf_idx.iter_mut().enumerate() {
            cell[2] = classes[col >> 1].gradient_bias;
        }
        for (k, &tap) in tap_offsets.iter().enumerate() {
            let negative =
                exact_slice(source.samples, base - tap, MI_SIZE).ok_or_else(source_error)?;
            let positive =
                exact_slice(source.samples, base + tap, MI_SIZE).ok_or_else(source_error)?;
            for col in 0..MI_SIZE {
                let cls = usize::from(classes[col >> 1].index);
                let alpha = i32::from(alpha_table[k][cls]);
                let above =
                    ((i32::from(negative[col]) - center_values[col]) << shift).clamp(-alpha, alpha);
                let below =
                    ((i32::from(positive[col]) - center_values[col]) << shift).clamp(-alpha, alpha);
                let comb = (above + below).clamp(-512, 511);
                for (idx, total) in gdf_idx[col].iter_mut().enumerate() {
                    *total += comb * i32::from(weight_table[idx][k][cls]);
                }
            }
        }
        let out = &mut output[row_offset];
        if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            for (col, slot) in out.iter_mut().enumerate() {
                *slot = finish_gdf_sample_with_error::<8, 4096>(
                    i32::from(base_values[row_offset][col]),
                    &params,
                    error,
                    &gdf_idx[col],
                );
            }
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            for (col, slot) in out.iter_mut().enumerate() {
                *slot = finish_gdf_sample_with_error::<5, 1000>(
                    i32::from(base_values[row_offset][col]),
                    &params,
                    error,
                    &gdf_idx[col],
                );
            }
        }
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn gdf_uniform_width_rows<const WIDTH: usize, const CLASS: usize, const ROWS: usize>(
    base_values: [[u16; WIDTH]; ROWS],
    source: &GdfSource<'_>,
    tap_offsets: &[usize; GDF_COORDS.len()],
    classes: &[GdfClass],
    block: &GdfBlock,
    source_origin: (usize, usize),
    offset: ByteOffset,
) -> Result<[[u16; WIDTH]; ROWS]> {
    let source_error = || {
        gdf_filter_error(
            offset,
            "unsupported_wienerns_lr_selectable_transform_records_gdf_source",
        )
    };
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    let mut output = [[0; WIDTH]; ROWS];
    let params = GdfFinishParams::from_block(block);
    for row_offset in 0..ROWS {
        let base = (source_origin.1 + row_offset) * source.stride + source_origin.0;
        let centers = exact_slice(source.samples, base, WIDTH).ok_or_else(source_error)?;
        let mut gdf_idx0 = [0_i32; WIDTH];
        let mut gdf_idx1 = [0_i32; WIDTH];
        let mut gdf_idx2 = [0i32; WIDTH];
        for (col, value) in gdf_idx2.iter_mut().enumerate() {
            *value = classes[col >> 1].gradient_bias;
        }
        for (k, &tap) in tap_offsets.iter().enumerate() {
            let negative =
                exact_slice(source.samples, base - tap, WIDTH).ok_or_else(source_error)?;
            let positive =
                exact_slice(source.samples, base + tap, WIDTH).ok_or_else(source_error)?;
            let alpha = i32::from(alpha_table[k][CLASS]);
            let weight0 = i32::from(weight_table[0][k][CLASS]);
            let weight1 = i32::from(weight_table[1][k][CLASS]);
            let weight2 = i32::from(weight_table[2][k][CLASS]);
            for col in 0..WIDTH {
                let center = i32::from(centers[col]);
                let above = ((i32::from(negative[col]) - center) << shift).clamp(-alpha, alpha);
                let below = ((i32::from(positive[col]) - center) << shift).clamp(-alpha, alpha);
                let comb = (above + below).clamp(-512, 511);
                gdf_idx0[col] += comb * weight0;
                gdf_idx1[col] += comb * weight1;
                gdf_idx2[col] += comb * weight2;
            }
        }
        let out = &mut output[row_offset];
        if block.ref_dst_idx == GDF_INTRA_REF_DST {
            let error = &GDF_INTRA_ERROR[block.qp_idx];
            for (col, slot) in out.iter_mut().enumerate() {
                let gdf_idx = [gdf_idx0[col], gdf_idx1[col], gdf_idx2[col]];
                *slot = finish_gdf_sample_with_error::<8, 4096>(
                    i32::from(base_values[row_offset][col]),
                    &params,
                    error,
                    &gdf_idx,
                );
            }
        } else {
            let error = &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx];
            for (col, slot) in out.iter_mut().enumerate() {
                let gdf_idx = [gdf_idx0[col], gdf_idx1[col], gdf_idx2[col]];
                *slot = finish_gdf_sample_with_error::<5, 1000>(
                    i32::from(base_values[row_offset][col]),
                    &params,
                    error,
                    &gdf_idx,
                );
            }
        }
    }
    Ok(output)
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
    class: GdfClass,
) -> u16 {
    let (source_col, source_row) = source_position;
    let samples = source.samples;
    let base = source_row * source.stride + source_col;
    let sample2 = i32::from(samples[base]);
    let cls = usize::from(class.index);
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let mut gdf_idx = [0, 0, class.gradient_bias];
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    for (k, &tap) in tap_offsets.iter().enumerate() {
        let alpha = i32::from(alpha_table[k][cls]);
        let sample3 = i32::from(samples[base - tap]);
        let sample4 = i32::from(samples[base + tap]);
        let above = ((sample3 - sample2) << shift).clamp(-alpha, alpha);
        let below = ((sample4 - sample2) << shift).clamp(-alpha, alpha);
        let comb = (above + below).clamp(-512, 511);
        for (idx, total) in gdf_idx.iter_mut().enumerate() {
            *total += comb * i32::from(weight_table[idx][k][cls]);
        }
    }
    let base = block
        .y
        .checked_add(row)
        .and_then(|y| y.checked_sub(block.base_origin_y))
        .and_then(|y| y.checked_mul(block.frame_width))
        .and_then(|index| index.checked_add(block.x + col))
        .and_then(|index| base_luma.get(index))
        .copied()
        .map(i32::from)
        .unwrap_or_default();
    finish_gdf_sample(base, block, &gdf_idx)
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn finish_gdf_sample(base: i32, block: &GdfBlock, gdf_idx: &[i32; 3]) -> u16 {
    let params = GdfFinishParams::from_block(block);
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        finish_gdf_sample_with_error::<8, 4096>(
            base,
            &params,
            &GDF_INTRA_ERROR[block.qp_idx],
            gdf_idx,
        )
    } else {
        finish_gdf_sample_with_error::<5, 1000>(
            base,
            &params,
            &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx],
            gdf_idx,
        )
    }
}

/// The block-constant inputs to [`finish_gdf_sample_with_error`], resolved once per block so
/// the per-pixel finish stops re-reading `block` fields (and re-indexing `GDF_BIAS`) through
/// the shared `&GdfBlock` reference on every sample.
struct GdfFinishParams {
    bias: &'static [i32; 3],
    pix_scale: i32,
    residual_shift: u32,
    max_sample: i32,
}

impl GdfFinishParams {
    fn from_block(block: &GdfBlock) -> Self {
        Self {
            bias: &GDF_BIAS[block.ref_dst_idx][block.qp_idx],
            pix_scale: block.pix_scale,
            residual_shift: 12 - u32::from(block.bit_depth.bits()),
            max_sample: block.max_sample,
        }
    }
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn finish_gdf_sample_with_error<const SCALE: i32, const ERROR_LEN: usize>(
    base: i32,
    params: &GdfFinishParams,
    error: &[i32; ERROR_LEN],
    gdf_idx: &[i32; 3],
) -> u16 {
    let mut pos = 0_usize;
    for (idx, value) in gdf_idx.iter().enumerate() {
        let biased = (*value + params.bias[idx]) * SCALE;
        let v = round2_signed_i32(biased, 15);
        let digit = v.clamp(-SCALE, SCALE - 1) + SCALE;
        pos = pos * (SCALE as usize * 2) + usize::try_from(digit).unwrap_or_default();
    }
    let residual = round2_signed_i32(error[pos] * params.pix_scale, params.residual_shift);
    (base + residual).clamp(0, params.max_sample) as u16
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

#[cold]
#[inline(never)]
fn gdf_filter_error(offset: ByteOffset, reason: &'static str) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(offset, reason)
}

#[cfg(test)]
#[path = "gdf_wide_tests.rs"]
mod wide_tests;

#[cfg(test)]
mod tests {
    use super::{
        GDF_COORDS, GDF_READ_RADIUS, GDF_SCRATCH_ALLOCATION_REASON, GDF_WEIGHT, GdfBlock,
        GdfBlockGrid, GdfReferenceContext, GdfSource, RESTRICTED_ORDER_HINT, band_classes,
        band_classes_from_source, band_gradients, compute_block, copy_base_block,
        gdf_inter_ref_dst_idx, gdf_inter_ref_dst_idx_from_max_dist, gdf_stripe_end_for_tile,
        gdf_stripe_row_for_tile_end, grad_sum, preserve_lossless_luma_samples, resize_scratch,
        tile_axis_bounds,
    };
    use crate::error::DecodeError;
    use crate::filters::deblock::DeblockBlock;
    use crate::filters::lossless::LosslessBlockGrid;
    use splot_core::span::ByteOffset;
    use splot_recon::{BitDepth, LoopRestorationSourceBounds};

    #[test]
    fn gradient_weights_only_contribute_to_third_index() {
        for reference in &GDF_WEIGHT {
            for qp in reference {
                for weights in &qp[..2] {
                    for direction_weights in weights.iter().skip(GDF_COORDS.len()).take(4) {
                        assert!(direction_weights.iter().all(|&weight| weight == 0));
                    }
                }
            }
        }
    }

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
    fn stripe_tasks_follow_restoration_boundaries_and_clip_to_tiles() {
        assert_eq!(gdf_stripe_end_for_tile(0, 0, 128), Some(56));
        assert_eq!(gdf_stripe_end_for_tile(52, 0, 128), Some(56));
        assert_eq!(gdf_stripe_end_for_tile(56, 0, 128), Some(120));
        assert_eq!(gdf_stripe_end_for_tile(116, 0, 128), Some(120));
        assert_eq!(gdf_stripe_end_for_tile(120, 0, 128), Some(128));
        assert_eq!(gdf_stripe_end_for_tile(0, 0, 64), Some(56));
        assert_eq!(gdf_stripe_end_for_tile(56, 0, 64), Some(64));
        assert_eq!(gdf_stripe_end_for_tile(64, 64, 128), Some(120));
        assert_eq!(gdf_stripe_end_for_tile(120, 64, 128), Some(128));
        assert_eq!(gdf_stripe_end_for_tile(128, 64, 128), None);
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

        let result = copy_base_block::<u8>(&base, 8, 0, 4, 2, 4, 4, ByteOffset::new(0));
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
        assert!(copy_base_block::<u8>(&base[..60], 8, 0, 4, 6, 4, 4, ByteOffset::new(0)).is_err());
    }

    #[test]
    fn stripe_source_matches_per_block_windows_at_halo_boundaries() {
        let frame_width = 16;
        let frame_height = 128;
        let curr: Vec<u16> = (0..frame_width * frame_height)
            .map(|index| index as u16)
            .collect();
        let cdef: Vec<u16> = curr.iter().map(|&sample| sample + 1_000).collect();
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: frame_width - 1,
            luma_start_y: 0,
            luma_end_y: frame_height - 1,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 55,
            subsampling_x: 0,
            subsampling_y: 0,
        };
        let stripe_block = GdfBlock {
            x: 0,
            y: 0,
            width: frame_width,
            height: 56,
            frame_width,
            frame_height,
            base_origin_y: 0,
            bit_depth: BitDepth::Ten,
            qp_idx: 0,
            ref_dst_idx: 0,
            pix_scale: 1,
            max_sample: 1_023,
        };
        let mut stripe_samples = Vec::new();
        let stripe_result = GdfSource::materialize::<u16>(
            &mut stripe_samples,
            &curr,
            &cdef,
            &bounds,
            &stripe_block,
            ByteOffset::new(0),
        );
        assert!(stripe_result.is_ok());
        let Ok(stripe_source) = stripe_result else {
            return;
        };

        for y in (0..56).step_by(4) {
            for x in (0..frame_width).step_by(4) {
                let block = GdfBlock {
                    x,
                    y,
                    width: 4,
                    height: 4,
                    ..stripe_block
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
                for sample_y in
                    block.y as isize - radius..block.y as isize + block.height as isize + radius
                {
                    for sample_x in
                        block.x as isize - radius..block.x as isize + block.width as isize + radius
                    {
                        assert_eq!(
                            stripe_source.get(sample_x, sample_y),
                            local_source.get(sample_x, sample_y)
                        );
                    }
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
            0,
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
            base_origin_y: 0,
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
        let mut fused_classes = Vec::new();
        let mut gradient_pairs = [Vec::new(), Vec::new()];
        let mut gradient_tmp = Vec::new();
        let fused_result = band_classes_from_source(
            &source,
            origin,
            &block,
            &mut fused_classes,
            &mut gradient_pairs,
            &mut gradient_tmp,
            ByteOffset::new(0),
        );
        assert!(fused_result.is_ok());
        assert_eq!(fused_classes, classes);
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
}
