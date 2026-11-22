// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.5 guided detail filter application.

use splot_core::headers::frame::{FrameHeaderCore, RESTRICTED_ORDER_HINT, get_relative_dist};
use splot_core::tables::loop_restoration::{GDF_ALPHA, GDF_BIAS, GDF_WEIGHT};
#[cfg(test)]
use splot_core::tables::loop_restoration::{GDF_INTER_ERROR, GDF_INTRA_ERROR};
#[cfg(test)]
use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BitDepth, LoopRestorationPlaneBounds, LoopRestorationSource, LoopRestorationSourceBounds,
    PlaneId, ReconSample,
};

use crate::Result;
use crate::filters::source::{FramePlane, StripePlane};
use crate::support::reusable_scratch::with_reusable_scratch;

#[path = "gdf_simd.rs"]
mod simd;

use simd::{ClassScratch, TAP_REACH};
#[cfg(test)]
use simd::{GdfSymParams, source_rows, sym_rows, sym_rows_narrow, window_at};

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

pub(crate) const GDF_READ_RADIUS: usize = 7;
const GDF_INTRA_REF_DST: usize = 0;

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
) -> impl Iterator<Item = Result<(usize, usize)>> + '_ {
    let mut next = Some(0);
    std::iter::from_fn(move || {
        let y = next.take()?;
        let range = (|| {
            if luma_height == 0 {
                return Err(gdf_state_error());
            }
            let (tile_start, tile_end) = core
                .tile_info
                .as_ref()
                .map_or(Ok((0, luma_height - 1)), |tile| {
                    tile_axis_bounds(&tile.mi_row_starts, y, luma_height)
                })?;
            let tile_end = tile_end.checked_add(1).ok_or_else(gdf_state_error)?;
            let end =
                gdf_stripe_end_for_tile(y, tile_start, tile_end).ok_or_else(gdf_state_error)?;
            Ok((y, end))
        })();
        if let Ok((_, end)) = range
            && end < luma_height
        {
            next = Some(end);
        }
        Some(range)
    })
}

/// Lays a frame's stripe ranges into a list a previous frame left behind.
pub(crate) fn stripe_ranges_into(
    core: &FrameHeaderCore,
    luma_height: usize,
    ranges: &mut Vec<(usize, usize)>,
) -> Result<()> {
    ranges.clear();
    for range in stripe_ranges(core, luma_height) {
        ranges.push(range?);
    }
    Ok(())
}

#[derive(Default)]
struct GdfScratch {
    source: Vec<u16>,
    class_scratch: ClassScratch,
    lossless: (Vec<u8>, Vec<u16>),
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
    ) -> Option<Self> {
        let expected = rows.checked_mul(cols)?;
        if block_size == 0
            || !block_size.is_multiple_of(MI_SIZE)
            || rows == 0
            || cols == 0
            || values.len() != expected
            || values.iter().any(|&value| value > 1)
        {
            return None;
        }
        Some(Self {
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

    /// Returns the first run of enabled units in `x..end` as a column range;
    /// the range is empty when no unit in `x..end` is enabled.
    fn enabled_run(
        &self,
        stripe_row: usize,
        x: usize,
        end: usize,
    ) -> Option<core::ops::Range<usize>> {
        let unit_end = |x: usize| ((x / self.block_size + 1) * self.block_size).min(end);
        let mut start = x;
        while start < end && !self.enabled(stripe_row, start)? {
            start = unit_end(start);
        }
        let mut stop = start;
        while stop < end && self.enabled(stripe_row, stop)? {
            stop = unit_end(stop);
        }
        Some(start..stop)
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
) -> Result<Option<GdfConfig>> {
    let Some(gdf) = core.gdf_params.as_ref().filter(|gdf| gdf.gdf_frame_enable) else {
        return Ok(None);
    };
    let per_block = gdf.gdf_per_block.ok_or_else(gdf_state_error)?;
    if per_block && block_grid.is_none() {
        return Err(gdf_state_error());
    }
    let quant = core
        .quantization_params
        .as_ref()
        .ok_or_else(gdf_state_error)?;
    let frame_is_intra = core.frame_is_intra.ok_or_else(gdf_state_error)?;
    let qp_idx = gdf_qp_idx(
        quant.base_q_idx,
        bit_depth,
        frame_is_intra,
        gdf.gdf_pic_qc_idx,
    )?;
    let ref_dst_idx = if frame_is_intra {
        GDF_INTRA_REF_DST
    } else {
        gdf_inter_ref_dst_idx(reference.ok_or_else(gdf_state_error)?)
    };
    Ok(Some(GdfConfig {
        per_block,
        qp_idx,
        ref_dst_idx,
        pix_scale: i32::from(gdf.gdf_pic_scale_idx.unwrap_or(0)) + 1,
        max_sample: i32::from(bit_depth.max_sample()),
    }))
}

pub(crate) fn is_active(
    core: &FrameHeaderCore,
    block_grid: Option<&GdfBlockGrid>,
    bit_depth: BitDepth,
    reference: Option<GdfReferenceContext>,
) -> Result<bool> {
    gdf_config(core, block_grid, bit_depth, reference).map(|config| config.is_some())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_stripe<T: ReconSample>(
    core: &FrameHeaderCore,
    deblocked_luma: FramePlane<'_, T>,
    separate_cdef_luma: Option<&StripePlane<T>>,
    cdef_overlap: &[StripePlane<T>],
    post_lr_luma: &mut StripePlane<T>,
    block_grid: Option<&GdfBlockGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    bit_depth: BitDepth,
    disable_loopfilters_across_tiles: bool,
    reference: Option<GdfReferenceContext>,
) -> Result<()> {
    let Some(config) = gdf_config(core, block_grid, bit_depth, reference)? else {
        return Ok(());
    };
    apply_stripe_config(
        config,
        core,
        (deblocked_luma, separate_cdef_luma, cdef_overlap),
        post_lr_luma,
        (block_grid, lossless_grid),
        bit_depth,
        disable_loopfilters_across_tiles,
    )
}

/// The § 7.20.5 `err` of every enabled luma sample of a stripe whose
/// `post_lr_luma` holds [`GDF_ERROR_BASE`] everywhere, written as
/// `GDF_ERROR_BASE + err`: the stripe runs with `GdfPixScale = 1 << (12 -
/// BitDepth)`, so the rounding keeps `err` exact, and a sample range wide
/// enough that no `err` clips.
#[allow(clippy::too_many_arguments)]
pub(crate) fn error_stripe(
    core: &FrameHeaderCore,
    deblocked_luma: FramePlane<'_, u16>,
    separate_cdef_luma: Option<&StripePlane<u16>>,
    cdef_overlap: &[StripePlane<u16>],
    post_lr_luma: &mut StripePlane<u16>,
    block_grid: Option<&GdfBlockGrid>,
    bit_depth: BitDepth,
    disable_loopfilters_across_tiles: bool,
    reference: Option<GdfReferenceContext>,
) -> Result<()> {
    let Some(config) = gdf_config(core, block_grid, bit_depth, reference)? else {
        return Ok(());
    };
    let config = GdfConfig {
        pix_scale: 1 << (12 - i32::from(bit_depth.bits())),
        max_sample: 2 * GDF_ERROR_BASE as i32 - 1,
        ..config
    };
    apply_stripe_config(
        config,
        core,
        (deblocked_luma, separate_cdef_luma, cdef_overlap),
        post_lr_luma,
        (block_grid, None),
        bit_depth,
        disable_loopfilters_across_tiles,
    )
}

/// The base sample of [`error_stripe`]; every `err` table entry lies well
/// inside `-GDF_ERROR_BASE..GDF_ERROR_BASE`.
pub(crate) const GDF_ERROR_BASE: u16 = 1024;

fn apply_stripe_config<T: ReconSample>(
    config: GdfConfig,
    core: &FrameHeaderCore,
    (deblocked_luma, separate_cdef_luma, cdef_overlap): (
        FramePlane<'_, T>,
        Option<&StripePlane<T>>,
        &[StripePlane<T>],
    ),
    post_lr_luma: &mut StripePlane<T>,
    (block_grid, lossless_grid): (
        Option<&GdfBlockGrid>,
        Option<&crate::filters::lossless::LosslessBlockGrid>,
    ),
    bit_depth: BitDepth,
    disable_loopfilters_across_tiles: bool,
) -> Result<()> {
    let (width, stride, frame_height, y) = (
        post_lr_luma.width(),
        post_lr_luma.stride(),
        post_lr_luma.frame_height(),
        post_lr_luma.origin_y(),
    );
    let Some(height) = post_lr_luma
        .end_y()
        .and_then(|end_y| end_y.checked_sub(y))
        .filter(|&height| height >= 2 && height.is_multiple_of(2))
    else {
        return Err(gdf_state_error());
    };
    let end_y = y.checked_add(height).ok_or_else(gdf_state_error)?;
    let (tile_start, tile_end) = core
        .tile_info
        .as_ref()
        .map_or(Ok((0, frame_height - 1)), |tile| {
            tile_axis_bounds(&tile.mi_row_starts, y, frame_height)
        })?;
    let tile_end = tile_end.checked_add(1).ok_or_else(gdf_state_error)?;
    if gdf_stripe_end_for_tile(y, tile_start, tile_end) != Some(end_y)
        || deblocked_luma.width() != width
        || deblocked_luma.frame_height() != frame_height
        || separate_cdef_luma.is_some_and(|cdef_luma| {
            cdef_luma.width() != width
                || cdef_luma.frame_height() != frame_height
                || cdef_luma.origin_y() != y
                || cdef_luma.end_y() != Some(end_y)
        })
    {
        return Err(gdf_state_error());
    }
    let tile_columns = if disable_loopfilters_across_tiles {
        gdf_band_segments(
            core.tile_info
                .as_ref()
                .map(|tile| (tile.tile_cols, tile.mi_col_starts.as_ref())),
            width,
        )?
    } else {
        None
    };
    let segment_count = tile_columns.map_or(1, |starts| starts.len().saturating_sub(1));
    let stripe_row = gdf_stripe_row(core, y, frame_height)?;
    with_reusable_scratch(&GDF_SCRATCH, |scratch| {
        for segment_index in 0..segment_count {
            let (segment_x, segment_width) = if let Some(starts) = tile_columns {
                let end = segment_index.checked_add(2).ok_or_else(gdf_state_error)?;
                let window = starts.get(segment_index..end).ok_or_else(gdf_state_error)?;
                gdf_band_segment(window, width)?
            } else {
                (0, width)
            };
            let segment_end = segment_x
                .checked_add(segment_width)
                .ok_or_else(gdf_state_error)?;
            let mut next_x = segment_x;
            while next_x < segment_end {
                let run = if config.per_block {
                    block_grid
                        .and_then(|grid| grid.enabled_run(stripe_row, next_x, segment_end))
                        .ok_or_else(gdf_state_error)?
                } else {
                    segment_x..segment_end
                };
                next_x = run.end;
                if run.is_empty() {
                    continue;
                }
                let run_block = GdfBlock {
                    x: run.start,
                    y,
                    width: run.len(),
                    height,
                    frame_width: width,
                    frame_height,
                    base_stride: stride,
                    bit_depth,
                    qp_idx: config.qp_idx,
                    ref_dst_idx: config.ref_dst_idx,
                    pix_scale: config.pix_scale,
                    max_sample: config.max_sample,
                };
                let bounds = source_bounds(core, &run_block, disable_loopfilters_across_tiles)?;
                let sources = (deblocked_luma, separate_cdef_luma, cdef_overlap);
                if let Some(grid) = lossless_grid {
                    let samples = post_lr_luma.samples_mut();
                    keep_lossless_spans(grid, &run_block, samples, &mut scratch.lossless, false)?;
                }
                filter_run_tiles(sources, &bounds, &run_block, post_lr_luma, scratch)?;
                if let Some(grid) = lossless_grid {
                    let samples = post_lr_luma.samples_mut();
                    keep_lossless_spans(grid, &run_block, samples, &mut scratch.lossless, true)?;
                }
            }
        }
        Ok(())
    })
}

/// Filters the run `run` in column tiles of at most [`simd::TILE_COLS`]
/// columns, each tile's source window copied just before the tile is
/// filtered. When `post_lr_luma` is the CDEF luma the windows read, the next
/// tile's window is copied before a tile writes the columns it shares.
fn filter_run_tiles<T: ReconSample>(
    (deblocked_luma, separate_cdef_luma, cdef_overlap): (
        FramePlane<'_, T>,
        Option<&StripePlane<T>>,
        &[StripePlane<T>],
    ),
    bounds: &LoopRestorationPlaneBounds,
    run: &GdfBlock,
    post_lr_luma: &mut StripePlane<T>,
    scratch: &mut GdfScratch,
) -> Result<()> {
    const SIMD_ALIGN: usize = 32;
    const ALIGN_PADDING: usize = SIMD_ALIGN / size_of::<u16>() - 1;
    const _: () = assert!((simd::TILE_STRIDE * size_of::<u16>()).is_multiple_of(SIMD_ALIGN));
    let tile = |index: usize| GdfBlock {
        x: run.x + index * simd::TILE_COLS,
        width: simd::TILE_COLS.min(run.width - index * simd::TILE_COLS),
        ..*run
    };
    let tiles = run.width.div_ceil(simd::TILE_COLS);
    let rows = window_size(run)?.1;
    let area = rows
        .checked_mul(simd::TILE_STRIDE)
        .ok_or_else(gdf_state_error)?;
    let ahead = usize::from(separate_cdef_luma.is_none());
    let buffers = (1 + ahead).min(tiles);
    grow_scratch(&mut scratch.source, buffers * area + ALIGN_PADDING)?;
    let offset = scratch
        .source
        .as_ptr()
        .wrapping_add(TAP_REACH)
        .align_offset(SIMD_ALIGN);
    let source = scratch
        .source
        .get_mut(offset..)
        .ok_or_else(gdf_state_error)?;
    let grads = 2 * ClassScratch::chunks(simd::TILE_COLS.min(run.width) >> 1);
    grow_scratch(&mut scratch.class_scratch.grads, grads)?;
    let errors = 2 * ClassScratch::error_row(simd::TILE_COLS.min(run.width));
    grow_scratch(&mut scratch.class_scratch.errors, errors)?;
    let layout = |index: usize| (simd::TILE_STRIDE, simd::TILE_COLS, index);
    let buffer = |index: usize| index % buffers * area..(index % buffers + 1) * area;
    for index in 0..tiles {
        let first = if index == 0 { 0 } else { index + ahead };
        for staged in first..=(index + ahead).min(tiles - 1) {
            let cdef_luma = separate_cdef_luma.unwrap_or(&*post_lr_luma);
            let rows = stripe_rows(deblocked_luma, cdef_luma, cdef_overlap);
            let samples = source.get_mut(buffer(staged)).ok_or_else(gdf_state_error)?;
            fill_tile::<T, T>(samples, layout(staged), bounds, run, rows)?;
        }
        let samples = source.get(buffer(index)).ok_or_else(gdf_state_error)?;
        let tile = tile(index);
        let scratch = &mut scratch.class_scratch;
        let output = post_lr_luma.samples_mut();
        if let Some(output) = T::u8_slice_mut(output) {
            simd::filter_run(samples, &tile, output, scratch)
        } else if let Some(output) = T::u16_slice_mut(output) {
            simd::filter_run(samples, &tile, output, scratch)
        } else {
            None
        }
        .ok_or_else(gdf_state_error)?;
    }
    Ok(())
}

/// Saves, or with `restore` writes back, the row spans of the lossless luma
/// 4x4 cells of `run` in `samples`; § 7.20.1 GDF leaves them unchanged.
#[cold]
fn keep_lossless_spans<T: ReconSample>(
    grid: &crate::filters::lossless::LosslessBlockGrid,
    run: &GdfBlock,
    samples: &mut [T],
    (eight, ten): &mut (Vec<u8>, Vec<u16>),
    restore: bool,
) -> Result<()> {
    let saved = T::depth_vec(eight, ten);
    if !restore {
        saved.clear();
    }
    let mut next = 0;
    lossless_spans(grid, run, |start, len| {
        let row = samples
            .get_mut(start..start + len)
            .ok_or_else(gdf_state_error)?;
        if restore {
            row.copy_from_slice(saved.get(next..next + len).ok_or_else(gdf_state_error)?);
            next += len;
        } else {
            saved.try_reserve(len).map_err(|_| gdf_allocation_error())?;
            saved.extend_from_slice(row);
        }
        Ok(())
    })
}

/// Calls `span` with the local start and length of every row span of the
/// lossless luma 4x4 cells of `run`.
fn lossless_spans(
    grid: &crate::filters::lossless::LosslessBlockGrid,
    run: &GdfBlock,
    mut span: impl FnMut(usize, usize) -> Result<()>,
) -> Result<()> {
    let run_end = run.x + run.width;
    let mut row = 0;
    while row < run.height {
        let y = run.y + row;
        let rows_end = ((y / MI_SIZE + 1) * MI_SIZE - run.y).min(run.height);
        let mut x = run.x;
        while x < run_end {
            let cell_end = ((x / MI_SIZE + 1) * MI_SIZE).min(run_end);
            if !grid.plane_sample_lossless(PlaneId::Y, x, y, 0, 0) {
                x = cell_end;
                continue;
            }
            let mut end = cell_end;
            while end < run_end && grid.plane_sample_lossless(PlaneId::Y, end, y, 0, 0) {
                end = (end + MI_SIZE).min(run_end);
            }
            for local_y in row..rows_end {
                span(local_y * run.base_stride + x, end - x)?;
            }
            x = end;
        }
        row = rows_end;
    }
    Ok(())
}

pub(crate) fn gdf_stripe_row(
    core: &FrameHeaderCore,
    y: usize,
    luma_height: usize,
) -> Result<usize> {
    let frame_end_y = luma_height.checked_sub(1).ok_or_else(gdf_state_error)?;
    let tile_end_y = core.tile_info.as_ref().map_or(Ok(frame_end_y), |tile| {
        tile_axis_bounds(&tile.mi_row_starts, y, luma_height).map(|(_, end)| end)
    })?;
    gdf_stripe_row_for_tile_end(y, tile_end_y)
}

fn gdf_stripe_row_for_tile_end(y: usize, tile_end_y: usize) -> Result<usize> {
    let row = y / MI_SIZE;
    let stripe_row = row
        .checked_add(2)
        .map(|value| (value >> 4) << 4)
        .ok_or_else(gdf_state_error)?;
    Ok(stripe_row.min(tile_end_y / MI_SIZE))
}

#[derive(Clone, Copy)]
struct GdfBlock {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    frame_width: usize,
    frame_height: usize,
    base_stride: usize,
    bit_depth: BitDepth,
    qp_idx: usize,
    ref_dst_idx: usize,
    pix_scale: i32,
    max_sample: i32,
}

/// The `W` samples from column `x` of both output rows.
#[cfg(test)]
fn output_pair<'a, const W: usize>(
    top: &'a mut [u16],
    bottom: &'a mut [u16],
    x: usize,
) -> Option<[&'a mut [u16; W]; 2]> {
    Some([
        top.get_mut(x..)?.first_chunk_mut()?,
        bottom.get_mut(x..)?.first_chunk_mut()?,
    ])
}

#[cfg(test)]
fn compute_enabled_segment(
    source: &GdfSource<'_>,
    base_luma: &mut [u16],
    classes: &[GdfClass],
    block: &GdfBlock,
    source_origin: (usize, usize),
    cols: core::ops::Range<usize>,
) -> Result<()> {
    let geometry_error = || gdf_state_error();
    if !block.height.is_multiple_of(2) || cols.end > block.width {
        return Err(geometry_error());
    }
    let class_cols = block.width >> 1;
    let params = GdfSymParams::new(block);
    let run = cols.len();
    for row in (0..block.height).step_by(2) {
        let class_row = (row >> 1)
            .checked_mul(class_cols)
            .ok_or_else(geometry_error)?;
        let output_row = row
            .checked_mul(block.base_stride)
            .and_then(|row| row.checked_add(block.x))
            .ok_or_else(geometry_error)?;
        let window_rows = source_rows(
            source,
            (source_origin.0 + cols.start, source_origin.1 + row),
            run + 2 * TAP_REACH,
        )?;
        let row_classes = classes
            .get(class_row + (cols.start >> 1)..)
            .ok_or_else(geometry_error)?;
        let (top, bottom) = base_luma
            .get_mut(output_row + cols.start..)
            .and_then(|rows| rows.split_at_mut_checked(block.base_stride))
            .ok_or_else(geometry_error)?;
        let (Some(top), Some(bottom)) = (top.get_mut(..run), bottom.get_mut(..run)) else {
            return Err(geometry_error());
        };
        let mut x = 0;
        while x < run {
            let width = (run - x).min(16);
            let classes = row_classes.get(x >> 1..).ok_or_else(geometry_error)?;
            if let Some(classes) = classes.first_chunk::<8>().filter(|_| width == 16) {
                let window = window_at::<28>(&window_rows, x).ok_or_else(geometry_error)?;
                let output = output_pair::<16>(top, bottom, x).ok_or_else(geometry_error)?;
                sym_rows(&window, output, classes, block, &params);
            } else {
                let output = [&mut top[x..], &mut bottom[x..]];
                sym_rows_narrow(&window_rows, x, width, output, classes, block, &params)
                    .ok_or_else(geometry_error)?;
            }
            x += width;
        }
    }
    Ok(())
}

fn gdf_band_segments(tile: Option<(u32, &[u32])>, luma_width: usize) -> Result<Option<&[u32]>> {
    let Some((tile_cols, mi_col_starts)) = tile else {
        return Ok(None);
    };
    let tile_cols = usize::try_from(tile_cols).map_err(|_| gdf_state_error())?;
    if mi_col_starts.len() != tile_cols.saturating_add(1) || luma_width == 0 {
        return Err(gdf_state_error());
    }
    let mut first_start = None;
    let mut previous_end = None;
    for window in mi_col_starts.windows(2) {
        let (start, width) = gdf_band_segment(window, luma_width)?;
        if previous_end.is_some_and(|end| end != start) {
            return Err(gdf_state_error());
        }
        first_start.get_or_insert(start);
        previous_end = start.checked_add(width);
    }
    if first_start != Some(0) || previous_end != Some(luma_width) {
        return Err(gdf_state_error());
    }
    Ok(Some(mi_col_starts))
}

fn gdf_band_segment(window: &[u32], luma_width: usize) -> Result<(usize, usize)> {
    let start = usize::try_from(window[0])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(gdf_state_error)?
        .min(luma_width);
    let end = usize::try_from(window[1])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(gdf_state_error)?
        .min(luma_width);
    let width = end
        .checked_sub(start)
        .filter(|&width| width != 0)
        .ok_or_else(gdf_state_error)?;
    Ok((start, width))
}

fn tile_axis_bounds(
    starts: &[u32],
    position: usize,
    frame_extent: usize,
) -> Result<(usize, usize)> {
    if position >= frame_extent {
        return Err(gdf_state_error());
    }
    let mi = position / MI_SIZE;
    let window = starts
        .windows(2)
        .find(|window| {
            usize::try_from(window[0]).is_ok_and(|start| start <= mi)
                && usize::try_from(window[1]).is_ok_and(|end| mi < end)
        })
        .ok_or_else(gdf_state_error)?;
    let start = usize::try_from(window[0])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(gdf_state_error)?;
    let end = usize::try_from(window[1])
        .ok()
        .and_then(|value| value.checked_mul(MI_SIZE))
        .map(|end| end.min(frame_extent))
        .and_then(|end| end.checked_sub(1))
        .filter(|&end| position >= start && position <= end)
        .ok_or_else(gdf_state_error)?;
    Ok((start, end))
}

fn source_bounds(
    core: &FrameHeaderCore,
    block: &GdfBlock,
    disable_loopfilters_across_tiles: bool,
) -> Result<LoopRestorationPlaneBounds> {
    let frame_end_x = block
        .frame_width
        .checked_sub(1)
        .ok_or_else(gdf_state_error)?;
    let frame_end_y = block
        .frame_height
        .checked_sub(1)
        .ok_or_else(gdf_state_error)?;
    let (tile_start_x, tile_end_x, tile_start_y, tile_end_y) = if let Some(tile) =
        core.tile_info.as_ref()
    {
        let (start_x, end_x) = tile_axis_bounds(&tile.mi_col_starts, block.x, block.frame_width)?;
        let (start_y, end_y) = tile_axis_bounds(&tile.mi_row_starts, block.y, block.frame_height)?;
        (start_x, end_x, start_y, end_y)
    } else {
        (0, frame_end_x, 0, frame_end_y)
    };
    let local_y = block
        .y
        .checked_sub(tile_start_y)
        .ok_or_else(gdf_state_error)?;
    let stripe_num = local_y
        .checked_add(GDF_TEST_STRIPE_OFF)
        .map(|value| value / GDF_TEST_STRIPE_SIZE)
        .ok_or_else(gdf_state_error)?;
    let stripe_base = stripe_num
        .checked_mul(GDF_TEST_STRIPE_SIZE)
        .and_then(|start| tile_start_y.checked_add(start))
        .ok_or_else(gdf_state_error)?;
    let stripe_start = stripe_base.saturating_sub(GDF_TEST_STRIPE_OFF);
    let stripe_end = stripe_base
        .checked_add(GDF_TEST_STRIPE_SIZE - GDF_TEST_STRIPE_OFF - 1)
        .ok_or_else(gdf_state_error)?;
    let (luma_start_x, luma_end_x, luma_start_y, luma_end_y) = if disable_loopfilters_across_tiles {
        (tile_start_x, tile_end_x, tile_start_y, tile_end_y)
    } else {
        (0, frame_end_x, 0, frame_end_y)
    };
    let bounds = LoopRestorationSourceBounds {
        luma_start_x,
        luma_end_x,
        luma_start_y,
        luma_end_y,
        luma_stripe_start_y: stripe_start.max(luma_start_y).min(luma_end_y),
        luma_stripe_end_y: stripe_end.min(luma_end_y),
        subsampling_x: 0,
        subsampling_y: 0,
    };
    LoopRestorationPlaneBounds::new(PlaneId::Y, &bounds).map_err(|_| gdf_state_error())
}

/// [`resize_overwrite_scratch`] that never shrinks `buffer`, so that a
/// later longer use refills nothing.
fn grow_scratch<T: Clone + Default>(buffer: &mut Vec<T>, len: usize) -> Result<()> {
    if len > buffer.len() {
        resize_overwrite_scratch(buffer, len)?;
    }
    Ok(())
}

fn resize_overwrite_scratch<T: Clone + Default>(buffer: &mut Vec<T>, len: usize) -> Result<()> {
    if len > buffer.len() {
        let additional = len - buffer.len();
        if buffer.try_reserve_exact(additional).is_err() {
            buffer.clear();
            return Err(gdf_allocation_error());
        }
        buffer.resize(len, T::default());
    } else {
        buffer.truncate(len);
    }
    Ok(())
}

#[cfg(test)]
struct GdfSource<'a> {
    /// Luma samples; 8-bit samples are scaled to 10 bits so taps need no shift.
    samples: &'a [u16],
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

/// One § 7.20.5 source row: deblocked or CDEF samples.
struct GdfSourceRow<'a, T>(&'a [T]);

impl<T: ReconSample> GdfSourceRow<'_, T> {
    const fn len(&self) -> usize {
        self.0.len()
    }

    fn copy_range_as<U: ReconSample>(
        &self,
        start: usize,
        dst: &mut [u16],
        shift: u32,
    ) -> Option<()> {
        let src = self.0.get(start..start.checked_add(dst.len())?)?;
        let max = if let Some(src) = T::u8_slice(src) {
            simd::copy_shifted(src, dst, shift)
        } else {
            simd::copy_shifted(T::u16_slice(src)?, dst, shift)
        };
        (max <= U::MAX_VALUE).then_some(())
    }
}

/// The § 7.20.5 source rows of a stripe: deblocked rows, or the CDEF rows of
/// the stripe or of its fringes.
fn stripe_rows<'b, T: ReconSample>(
    deblocked_luma: FramePlane<'b, T>,
    cdef_luma: &'b StripePlane<T>,
    cdef_overlap: &'b [StripePlane<T>],
) -> impl Fn(LoopRestorationSource, usize) -> Option<GdfSourceRow<'b, T>> {
    move |source, y| match source {
        LoopRestorationSource::CurrFrame => deblocked_luma.row(y).map(GdfSourceRow),
        LoopRestorationSource::CdefFrame => cdef_luma
            .row(y)
            .or_else(|| cdef_overlap.iter().find_map(|plane| plane.row(y)))
            .map(GdfSourceRow),
    }
}

#[cfg(test)]
impl<'a> GdfSource<'a> {
    fn materialize<T: ReconSample>(
        samples: &'a mut Vec<u16>,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        bounds: &LoopRestorationSourceBounds,
        block: &GdfBlock,
    ) -> Result<Self> {
        let bounds =
            LoopRestorationPlaneBounds::new(PlaneId::Y, bounds).map_err(|_| gdf_state_error())?;
        Self::materialize_rows::<u16, T>(samples, &bounds, block, |source, y| {
            let plane = match source {
                LoopRestorationSource::CurrFrame => curr_luma,
                LoopRestorationSource::CdefFrame => cdef_luma,
            };
            let start = y.checked_mul(block.frame_width)?;
            plane
                .get(start..start.checked_add(block.frame_width)?)
                .map(GdfSourceRow)
        })
    }

    fn materialize_rows<'b, S: ReconSample, T: ReconSample>(
        samples: &'a mut Vec<u16>,
        bounds: &LoopRestorationPlaneBounds,
        block: &GdfBlock,
        source_row: impl Fn(LoopRestorationSource, usize) -> Option<GdfSourceRow<'b, S>>,
    ) -> Result<Self> {
        let (width, height) = window_size(block)?;
        let sample_count = width.checked_mul(height).ok_or_else(gdf_state_error)?;
        resize_overwrite_scratch(samples, sample_count)?;
        let layout = (width, block.width.max(1), 0);
        let (origin_x, origin_y) = fill_tile::<S, T>(samples, layout, bounds, block, source_row)?;
        Ok(Self {
            samples: samples.as_slice(),
            stride: width,
            origin_x,
            origin_y,
        })
    }

    /// The source window [`fill_tile`] wrote into `samples` at
    /// [`simd::TILE_STRIDE`] for the column tile `block`.
    fn tile(samples: &'a [u16], block: &GdfBlock) -> Result<Self> {
        let radius = isize::try_from(TAP_REACH).map_err(|_| gdf_state_error())?;
        let origin = |at: usize| isize::try_from(at).map(|at| at - radius);
        Ok(Self {
            samples,
            stride: simd::TILE_STRIDE,
            origin_x: origin(block.x).map_err(|_| gdf_state_error())?,
            origin_y: origin(block.y).map_err(|_| gdf_state_error())?,
        })
    }

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

    fn get_at(&self, col: usize, row: usize) -> i32 {
        debug_assert!(col < self.stride);
        self.samples
            .get(row * self.stride + col)
            .map_or(0, |&sample| i32::from(sample))
    }
}

/// The columns and rows of `block`'s source window: the block and the
/// reach of the GDF taps around it.
fn window_size(block: &GdfBlock) -> Result<(usize, usize)> {
    let width = block.width.checked_add(TAP_REACH * 2);
    let height = block.height.checked_add(TAP_REACH * 2);
    width.zip(height).ok_or_else(gdf_state_error)
}

/// Writes the source window of column tile `tile` of `block`, the
/// `tile_cols` columns from `block.x + tile * tile_cols`, into `samples`,
/// rows `stride` apart, and returns its top-left frame position. Each row
/// is the § 7.20.2 source row; a row that § 7.20.2 clips to the row above
/// copies that row, and the columns it clips repeat the row's edge sample.
#[inline(never)]
fn fill_tile<'b, S: ReconSample, T: ReconSample>(
    samples: &mut [u16],
    (stride, tile_cols, tile): (usize, usize, usize),
    bounds: &LoopRestorationPlaneBounds,
    block: &GdfBlock,
    source_row: impl Fn(LoopRestorationSource, usize) -> Option<GdfSourceRow<'b, S>>,
) -> Result<(isize, isize)> {
    let skipped = tile.checked_mul(tile_cols).ok_or_else(gdf_state_error)?;
    let cols = block.width.saturating_sub(skipped).min(tile_cols);
    let (width, height) = (cols + 2 * TAP_REACH, window_size(block)?.1);
    let x = isize::try_from(block.x + skipped).map_err(|_| gdf_state_error())?;
    let y = isize::try_from(block.y).map_err(|_| gdf_state_error())?;
    let (origin_x, origin_y) = (x - TAP_REACH as isize, y - TAP_REACH as isize);
    let left = bounds.sample(isize::MIN, y).x;
    let right = bounds.sample(isize::MAX, y).x;
    let pre = usize::try_from(left as isize - origin_x)
        .unwrap_or(0)
        .min(width);
    let post = usize::try_from(origin_x + width as isize - 1 - right as isize)
        .unwrap_or(0)
        .min(width - pre);
    let mid = width - pre - post;
    let mid_start = usize::try_from(origin_x + pre as isize).map_err(|_| gdf_state_error())?;
    if cols == 0
        || mid == 0
        || right >= block.frame_width
        || stride < width
        || samples.len() < (height - 1) * stride + width
    {
        return Err(gdf_state_error());
    }
    let shift = u32::from(10 - block.bit_depth.bits().min(10));
    let mut previous = None;
    for (row, y) in (origin_y..).take(height).enumerate() {
        let start = row * stride;
        let at = bounds.sample(left as isize, y);
        if previous == Some((at.source, at.y)) {
            samples.copy_within(start - stride..start - stride + width, start);
            continue;
        }
        previous = Some((at.source, at.y));
        let source_row = source_row(at.source, at.y)
            .filter(|row| row.len() == block.frame_width)
            .ok_or_else(gdf_state_error)?;
        let dst = samples
            .get_mut(start..start + width)
            .ok_or_else(gdf_state_error)?;
        let (pads, rest) = dst.split_at_mut(pre);
        let (copied, posts) = rest.split_at_mut(mid);
        source_row
            .copy_range_as::<T>(mid_start, copied, shift)
            .ok_or_else(gdf_state_error)?;
        pads.fill(copied[0]);
        posts.fill(copied[mid - 1]);
    }
    Ok((origin_x, origin_y))
}

#[cfg(test)]
fn band_gradients(
    source: &GdfSource<'_>,
    source_origin: (usize, usize),
    rows: usize,
    cols: usize,
    grad: &mut Vec<u16>,
) -> Result<()> {
    let plane = rows.checked_mul(cols).ok_or_else(gdf_state_error)?;
    let len = GDF_DIRECTIONS
        .checked_mul(plane)
        .ok_or_else(gdf_state_error)?;
    resize_overwrite_scratch(grad, len)?;
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

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GdfClass(i32);

impl GdfClass {
    #[cfg(test)]
    fn new(index: u8, gradient_bias: i32) -> Self {
        Self((gradient_bias << 2) | i32::from(index))
    }

    #[cfg(test)]
    fn index(self) -> u8 {
        (self.0 & 3) as u8
    }

    #[cfg(test)]
    fn gradient_bias(self) -> i32 {
        self.0 >> 2
    }
}

#[cfg(test)]
fn band_classes(
    grad: &[u16],
    grad_cols: usize,
    block: &GdfBlock,
    classes: &mut Vec<GdfClass>,
) -> Result<()> {
    let class_cols = block.width >> 1;
    let class_rows = block.height >> 1;
    let plane = block
        .height
        .checked_add(2)
        .and_then(|rows| rows.checked_mul(grad_cols))
        .ok_or_else(gdf_state_error)?;
    let grad_len = GDF_DIRECTIONS
        .checked_mul(plane)
        .ok_or_else(gdf_state_error)?;
    if grad.len() != grad_len {
        return Err(gdf_state_error());
    }
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let len = class_rows
        .checked_mul(class_cols)
        .ok_or_else(gdf_state_error)?;
    resize_overwrite_scratch(classes, len)?;
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
                let comb = ((strength >> 4) as i32).min(i32::from(alpha));
                gradient_bias += comb * i32::from(weight_table[2][k][cls]);
            }
            classes[i * class_cols + j] = GdfClass::new(index, gradient_bias);
        }
    }
    Ok(())
}

#[cfg(test)]
fn band_classes_from_source(
    source: &GdfSource<'_>,
    source_origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut Vec<GdfClass>,
    scratch: &mut ClassScratch,
) -> Result<()> {
    if source_origin.0 == 0
        || source_origin.1 == 0
        || block.width == 0
        || !block.width.is_multiple_of(2)
        || !block.height.is_multiple_of(2)
    {
        return Err(gdf_state_error());
    }
    let class_cols = block.width >> 1;
    let len = (block.height >> 1)
        .checked_mul(class_cols)
        .ok_or_else(gdf_state_error)?;
    resize_overwrite_scratch(classes, len)?;
    resize_overwrite_scratch(&mut scratch.grads, 2 * ClassScratch::chunks(class_cols))?;
    simd::band_classes(source, source_origin, block, classes, scratch).ok_or_else(gdf_state_error)
}

/// Every table gives the gradient taps one clip bound and weight for all classes.
const _: () = {
    let mut index = 0;
    while index < 6 * 6 * GDF_DIRECTIONS * 4 {
        let (reference, qp) = (
            index / (6 * GDF_DIRECTIONS * 4),
            index / (GDF_DIRECTIONS * 4) % 6,
        );
        let (k, class) = (GDF_COORDS.len() + index / 4 % GDF_DIRECTIONS, index % 4);
        let (alpha, weight) = (
            &GDF_ALPHA[reference][qp][k],
            &GDF_WEIGHT[reference][qp][2][k],
        );
        assert!(alpha[class] == alpha[0] && weight[class] == weight[0]);
        assert!(GDF_WEIGHT[reference][qp][0][k][class] == 0);
        assert!(GDF_WEIGHT[reference][qp][1][k][class] == 0);
        index += 1;
    }
};

#[cfg(test)]
fn gdf_tap_offsets(stride: usize) -> Result<[usize; GDF_COORDS.len()]> {
    let stride = isize::try_from(stride).map_err(|_| gdf_state_error())?;
    let mut offsets = [0_usize; GDF_COORDS.len()];
    for (slot, &(dy, dx)) in offsets.iter_mut().zip(GDF_COORDS.iter()) {
        *slot = dy
            .checked_mul(stride)
            .and_then(|row| row.checked_add(dx))
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(gdf_state_error)?;
    }
    Ok(offsets)
}

#[cfg(test)]
fn exact_slice<T>(samples: &[T], start: usize, len: usize) -> Option<&[T]> {
    samples.get(start..)?.get(..len)
}

#[cfg(test)]
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
    let cls = usize::from(class.index());
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
    let mut gdf_idx = [0, 0, class.gradient_bias()];
    for (k, &tap) in tap_offsets.iter().enumerate() {
        let alpha = i32::from(alpha_table[k][cls]);
        let sample3 = i32::from(samples[base - tap]);
        let sample4 = i32::from(samples[base + tap]);
        let above = (sample3 - sample2).clamp(-alpha, alpha);
        let below = (sample4 - sample2).clamp(-alpha, alpha);
        let comb = (above + below).clamp(-512, 511);
        for (idx, total) in gdf_idx.iter_mut().enumerate() {
            *total += comb * i32::from(weight_table[idx][k][cls]);
        }
    }
    let base = block
        .y
        .checked_add(row)
        .and_then(|y| y.checked_mul(block.base_stride))
        .and_then(|index| index.checked_add(block.x + col))
        .and_then(|index| base_luma.get(index))
        .copied()
        .map(i32::from)
        .unwrap_or_default();
    finish_gdf_sample(base, block, &gdf_idx)
}

#[cfg(test)]
#[allow(clippy::inline_always)]
#[inline(always)]
fn finish_gdf_sample(base: i32, block: &GdfBlock, gdf_idx: &[i32; 3]) -> u16 {
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        finish_gdf_sample_with_error::<8, 4096>(
            base,
            block,
            &GDF_INTRA_ERROR[block.qp_idx],
            gdf_idx,
        )
    } else {
        finish_gdf_sample_with_error::<5, 1000>(
            base,
            block,
            &GDF_INTER_ERROR[block.ref_dst_idx - 1][block.qp_idx],
            gdf_idx,
        )
    }
}

#[cfg(test)]
#[allow(clippy::inline_always)]
#[inline(always)]
fn finish_gdf_sample_with_error<const SCALE: i32, const ERROR_LEN: usize>(
    base: i32,
    block: &GdfBlock,
    error: &[i32; ERROR_LEN],
    gdf_idx: &[i32; 3],
) -> u16 {
    let mut pos = 0_usize;
    for (idx, value) in gdf_idx.iter().enumerate() {
        let biased = (*value + GDF_BIAS[block.ref_dst_idx][block.qp_idx][idx]) * SCALE;
        let v = round2_signed_i32(biased, 15);
        let digit = v.clamp(-SCALE, SCALE - 1) + SCALE;
        pos = pos * (SCALE as usize * 2) + usize::try_from(digit).unwrap_or_default();
    }
    let residual = round2_signed_i32(
        error[pos] * block.pix_scale,
        12 - u32::from(block.bit_depth.bits()),
    );
    (base + residual).clamp(0, block.max_sample) as u16
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
) -> Result<usize> {
    let qp_base = if frame_is_intra { 85_i32 } else { 110_i32 };
    let qp_diff = i32::try_from(base_q_idx).map_err(|_| gdf_state_error())?
        - qp_base
        - 24 * (i32::from(bit_depth.bits()) - 8);
    let qp_bucket = ((qp_diff - 37) / 25).clamp(0, 2);
    let qc_idx = i32::from(pic_qc_idx.unwrap_or(0));
    usize::try_from(qp_bucket + qc_idx).map_err(|_| gdf_state_error())
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

#[cold]
#[inline(never)]
fn gdf_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidGdfFilterState.into()
}

#[cold]
#[inline(never)]
fn gdf_allocation_error() -> crate::error::DecodeError {
    splot_recon::ReconError::WorkspaceAllocationFailed {
        plane: splot_recon::PlaneId::Y,
        context: "GDF filter scratch",
    }
    .into()
}

#[cfg(test)]
#[path = "gdf_wide_tests.rs"]
mod wide_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::{
        ClassScratch, GDF_COORDS, GDF_WEIGHT, GdfBlock, GdfBlockGrid, GdfClass,
        GdfReferenceContext, GdfSource, GdfSourceRow, LoopRestorationSource, RESTRICTED_ORDER_HINT,
        TAP_REACH, band_classes, band_classes_from_source, band_gradients, compute_enabled_segment,
        fill_tile, gdf_band_segment, gdf_band_segments, gdf_inter_ref_dst_idx,
        gdf_inter_ref_dst_idx_from_max_dist, gdf_stripe_end_for_tile, gdf_stripe_row_for_tile_end,
        grad_sum, lossless_spans, resize_overwrite_scratch, simd, tile_axis_bounds, window_size,
    };
    use crate::error::{DecodeError, DecodeHeaderStateError};
    use crate::filters::deblock::DeblockBlock;
    use crate::filters::lossless::LosslessBlockGrid;
    use splot_recon::{BitDepth, LoopRestorationPlaneBounds, LoopRestorationSourceBounds, PlaneId};

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
        let grid = result.expect("valid result");

        assert_eq!(grid.enabled(0, 0), Some(true));
        assert_eq!(grid.enabled(0, 64), Some(true));
        assert_eq!(grid.enabled(16, 0), Some(false));
        assert_eq!(grid.enabled(16, 64), Some(true));
    }

    #[test]
    fn per_block_grid_runs_cover_only_enabled_units() {
        let result = GdfBlockGrid::new(64, 1, 5, vec![0, 1, 1, 0, 1]);
        let grid = result.expect("valid result");

        assert_eq!(grid.enabled_run(0, 0, 320), Some(64..192));
        assert_eq!(grid.enabled_run(0, 192, 320), Some(256..320));
        assert_eq!(grid.enabled_run(0, 100, 150), Some(100..150));
        assert_eq!(grid.enabled_run(0, 0, 60), Some(60..60));
        assert_eq!(grid.enabled_run(0, 192, 256), Some(256..256));
        assert_eq!(grid.enabled_run(0, 256, 324), None);
        assert_eq!(grid.enabled_run(16, 0, 64), None);
    }

    #[test]
    fn per_block_grid_rejects_invalid_shape_and_values() {
        assert!(GdfBlockGrid::new(0, 1, 1, vec![1]).is_none());
        assert!(GdfBlockGrid::new(62, 1, 1, vec![1]).is_none());
        assert!(GdfBlockGrid::new(64, 2, 2, vec![1, 0, 1]).is_none());
        assert!(GdfBlockGrid::new(64, 1, 1, vec![2]).is_none());
    }

    #[test]
    fn stripe_row_switches_before_the_next_sixty_four_pixel_band() {
        assert_eq!(gdf_stripe_row_for_tile_end(52, 127).ok(), Some(0));
        assert_eq!(gdf_stripe_row_for_tile_end(56, 127).ok(), Some(16));
        assert_eq!(gdf_stripe_row_for_tile_end(124, 127).ok(), Some(31));
        assert_eq!(gdf_stripe_row_for_tile_end(60, 63).ok(), Some(15));
        assert_eq!(gdf_stripe_row_for_tile_end(64, 127).ok(), Some(16));
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

        assert_eq!(tile_axis_bounds(&starts, 0, 128).ok(), Some((0, 63)));
        assert_eq!(tile_axis_bounds(&starts, 63, 128).ok(), Some((0, 63)));
        assert_eq!(tile_axis_bounds(&starts, 64, 128).ok(), Some((64, 127)));
        assert_eq!(tile_axis_bounds(&starts, 127, 128).ok(), Some((64, 127)));
        assert!(tile_axis_bounds(&starts, 128, 128).is_err());
        assert!(tile_axis_bounds(&[0, 16], 64, 128).is_err());
    }

    #[test]
    fn gdf_band_segments_visit_tile_columns_in_exact_order() -> crate::Result<()> {
        let starts = gdf_band_segments(Some((3, &[0, 16, 24, 32])), 128)?
            .ok_or_else(super::gdf_state_error)?;
        let mut actual = [None; 3];
        let mut len = 0;
        let result = starts
            .windows(2)
            .try_for_each(|window| -> crate::Result<()> {
                let (x, width) = gdf_band_segment(window, 128)?;
                let Some(slot) = actual.get_mut(len) else {
                    return Err(super::gdf_state_error());
                };
                *slot = Some((x, width));
                len += 1;
                Ok(())
            });

        assert!(result.is_ok());
        assert_eq!(len, 3);
        assert_eq!(actual, [Some((0, 64)), Some((64, 32)), Some((96, 32))]);
        assert!(matches!(gdf_band_segments(None, 128), Ok(None)));
        Ok(())
    }

    #[test]
    fn malformed_gdf_band_boundaries_fail_before_visiting_any_segment() {
        let cases: [(&str, &[u32], u32, usize); 8] = [
            ("empty", &[0], 0, 128),
            ("zero width", &[0, 16], 1, 0),
            ("wrong count", &[0, 16], 2, 128),
            ("duplicate", &[0, 16, 16, 32], 3, 128),
            ("out of order", &[0, 24, 16, 32], 3, 128),
            ("nonzero origin", &[4, 16, 32], 2, 128),
            ("incomplete coverage", &[0, 16], 1, 128),
            ("out of range interior", &[0, 40, 64], 2, 128),
        ];

        for (name, starts, tile_cols, width) in cases {
            let mut visits = 0;
            let result = gdf_band_segments(Some((tile_cols, starts)), width).and_then(|segments| {
                if let Some(starts) = segments {
                    starts
                        .windows(2)
                        .try_for_each(|window| -> crate::Result<()> {
                            gdf_band_segment(window, width)?;
                            visits += 1;
                            Ok(())
                        })?;
                }
                Ok(())
            });

            assert!(
                matches!(
                    result,
                    Err(DecodeError::HeaderState {
                        source: DecodeHeaderStateError::InvalidGdfFilterState
                    })
                ),
                "{name} returned {result:?}"
            );
            assert_eq!(visits, 0, "{name} mutated before validation completed");
        }
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn overflowing_gdf_band_boundary_fails_before_visiting_any_segment() {
        let mut visits = 0;
        let result =
            gdf_band_segments(Some((1, &[0, u32::MAX])), usize::MAX).and_then(|segments| {
                if let Some(starts) = segments {
                    visits += starts.windows(2).count();
                }
                Ok(())
            });

        assert!(matches!(
            result,
            Err(DecodeError::HeaderState {
                source: DecodeHeaderStateError::InvalidGdfFilterState
            })
        ));
        assert_eq!(visits, 0);
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
            base_stride: frame_width,
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
        );
        let stripe_source = stripe_result.expect("valid stripe result");

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
                );
                let local_source = local_result.expect("valid local result");
                let radius = TAP_REACH as isize;
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

    /// Every sample of every column tile window is the § 7.20.2 source sample,
    /// scaled to 10 bits, at frame, tile and stripe edges.
    #[test]
    fn tile_windows_match_the_source_sample_process() {
        fn check<S: splot_recon::ReconSample>(
            planes: [&[S]; 2],
            frame: (usize, usize),
            bounds: LoopRestorationSourceBounds,
            (x, y, width, height): (usize, usize, usize, usize),
            bit_depth: BitDepth,
        ) {
            let (frame_width, frame_height) = frame;
            let plane = LoopRestorationPlaneBounds::new(PlaneId::Y, &bounds).expect("bounds");
            let run = GdfBlock {
                x,
                y,
                width,
                height,
                frame_width,
                frame_height,
                base_stride: frame_width,
                bit_depth,
                qp_idx: 0,
                ref_dst_idx: 0,
                pix_scale: 1,
                max_sample: i32::from(bit_depth.max_sample()),
            };
            let select = |source| match source {
                LoopRestorationSource::CurrFrame => planes[0],
                LoopRestorationSource::CdefFrame => planes[1],
            };
            let rows = |source, y: usize| {
                select(source)
                    .get(y * frame_width..(y + 1) * frame_width)
                    .map(GdfSourceRow)
            };
            let shift = 10 - u32::from(bit_depth.bits());
            let area = window_size(&run).expect("window").1 * simd::TILE_STRIDE;
            for index in 0..run.width.div_ceil(simd::TILE_COLS) {
                let mut samples = vec![u16::MAX; area];
                let layout = (simd::TILE_STRIDE, simd::TILE_COLS, index);
                fill_tile::<S, S>(&mut samples, layout, &plane, &run, rows)
                    .expect("valid tile window");
                let tile = GdfBlock {
                    x: run.x + index * simd::TILE_COLS,
                    width: simd::TILE_COLS.min(run.width - index * simd::TILE_COLS),
                    ..run
                };
                let source = GdfSource::tile(&samples, &tile).expect("valid tile");
                let reach = TAP_REACH as isize;
                for y in y as isize - reach..(y + height) as isize + reach {
                    for x in tile.x as isize - reach..(tile.x + tile.width) as isize + reach {
                        let at =
                            splot_recon::loop_restoration_source_sample(PlaneId::Y, x, y, &bounds)
                                .expect("source sample");
                        let expected =
                            select(at.source)[at.y * frame_width + at.x].to_u16() << shift;
                        assert_eq!(
                            source.get(x, y),
                            i32::from(expected),
                            "tile {index} ({x}, {y})"
                        );
                    }
                }
            }
        }
        let bounds = |x: (usize, usize), y: (usize, usize), stripe: (usize, usize)| {
            LoopRestorationSourceBounds {
                luma_start_x: x.0,
                luma_end_x: x.1,
                luma_start_y: y.0,
                luma_end_y: y.1,
                luma_stripe_start_y: stripe.0,
                luma_stripe_end_y: stripe.1,
                subsampling_x: 0,
                subsampling_y: 0,
            }
        };
        let wide = 2 * simd::TILE_COLS + 40;
        let curr: Vec<u16> = (0..wide * 128)
            .map(|index| (index % 1_021) as u16)
            .collect();
        let cdef: Vec<u16> = curr.iter().map(|&sample| 1_023 - sample).collect();
        let frame = (wide, 128);
        let ten = BitDepth::Ten;
        let interior = bounds((0, wide - 1), (0, 127), (56, 119));
        check([&curr, &cdef], frame, interior, (0, 56, wide, 64), ten);
        let last = bounds((0, wide - 1), (0, 127), (120, 127));
        check([&curr, &cdef], frame, last, (8, 120, wide - 16, 8), ten);
        let curr: Vec<u8> = (0..40 * 64).map(|index| (index * 7 % 251) as u8).collect();
        let cdef: Vec<u8> = curr.iter().map(|&sample| 255 - sample).collect();
        let first = bounds((0, 39), (0, 63), (0, 55));
        check(
            [&curr, &cdef],
            (40, 64),
            first,
            (8, 0, 24, 56),
            BitDepth::Eight,
        );
        let tile = bounds((16, 31), (0, 63), (0, 55));
        check(
            [&curr, &cdef],
            (40, 64),
            tile,
            (16, 0, 16, 56),
            BitDepth::Eight,
        );
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
    fn lossless_spans_cover_only_lossless_cells_of_the_run() {
        let cell = |r, c, n4w| DeblockBlock {
            r,
            c,
            prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: r,
                base_c: c,
                default_sub_pu_tx: 0,
            },
            n4w,
            n4h: 1,
            tx: 0,
            sub_pu_size: None,
            qindex: 0,
            skip: false,
            lossless: true,
        };
        let lossless = [cell(1, 1, 2), cell(2, 3, 1)];
        let grid = LosslessBlockGrid::from_deblock_blocks(4, 4, &lossless, &[]);
        let grid = grid.expect("valid grid");
        let run = GdfBlock {
            x: 2,
            y: 2,
            width: 12,
            height: 8,
            frame_width: 16,
            frame_height: 16,
            base_stride: 16,
            bit_depth: BitDepth::Eight,
            qp_idx: 0,
            ref_dst_idx: 0,
            pix_scale: 1,
            max_sample: 255,
        };
        let mut spans = Vec::new();
        let result = lossless_spans(&grid, &run, |start, len| {
            spans.push((start, len));
            Ok(())
        });
        assert!(result.is_ok());
        let expected: Vec<_> = (2..6)
            .map(|row| (row * 16 + 4, 8))
            .chain((6..8).map(|row| (row * 16 + 12, 2)))
            .collect();
        assert_eq!(spans, expected);
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
            base_stride: frame_width,
            bit_depth: BitDepth::Ten,
            qp_idx: 0,
            ref_dst_idx: 0,
            pix_scale: 4,
            max_sample: 1023,
        };
        let mut source_samples = Vec::new();
        let source_result =
            GdfSource::materialize::<u16>(&mut source_samples, &curr, &curr, &bounds, &block);
        let source = source_result.expect("valid source result");
        let origin = source
            .relative_position(block.x, block.y)
            .expect("block origin");
        let grad_cols = block.width + 2;
        let plane = (block.height + 2) * grad_cols;
        let mut grad = Vec::new();
        let grad_result = band_gradients(&source, origin, block.height + 2, grad_cols, &mut grad);
        assert!(grad_result.is_ok());
        assert!(grad.iter().all(|&value| value <= 2046));
        assert!(
            grad.chunks_exact(plane)
                .all(|direction| grad_sum(direction, grad_cols, 0, 0, 4, 4) <= 32_736)
        );

        let mut classes = Vec::new();
        let classes_result = band_classes(&grad, grad_cols, &block, &mut classes);
        assert!(classes_result.is_ok());
        let mut fused_classes = Vec::new();
        let mut class_scratch = ClassScratch::default();
        let fused_result = band_classes_from_source(
            &source,
            origin,
            &block,
            &mut fused_classes,
            &mut class_scratch,
        );
        assert!(fused_result.is_ok());
        assert_eq!(fused_classes, classes);
        let filter = |source: &GdfSource<'_>, classes: &[GdfClass]| {
            let mut base = curr[block.y * frame_width..].to_vec();
            compute_enabled_segment(source, &mut base, classes, &block, origin, 0..block.width)
                .expect("valid filtered");
            base
        };
        let filtered = filter(&source, &classes);
        assert!(filtered.iter().all(|&sample| sample <= 1023));

        let mut reused_source_samples = vec![u16::MAX; source.samples.len() + 8];
        let reused_source = GdfSource::materialize::<u16>(
            &mut reused_source_samples,
            &curr,
            &curr,
            &bounds,
            &block,
        );
        let reused_source = reused_source.expect("valid reused source");
        assert_eq!(reused_source.samples, source.samples);

        let mut reused_classes = vec![GdfClass(i32::MAX); classes.len() + 4];
        let mut reused_class_scratch = ClassScratch {
            grads: vec![[[u16::MAX; 16]; 4]; class_scratch.grads.len() + 3],
            ..ClassScratch::default()
        };
        let reused_result = band_classes_from_source(
            &reused_source,
            origin,
            &block,
            &mut reused_classes,
            &mut reused_class_scratch,
        );
        assert!(reused_result.is_ok());
        assert_eq!(reused_classes, classes);
        let reused_filtered = filter(&reused_source, &reused_classes);
        assert_eq!(reused_filtered, filtered);
    }

    #[test]
    fn overwrite_scratch_resize_reuses_initialized_storage() {
        let mut scratch = Vec::new();
        assert!(resize_overwrite_scratch(&mut scratch, 8).is_ok());
        scratch.fill(9_u16);
        let capacity = scratch.capacity();

        assert!(resize_overwrite_scratch(&mut scratch, 4).is_ok());
        assert_eq!(scratch, vec![9; 4]);
        assert_eq!(scratch.capacity(), capacity);

        assert!(resize_overwrite_scratch(&mut scratch, 6).is_ok());
        assert_eq!(scratch, [9, 9, 9, 9, 0, 0]);
    }

    #[test]
    fn scratch_capacity_overflow_is_typed() {
        let mut scratch = vec![1_u8];
        let result = resize_overwrite_scratch(&mut scratch, usize::MAX);

        assert!(matches!(
            result,
            Err(DecodeError::Reconstruction {
                source: splot_recon::ReconError::WorkspaceAllocationFailed { .. },
            })
        ));
        assert!(scratch.is_empty());
    }
}
