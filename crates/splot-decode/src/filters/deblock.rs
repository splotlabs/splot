// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{DeblockingFilterParams, QuantizationParams, TileInfo};
use splot_core::tables::conversion::{
    Q_FIRST, Q_THRESH_MULTS, SIDE_THRESHOLDS, TX_HEIGHT, TX_WIDTH, W_MULT,
};
use splot_parallel::prelude::*;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DeblockFilterChoice, DeblockSampleFilter, PixelFormat,
    PlaneId, ReconSample, deblock_adaptive_filter_strength, deblock_filter_choice,
    deblock_filter_choice_strided, deblock_filter_max_width, deblock_sample_filter,
    deblock_sample_filter_strided, deblock_sample_filter_strided_4, deblock_side_threshold_index,
    max_quantizer_index,
};
use std::num::NonZeroUsize;

const MI_SIZE: usize = 4;

const SB_SIZE: usize = 64;

const VERTICAL_TX_CANDIDATE: u8 = 1;
const HORIZONTAL_TX_CANDIDATE: u8 = 2;
const SUB_PU_CANDIDATE: u8 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeblockPredictionUnit {
    pub(crate) base_r: usize,
    pub(crate) base_c: usize,
    pub(crate) default_sub_pu_tx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeblockSubPuSize {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl DeblockSubPuSize {
    pub(crate) const fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    pub(crate) const fn square(size: usize) -> Self {
        Self::new(size, size)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DeblockBlock {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) luma_prediction: DeblockPredictionUnit,
    pub(crate) chroma_prediction: DeblockPredictionUnit,
    pub(crate) chroma_base_r: usize,
    pub(crate) chroma_base_c: usize,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) luma_tx: usize,
    pub(crate) chroma_tx: Option<usize>,
    pub(crate) sub_pu_size: Option<DeblockSubPuSize>,
    pub(crate) chroma_transform_only: bool,
    pub(crate) qindex: u32,
    pub(crate) skip: bool,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeblockQuantDeltas {
    u_ac: i32,
    v_ac: i32,
}

impl DeblockQuantDeltas {
    pub(crate) const ZERO: Self = Self { u_ac: 0, v_ac: 0 };

    pub(crate) const fn from_frame_quant(
        quant: QuantizationParams,
        base_uv_ac_delta_q: i32,
    ) -> Self {
        Self {
            u_ac: quant.delta_q_u_ac + base_uv_ac_delta_q,
            v_ac: quant.delta_q_v_ac + base_uv_ac_delta_q,
        }
    }

    const fn ac_delta(self, plane: usize) -> i32 {
        match plane {
            1 => self.u_ac,
            2 => self.v_ac,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy)]
struct EdgeBlock<'a> {
    block: &'a DeblockBlock,
    chroma_transform: Option<&'a DeblockBlock>,
}

impl EdgeBlock<'_> {
    fn prediction(self, plane: usize) -> DeblockPredictionUnit {
        if plane == 0 {
            self.block.luma_prediction
        } else {
            self.block.chroma_prediction
        }
    }

    fn tx_base(self, plane: usize) -> (usize, usize) {
        if plane == 0 {
            (self.block.r, self.block.c)
        } else if let Some(transform) = self.chroma_transform {
            (transform.chroma_base_r, transform.chroma_base_c)
        } else {
            (self.block.chroma_base_r, self.block.chroma_base_c)
        }
    }

    fn tx(self, plane: usize) -> usize {
        if plane == 0 {
            self.block.luma_tx
        } else {
            self.chroma_transform
                .and_then(|transform| transform.chroma_tx)
                .or(self.block.chroma_tx)
                .unwrap_or(0)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn deblock_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    blocks: &[DeblockBlock],
    chroma_blocks: [&[DeblockBlock]; 2],
    mi_rows: usize,
    mi_cols: usize,
    filter: DeblockingFilterParams,
    tile_info: Option<&TileInfo>,
    disable_loopfilters_across_tiles: bool,
    quant_deltas: DeblockQuantDeltas,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    if filter.apply_deblocking_filter == [false; 4] {
        return Ok(());
    }

    let grid = build_mi_grid(blocks, mi_rows, mi_cols)?;
    let pixel_format = workspace.info().pixel_format();
    for plane in 0..3 {
        if plane != 0 && !filter.apply_deblocking_filter[plane + 1] {
            continue;
        }
        let chroma_grid = if plane != 0 {
            Some(overlay_mi_grid(
                &grid,
                chroma_blocks[plane - 1],
                mi_rows,
                mi_cols,
            )?)
        } else {
            None
        };
        let plane_grid = chroma_grid.as_ref().unwrap_or(&grid);
        for pass in 0..2usize {
            let Some(plane_pass) =
                PlanePass::active(plane, pass, filter, quant_deltas, bit_depth, pixel_format)
            else {
                continue;
            };
            deblock_plane_pass(
                workspace,
                plane_grid,
                plane_pass,
                mi_rows,
                mi_cols,
                tile_info,
                disable_loopfilters_across_tiles,
            )?;
        }
        if let Some(chroma_grid) = chroma_grid {
            let (cells, candidates) = chroma_grid.into_scratch();
            recycle_deblock_grid_scratch(cells, candidates);
        }
    }
    let (cells, candidates) = grid.into_scratch();
    recycle_deblock_grid_scratch(cells, candidates);

    Ok(())
}

pub(crate) fn deblock_tip_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    luma_unit_size: usize,
    quant: QuantizationParams,
    base_uv_ac_delta_q: i32,
    tile_starts: Option<(&[u32], &[u32])>,
    disable_loopfilters_across_tiles: bool,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let pixel_format = workspace.info().pixel_format();
    for plane in 0..3 {
        let plane_id = plane_index_to_id(plane);
        if workspace.plane(plane_id).is_err() {
            continue;
        }
        let (sub_x, sub_y) = if plane == 0 {
            (0, 0)
        } else {
            (
                usize::from(pixel_format.subsampling_x()),
                usize::from(pixel_format.subsampling_y()),
            )
        };
        let unit_width = (luma_unit_size >> sub_x).max(1);
        let unit_height = (luma_unit_size >> sub_y).max(1);
        let quant_delta = match plane {
            1 => quant.delta_q_u_ac + base_uv_ac_delta_q,
            2 => quant.delta_q_v_ac + base_uv_ac_delta_q,
            _ => 0,
        };
        let (q_thr, side) = adaptive_strength(
            q_clamped(quant.base_q_idx, quant_delta, bit_depth),
            bit_depth,
        );
        let (width, height) = coded_plane_dimensions(workspace, plane_id)?;
        let mut frame = workspace.as_frame_mut();
        let view = frame.plane_mut(plane_id).ok_or(DeblockError::Workspace)?;
        let stride = view.stride_samples();
        let mut plane_ctx = PlaneCtx::new(view.samples_mut(), stride, width, height)?;
        for y in (0..height).step_by(MI_SIZE) {
            for x in (unit_width..width).step_by(unit_width) {
                let tile_edge = tip_tile_edge(tile_starts.map(|(cols, _)| cols), x, sub_x);
                if disable_loopfilters_across_tiles && tile_edge {
                    continue;
                }
                let (max_width_neg, max_width_pos) =
                    deblock_filter_max_width(unit_width, plane != 0, tile_edge);
                apply_tip_filter_edge(
                    &mut plane_ctx,
                    x,
                    y,
                    1,
                    0,
                    height.saturating_sub(y).min(MI_SIZE),
                    q_thr,
                    side,
                    max_width_neg,
                    max_width_pos,
                    bit_depth,
                )?;
            }
        }
        for x in (0..width).step_by(MI_SIZE) {
            for y in (unit_height..height).step_by(unit_height) {
                let tile_edge = tip_tile_edge(tile_starts.map(|(_, rows)| rows), y, sub_y);
                if disable_loopfilters_across_tiles && tile_edge {
                    continue;
                }
                let (max_width_neg, max_width_pos) = deblock_filter_max_width(
                    unit_height,
                    plane != 0,
                    y.is_multiple_of(64 >> sub_y),
                );
                apply_tip_filter_edge(
                    &mut plane_ctx,
                    x,
                    y,
                    0,
                    1,
                    width.saturating_sub(x).min(MI_SIZE),
                    q_thr,
                    side,
                    max_width_neg,
                    max_width_pos,
                    bit_depth,
                )?;
            }
        }
    }
    Ok(())
}

fn tip_tile_edge(starts: Option<&[u32]>, coordinate: usize, subsampling: usize) -> bool {
    let Some(starts) = starts else {
        return false;
    };
    let Some(luma_coordinate) = coordinate.checked_mul(1 << subsampling) else {
        return false;
    };
    let Ok(mi_coordinate) = u32::try_from(luma_coordinate / MI_SIZE) else {
        return false;
    };
    starts
        .get(1..starts.len().saturating_sub(1))
        .is_some_and(|starts| starts.contains(&mi_coordinate))
}

#[allow(clippy::too_many_arguments)]
fn apply_tip_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
    lanes: usize,
    q_thr: i32,
    side: i32,
    max_width_neg: usize,
    max_width_pos: usize,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let width = choose_filter_width(
        plane_ctx,
        x,
        y,
        dx,
        dy,
        q_thr,
        side,
        max_width_neg,
        max_width_pos,
    )?;
    if width == 0 {
        return Ok(());
    }
    let eff_neg = width.min(max_width_neg);
    let eff_pos = width.min(max_width_pos);
    let params = DeblockSampleFilter {
        boundary: GATHER_HALF,
        q_thr,
        max_width_neg: eff_neg,
        max_width_pos: eff_pos,
        q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
        w_mult_neg: W_MULT[eff_neg - 1],
        w_mult_pos: W_MULT[eff_pos - 1],
        prev_lossless: false,
        curr_lossless: false,
        bit_depth,
    };
    apply_edge_samples(plane_ctx, PerpLine::new(x, y, dx, dy), lanes, params)
}

fn deblock_plane_pass<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    grid: &MiGrid,
    plane_pass: PlanePass,
    mi_rows: usize,
    mi_cols: usize,
    tile_info: Option<&TileInfo>,
    disable_loopfilters_across_tiles: bool,
) -> Result<(), DeblockError> {
    let tile_starts = tile_info.and_then(|tile_info| {
        let starts = if plane_pass.pass == 0 {
            &tile_info.mi_col_starts
        } else {
            &tile_info.mi_row_starts
        };
        starts
            .get(1..starts.len().saturating_sub(1))
            .filter(|starts| !starts.is_empty())
    });
    let plane_id = plane_pass.plane_id;
    if workspace.plane(plane_id).is_err() {
        return Ok(());
    }
    let (width, height) = coded_plane_dimensions(workspace, plane_id)?;
    let mut frame = workspace.as_frame_mut();
    let Some(view) = frame.plane_mut(plane_id) else {
        return Ok(());
    };
    let stride = view.stride_samples();
    let samples = view.samples_mut();

    let covered_rows = (mi_rows * MI_SIZE) >> plane_pass.plane_sub_y;
    let covered_cols = (mi_cols * MI_SIZE) >> plane_pass.plane_sub_x;
    let use_parallel = splot_parallel::on_multiworker_pool()
        && (plane_pass.pass == 0 && covered_rows <= height
            || plane_pass.pass == 1 && covered_cols <= width);
    if !use_parallel {
        return deblock_plane_pass_serial(
            samples,
            stride,
            width,
            height,
            grid,
            plane_pass,
            mi_rows,
            mi_cols,
            tile_starts,
            disable_loopfilters_across_tiles,
        );
    }

    let workers = splot_parallel::current_pool_width();
    let timer = crate::timing::start();
    let tally = crate::timing::WorkerTally::new();
    let samples = validated_plane_samples(samples, stride, width, height)?;
    let process_band = |unit_start: usize, unit_end: usize, mut ctx: PlaneCtx<'_, '_, T>| {
        tally.note_worker();
        let mut strengths = StrengthCache::default();
        if plane_pass.pass == 0 {
            for unit in unit_start..unit_end {
                let r = unit * plane_pass.row_step;
                if r >= mi_rows {
                    break;
                }
                for c in (0..mi_cols).step_by(plane_pass.col_step) {
                    if !grid.is_candidate(
                        r,
                        c,
                        plane_pass.pass,
                        plane_pass.allow_df_sub_pu,
                        plane_pass.plane_sub_x,
                        plane_pass.plane_sub_y,
                    ) {
                        continue;
                    }
                    deblock_filter_edge(
                        &mut ctx,
                        grid,
                        plane_pass.edge_context(r, c, tile_starts),
                        disable_loopfilters_across_tiles,
                        &mut strengths,
                    )?;
                }
            }
        } else {
            for unit in unit_start..unit_end {
                let c = unit * plane_pass.col_step;
                if c >= mi_cols {
                    break;
                }
                for r in (0..mi_rows).step_by(plane_pass.row_step) {
                    if !grid.is_candidate(
                        r,
                        c,
                        plane_pass.pass,
                        plane_pass.allow_df_sub_pu,
                        plane_pass.plane_sub_x,
                        plane_pass.plane_sub_y,
                    ) {
                        continue;
                    }
                    deblock_filter_edge(
                        &mut ctx,
                        grid,
                        plane_pass.edge_context(r, c, tile_starts),
                        disable_loopfilters_across_tiles,
                        &mut strengths,
                    )?;
                }
            }
        }
        Ok(())
    };
    let (result, band_count) = if plane_pass.pass == 0 {
        let plane_units = height.div_ceil(MI_SIZE);
        let units_per_band = plane_units.div_ceil(workers * 4).max(1);
        let rows_per_band = units_per_band * MI_SIZE;
        let samples_per_band = rows_per_band
            .checked_mul(stride)
            .ok_or(DeblockError::Workspace)?;
        let band_count = height.div_ceil(rows_per_band);
        let result = samples
            .par_chunks_mut(samples_per_band)
            .enumerate()
            .try_for_each(|(band, band_samples)| {
                let unit_start = band * units_per_band;
                let y_origin = unit_start * MI_SIZE;
                process_band(
                    unit_start,
                    unit_start + units_per_band,
                    PlaneCtx::contiguous_band(band_samples, stride, width, height, 0, y_origin),
                )
            });
        (result, band_count)
    } else {
        let plane_units = width.div_ceil(MI_SIZE);
        let units_per_band = plane_units.div_ceil(workers * 2).max(1);
        let band_count = plane_units.div_ceil(units_per_band);
        let cols_per_band = units_per_band * MI_SIZE;
        let split_row_count = band_count
            .checked_mul(height)
            .ok_or(DeblockError::Workspace)?;
        let row_count = height
            .checked_add(split_row_count)
            .ok_or(DeblockError::Workspace)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(row_count)
            .map_err(|_| DeblockError::Workspace)?;
        for row in samples.chunks_mut(stride) {
            rows.push(&mut row[..width]);
        }
        for _ in 0..band_count {
            for row in 0..height {
                let available = core::mem::take(&mut rows[row]);
                let split_at = cols_per_band.min(available.len());
                let (column, remainder) = available.split_at_mut(split_at);
                rows[row] = remainder;
                rows.push(column);
            }
        }
        let (_, split_rows) = rows.split_at_mut(height);
        let result = split_rows
            .par_chunks_mut(height.max(1))
            .enumerate()
            .try_for_each(|(band, rows)| {
                let unit_start = band * units_per_band;
                let x_origin = unit_start * MI_SIZE;
                process_band(
                    unit_start,
                    unit_start + units_per_band,
                    PlaneCtx::split_band(rows, width, height, x_origin, 0),
                )
            });
        (result, band_count)
    };
    if timer.is_some() {
        crate::timing::report_detail(
            "deblock_pass_bands",
            timer,
            &format!(
                "plane={} pass={} units={band_count} threads={workers} workers_used={}",
                plane_pass.plane,
                plane_pass.pass,
                tally.workers_used()
            ),
        );
    }
    result
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn deblock_plane_pass_serial<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    width: usize,
    height: usize,
    grid: &MiGrid<'_>,
    plane_pass: PlanePass,
    mi_rows: usize,
    mi_cols: usize,
    tile_starts: Option<&[u32]>,
    disable_loopfilters_across_tiles: bool,
) -> Result<(), DeblockError> {
    let mut ctx = PlaneCtx::new(samples, stride, width, height)?;
    let mut strengths = StrengthCache::default();
    for r in (0..mi_rows).step_by(plane_pass.row_step) {
        for c in (0..mi_cols).step_by(plane_pass.col_step) {
            if grid.is_candidate(
                r,
                c,
                plane_pass.pass,
                plane_pass.allow_df_sub_pu,
                plane_pass.plane_sub_x,
                plane_pass.plane_sub_y,
            ) {
                deblock_filter_edge(
                    &mut ctx,
                    grid,
                    plane_pass.edge_context(r, c, tile_starts),
                    disable_loopfilters_across_tiles,
                    &mut strengths,
                )?;
            }
        }
    }
    Ok(())
}

enum PlaneRows<'samples, 'rows, T> {
    Contiguous {
        samples: &'samples mut [T],
        stride: usize,
    },
    Split(&'rows mut [&'samples mut [T]]),
}

impl<T> PlaneRows<'_, '_, T> {
    fn row(&self, index: usize) -> Option<&[T]> {
        match self {
            Self::Contiguous { samples, stride } => {
                let start = index.checked_mul(*stride)?;
                let end = start.checked_add(*stride)?;
                samples.get(start..end)
            }
            Self::Split(rows) => rows.get(index).map(|row| &**row),
        }
    }

    fn row_mut(&mut self, index: usize) -> Option<&mut [T]> {
        match self {
            Self::Contiguous { samples, stride } => {
                let start = index.checked_mul(*stride)?;
                let end = start.checked_add(*stride)?;
                samples.get_mut(start..end)
            }
            Self::Split(rows) => rows.get_mut(index).map(|row| &mut **row),
        }
    }
}

struct PlaneCtx<'samples, 'rows, T: ReconSample> {
    rows: PlaneRows<'samples, 'rows, T>,
    width: usize,
    height: usize,
    x_origin: usize,
    y_origin: usize,
}

fn validated_plane_samples<T>(
    samples: &mut [T],
    stride: usize,
    width: usize,
    height: usize,
) -> Result<&mut [T], DeblockError> {
    let required = stride.checked_mul(height).ok_or(DeblockError::Workspace)?;
    if width > stride || stride == 0 {
        return Err(DeblockError::Workspace);
    }
    samples.get_mut(..required).ok_or(DeblockError::Workspace)
}

impl<'samples, 'rows, T: ReconSample> PlaneCtx<'samples, 'rows, T> {
    fn new(
        samples: &'samples mut [T],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self, DeblockError> {
        let samples = validated_plane_samples(samples, stride, width, height)?;
        Ok(Self {
            rows: PlaneRows::Contiguous { samples, stride },
            width,
            height,
            x_origin: 0,
            y_origin: 0,
        })
    }

    const fn contiguous_band(
        samples: &'samples mut [T],
        stride: usize,
        width: usize,
        height: usize,
        x_origin: usize,
        y_origin: usize,
    ) -> Self {
        Self {
            rows: PlaneRows::Contiguous { samples, stride },
            width,
            height,
            x_origin,
            y_origin,
        }
    }

    fn split_band(
        rows: &'rows mut [&'samples mut [T]],
        width: usize,
        height: usize,
        x_origin: usize,
        y_origin: usize,
    ) -> Self {
        Self {
            rows: PlaneRows::Split(rows),
            width,
            height,
            x_origin,
            y_origin,
        }
    }

    fn sample(&self, x: usize, y: usize) -> T {
        let row = y - self.y_origin;
        let col = x - self.x_origin;
        match &self.rows {
            PlaneRows::Contiguous { samples, stride } => samples[row * stride + col],
            PlaneRows::Split(rows) => rows[row][col],
        }
    }

    fn set_sample(&mut self, x: usize, y: usize, value: T) {
        let row = y - self.y_origin;
        let col = x - self.x_origin;
        match &mut self.rows {
            PlaneRows::Contiguous { samples, stride } => samples[row * *stride + col] = value,
            PlaneRows::Split(rows) => rows[row][col] = value,
        }
    }
}

const STRENGTH_CACHE_LEN: usize = max_quantizer_index(BitDepth::Ten) as usize + 1;

struct StrengthCache {
    entries: [Option<(i32, i32)>; STRENGTH_CACHE_LEN],
}

impl Default for StrengthCache {
    fn default() -> Self {
        Self {
            entries: [None; STRENGTH_CACHE_LEN],
        }
    }
}

impl StrengthCache {
    fn get(
        &mut self,
        qindex: u32,
        quant_delta: i32,
        df_delta_q: i32,
        bit_depth: BitDepth,
    ) -> (i32, i32) {
        let calculate = || {
            adaptive_strength(
                deblock_level(qindex, quant_delta, df_delta_q, bit_depth),
                bit_depth,
            )
        };
        let Some(entry) = usize::try_from(qindex)
            .ok()
            .and_then(|index| self.entries.get_mut(index))
        else {
            return calculate();
        };
        *entry.get_or_insert_with(calculate)
    }
}

#[derive(Clone, Copy)]
struct PlanePass {
    plane: usize,
    plane_id: PlaneId,
    pass: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    row_step: usize,
    col_step: usize,
    df_delta_q: i32,
    quant_delta: i32,
    bit_depth: BitDepth,
    allow_df_sub_pu: bool,
}

impl PlanePass {
    fn active(
        plane: usize,
        pass: usize,
        filter: DeblockingFilterParams,
        quant_deltas: DeblockQuantDeltas,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
    ) -> Option<Self> {
        let apply_index = if plane == 0 { pass } else { plane + 1 };
        if !filter.apply_deblocking_filter[apply_index] {
            return None;
        }

        let (plane_sub_x, plane_sub_y) = if plane == 0 {
            (0, 0)
        } else {
            (
                usize::from(pixel_format.subsampling_x()),
                usize::from(pixel_format.subsampling_y()),
            )
        };
        Some(Self {
            plane,
            plane_id: plane_index_to_id(plane),
            pass,
            plane_sub_x,
            plane_sub_y,
            row_step: 1 << plane_sub_y,
            col_step: 1 << plane_sub_x,
            df_delta_q: filter.df_delta_q[apply_index],
            quant_delta: quant_deltas.ac_delta(plane),
            bit_depth,
            allow_df_sub_pu: filter.allow_df_sub_pu,
        })
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn edge_context(self, row: usize, col: usize, tile_starts: Option<&[u32]>) -> EdgeContext {
        let coordinate = if self.pass == 0 { col } else { row };
        let tile_edge = tile_starts.is_some_and(|starts| {
            u32::try_from(coordinate).is_ok_and(|coordinate| starts.contains(&coordinate))
        });
        EdgeContext {
            plane: self.plane,
            pass: self.pass,
            row,
            col,
            plane_sub_x: self.plane_sub_x,
            plane_sub_y: self.plane_sub_y,
            df_delta_q: self.df_delta_q,
            quant_delta: self.quant_delta,
            bit_depth: self.bit_depth,
            allow_df_sub_pu: self.allow_df_sub_pu,
            tile_edge,
        }
    }
}

#[derive(Clone, Copy)]
struct EdgeContext {
    plane: usize,
    pass: usize,
    row: usize,
    col: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    df_delta_q: i32,
    quant_delta: i32,
    bit_depth: BitDepth,
    allow_df_sub_pu: bool,
    tile_edge: bool,
}

fn sub_pu_dimension(
    info: EdgeBlock<'_>,
    plane: usize,
    pass: usize,
    sub_x: usize,
    sub_y: usize,
) -> usize {
    if let Some(size) = info.block.sub_pu_size {
        let (dimension, subsampling) = if pass == 0 {
            (size.width, sub_x)
        } else {
            (size.height, sub_y)
        };
        return (dimension >> subsampling).max(1);
    }
    let tx = if plane == 0 {
        info.block.luma_prediction.default_sub_pu_tx
    } else {
        info.block.chroma_prediction.default_sub_pu_tx
    };
    let dimensions = if pass == 0 { &TX_WIDTH } else { &TX_HEIGHT };
    dimensions
        .get(tx)
        .and_then(|&size| usize::try_from(size).ok())
        .unwrap_or(1)
}

fn sub_pu_base(
    info: EdgeBlock<'_>,
    plane: usize,
    x: usize,
    y: usize,
    sub_x: usize,
    sub_y: usize,
) -> (usize, usize) {
    let prediction = info.prediction(plane);
    let block_x = (prediction.base_c * MI_SIZE) >> sub_x;
    let block_y = (prediction.base_r * MI_SIZE) >> sub_y;
    let Some(size) = info.block.sub_pu_size else {
        return (block_x, block_y);
    };
    let width = (size.width >> sub_x).max(1);
    let height = (size.height >> sub_y).max(1);
    (
        block_x + x.saturating_sub(block_x) / width * width,
        block_y + y.saturating_sub(block_y) / height * height,
    )
}

fn sub_pu_filter_dimension(tx_size: usize, sub_pu_size: usize, is_tx_edge: bool) -> (usize, bool) {
    if tx_size < sub_pu_size {
        (tx_size, false)
    } else if !is_tx_edge && tx_size == 8 {
        (4, true)
    } else if !is_tx_edge && tx_size == 16 && sub_pu_size == 16 {
        (8, true)
    } else {
        (sub_pu_size, true)
    }
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn deblock_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    grid: &MiGrid,
    ctx: EdgeContext,
    disable_loopfilters_across_tiles: bool,
    strengths: &mut StrengthCache,
) -> Result<(), DeblockError> {
    let EdgeContext {
        plane,
        pass,
        row,
        col,
        plane_sub_x,
        plane_sub_y,
        df_delta_q,
        quant_delta,
        bit_depth,
        allow_df_sub_pu,
        tile_edge,
    } = ctx;

    let (dx, dy) = if pass == 0 { (1usize, 0usize) } else { (0, 1) };

    let x = col * MI_SIZE;
    let y = row * MI_SIZE;

    if disable_loopfilters_across_tiles && tile_edge {
        return Ok(());
    }

    let sb_edge = pass == 1 && y.is_multiple_of(SB_SIZE) || pass == 0 && tile_edge;

    let on_screen = !((pass == 0 && x == 0) || (pass == 1 && y == 0));
    if !on_screen {
        return Ok(());
    }

    let x_p = x >> plane_sub_x;
    let y_p = y >> plane_sub_y;

    let prev_row = row - (dy << plane_sub_y);
    let prev_col = col - (dx << plane_sub_x);

    let curr = grid
        .get_edge(row, col)
        .ok_or(DeblockError::UncoveredMi { row, col })?;
    let prev = grid
        .get_edge(prev_row, prev_col)
        .ok_or(DeblockError::UncoveredMi {
            row: prev_row,
            col: prev_col,
        })?;

    let (tx_row_base, tx_col_base) = curr.tx_base(plane);
    let (prev_tx_row_base, prev_tx_col_base) = prev.tx_base(plane);
    let prediction = curr.prediction(plane);
    let block_y = (prediction.base_r * MI_SIZE) >> plane_sub_y;
    let block_x = (prediction.base_c * MI_SIZE) >> plane_sub_x;
    let skip = curr.block.skip;
    let tx_sz = curr.tx(plane);
    let prev_tx_sz = prev.tx(plane);
    let curr_tx_size = usize::try_from(if pass == 0 {
        TX_WIDTH[tx_sz]
    } else {
        TX_HEIGHT[tx_sz]
    })
    .unwrap_or(0);
    let prev_tx_size = usize::try_from(if pass == 0 {
        TX_WIDTH[prev_tx_sz]
    } else {
        TX_HEIGHT[prev_tx_sz]
    })
    .unwrap_or(0);
    let sub_pu_sizes = if allow_df_sub_pu {
        let curr_sub_pu_base = sub_pu_base(curr, plane, x_p, y_p, plane_sub_x, plane_sub_y);
        let prev_sub_pu_base = sub_pu_base(
            prev,
            plane,
            x_p.saturating_sub(dx),
            y_p.saturating_sub(dy),
            plane_sub_x,
            plane_sub_y,
        );
        (curr_sub_pu_base != prev_sub_pu_base).then(|| {
            (
                sub_pu_dimension(curr, plane, pass, plane_sub_x, plane_sub_y),
                sub_pu_dimension(prev, plane, pass, plane_sub_x, plane_sub_y),
            )
        })
    } else {
        None
    };
    let is_block_edge = (pass == 0 && x_p == block_x) || (pass == 1 && y_p == block_y);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;
    let (curr_filter_size, curr_sub_pu_edge) = if let Some((curr_sub_pu_size, _)) = sub_pu_sizes {
        sub_pu_filter_dimension(curr_tx_size, curr_sub_pu_size, is_tx_edge)
    } else {
        (curr_tx_size, false)
    };
    let prev_filter_size = if let Some((_, prev_sub_pu_size)) = sub_pu_sizes {
        sub_pu_filter_dimension(prev_tx_size, prev_sub_pu_size, is_tx_edge).0
    } else {
        prev_tx_size
    };
    let is_sub_pu_edge = curr_sub_pu_edge && !is_block_edge;

    let (curr_q, curr_side) = strengths.get(curr.block.qindex, quant_delta, df_delta_q, bit_depth);
    let (prev_q, prev_side) = strengths.get(prev.block.qindex, quant_delta, df_delta_q, bit_depth);

    let curr_strong = curr_q != 0 && curr_side != 0;
    let prev_strong = prev_q != 0 && prev_side != 0;
    let apply_filter = (is_tx_edge || is_sub_pu_edge)
        && (curr_strong || prev_strong)
        && (is_block_edge || !skip || is_sub_pu_edge);
    if !apply_filter {
        return Ok(());
    }

    let mut filter_size = curr_filter_size.min(prev_filter_size);

    let (plane_width, plane_height) = (plane_ctx.width, plane_ctx.height);
    if plane == 0 {
        if x_p + dx * 16 > plane_width || y_p + dy * 16 > plane_height {
            filter_size = filter_size.min(16);
        }
    } else if x_p + dx * 8 > plane_width || y_p + dy * 8 > plane_height {
        filter_size = filter_size.min(8);
    }

    let (mut q_thr, mut side) = combine_strengths(curr_q, prev_q, curr_side, prev_side);
    if is_sub_pu_edge && !is_tx_edge {
        q_thr >>= 3;
        side >>= 3;
    }

    let (max_width_neg, max_width_pos) = deblock_filter_max_width(filter_size, plane != 0, sb_edge);
    if max_width_neg == 0 || max_width_pos == 0 {
        return Ok(());
    }

    let horizontal = dx == 1
        && dy == 0
        && x_p >= plane_ctx.x_origin.saturating_add(GATHER_HALF)
        && x_p <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y_p >= plane_ctx.y_origin
        && y_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y_p >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y_p <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x_p >= plane_ctx.x_origin
        && x_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.width);
    if horizontal || vertical {
        let x_origin = plane_ctx.x_origin;
        let y_origin = plane_ctx.y_origin;
        if let PlaneRows::Contiguous { samples, stride } = &mut plane_ctx.rows {
            let stride = *stride;
            let boundary = (y_p - y_origin) * stride + x_p - x_origin;
            let (perpendicular, lane) = if horizontal { (1, stride) } else { (stride, 1) };
            return filter_contiguous_edge(
                samples,
                boundary,
                NonZeroUsize::new(perpendicular).ok_or(DeblockError::Workspace)?,
                NonZeroUsize::new(lane).ok_or(DeblockError::Workspace)?,
                q_thr,
                side,
                max_width_neg,
                max_width_pos,
                prev.block.lossless,
                curr.block.lossless,
                bit_depth,
            );
        }
    }

    let width = choose_filter_width(
        plane_ctx,
        x_p,
        y_p,
        dx,
        dy,
        q_thr,
        side,
        max_width_neg,
        max_width_pos,
    )?;
    if width == 0 {
        return Ok(());
    }

    let eff_neg = width.min(max_width_neg);
    let eff_pos = width.min(max_width_pos);
    let q_thresh_mult = Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1];
    let w_mult_neg = W_MULT[eff_neg - 1];
    let w_mult_pos = W_MULT[eff_pos - 1];
    let sample_params = DeblockSampleFilter {
        boundary: GATHER_HALF,
        q_thr,
        max_width_neg: eff_neg,
        max_width_pos: eff_pos,
        q_thresh_mult,
        w_mult_neg,
        w_mult_pos,
        prev_lossless: prev.block.lossless,
        curr_lossless: curr.block.lossless,
        bit_depth,
    };

    apply_edge_samples(
        plane_ctx,
        PerpLine::new(x_p, y_p, dx, dy),
        MI_SIZE,
        sample_params,
    )
}

#[allow(clippy::too_many_arguments)]
fn filter_contiguous_edge<T: ReconSample>(
    samples: &mut [T],
    boundary: usize,
    perpendicular_stride: NonZeroUsize,
    lane_stride: NonZeroUsize,
    q_thr: i32,
    side: i32,
    max_width_neg: usize,
    max_width_pos: usize,
    prev_lossless: bool,
    curr_lossless: bool,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let width = deblock_filter_choice_strided(
        samples,
        boundary + (MI_SIZE - 1) * lane_stride.get(),
        perpendicular_stride,
        &DeblockFilterChoice {
            boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        },
    )
    .map_err(|_| DeblockError::FilterChoice)?;
    if width == 0 {
        return Ok(());
    }
    let eff_neg = width.min(max_width_neg);
    let eff_pos = width.min(max_width_pos);
    let params = DeblockSampleFilter {
        boundary,
        q_thr,
        max_width_neg: eff_neg,
        max_width_pos: eff_pos,
        q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
        w_mult_neg: W_MULT[eff_neg - 1],
        w_mult_pos: W_MULT[eff_pos - 1],
        prev_lossless,
        curr_lossless,
        bit_depth,
    };
    deblock_sample_filter_strided_4(samples, perpendicular_stride, lane_stride, &params)
        .map_err(|_| DeblockError::SampleFilter)
}

#[allow(clippy::too_many_arguments)]
fn choose_filter_width<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, '_, T>,
    x_p: usize,
    y_p: usize,
    dx: usize,
    dy: usize,
    q_thr: i32,
    side: i32,
    max_width_neg: usize,
    max_width_pos: usize,
) -> Result<usize, DeblockError> {
    if q_thr == 0 || side == 0 {
        return Ok(0);
    }
    let boundary = GATHER_HALF;
    let horizontal = dx == 1
        && dy == 0
        && x_p >= plane_ctx.x_origin.saturating_add(GATHER_HALF)
        && x_p <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y_p >= plane_ctx.y_origin
        && y_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y_p >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y_p <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x_p >= plane_ctx.x_origin
        && x_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.width);
    if (horizontal || vertical)
        && let PlaneRows::Contiguous { samples, stride } = &plane_ctx.rows
    {
        let first_boundary = (y_p - plane_ctx.y_origin) * *stride + x_p - plane_ctx.x_origin;
        let perpendicular_stride = if horizontal { 1 } else { *stride };
        let lane_stride = if horizontal { *stride } else { 1 };
        let params = DeblockFilterChoice {
            boundary: first_boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        };
        return deblock_filter_choice_strided(
            samples,
            first_boundary + (MI_SIZE - 1) * lane_stride,
            NonZeroUsize::new(perpendicular_stride).ok_or(DeblockError::Workspace)?,
            &params,
        )
        .map_err(|_| DeblockError::FilterChoice);
    }

    let s = gather_line(plane_ctx, PerpLine::new(x_p, y_p, dx, dy));
    let end = MI_SIZE - 1;
    let t_x = x_p + dy * end;
    let t_y = y_p + dx * end;
    let t = gather_line(plane_ctx, PerpLine::new(t_x, t_y, dx, dy));

    let width = deblock_filter_choice(
        &s,
        &t,
        &DeblockFilterChoice {
            boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        },
    )
    .map_err(|_| DeblockError::FilterChoice)?;
    Ok(width)
}

fn coded_plane_dimensions<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
) -> Result<(usize, usize), DeblockError> {
    let plane = workspace
        .plane(plane_id)
        .map_err(|_| DeblockError::Workspace)?;
    let storage_size = plane.storage_size();
    Ok((storage_size.width(), storage_size.height()))
}

#[derive(Clone, Copy)]
struct PerpLine {
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
}

impl PerpLine {
    const fn new(x: usize, y: usize, dx: usize, dy: usize) -> Self {
        Self { x, y, dx, dy }
    }

    fn offset(self, offset: isize) -> Result<(usize, usize), DeblockError> {
        let fx = (self.x as isize + offset * self.dx as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        let fy = (self.y as isize + offset * self.dy as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        Ok((fx, fy))
    }
}

const GATHER_HALF: usize = 8;

fn apply_edge_samples<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    perp: PerpLine,
    lanes: usize,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let PerpLine { x, y, dx, dy } = perp;
    if lanes == 0 {
        return Ok(());
    }
    let horizontal = dx == 1
        && dy == 0
        && x >= plane_ctx.x_origin.saturating_add(GATHER_HALF)
        && x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y >= plane_ctx.y_origin
        && y.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x >= plane_ctx.x_origin
        && x.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.width);
    if lanes <= MI_SIZE
        && params.boundary == GATHER_HALF
        && (horizontal || vertical)
        && let PlaneRows::Contiguous { samples, stride } = &mut plane_ctx.rows
    {
        let boundary = (y - plane_ctx.y_origin) * *stride + x - plane_ctx.x_origin;
        let perpendicular_stride = if horizontal { 1 } else { *stride };
        let lane_stride = if horizontal { *stride } else { 1 };
        let perpendicular_stride =
            NonZeroUsize::new(perpendicular_stride).ok_or(DeblockError::Workspace)?;
        if lanes == MI_SIZE {
            deblock_sample_filter_strided_4(
                samples,
                perpendicular_stride,
                NonZeroUsize::new(lane_stride).ok_or(DeblockError::Workspace)?,
                &DeblockSampleFilter { boundary, ..params },
            )
            .map_err(|_| DeblockError::SampleFilter)?;
        } else {
            for lane in 0..lanes {
                deblock_sample_filter_strided(
                    samples,
                    perpendicular_stride,
                    &DeblockSampleFilter {
                        boundary: boundary + lane * lane_stride,
                        ..params
                    },
                )
                .map_err(|_| DeblockError::SampleFilter)?;
            }
        }
        return Ok(());
    }
    if lanes <= MI_SIZE
        && params.boundary == GATHER_HALF
        && dx == 1
        && dy == 0
        && x >= plane_ctx.x_origin.saturating_add(GATHER_HALF)
        && x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y >= plane_ctx.y_origin
        && y.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.height)
    {
        let row_start = y - plane_ctx.y_origin;
        let column_start = x - GATHER_HALF - plane_ctx.x_origin;
        let row_range = row_start..row_start + lanes;
        if row_range.clone().all(|row| {
            plane_ctx
                .rows
                .row(row)
                .is_some_and(|row| column_start + 2 * GATHER_HALF <= row.len())
        }) {
            for row in row_range {
                let row = plane_ctx.rows.row_mut(row).ok_or(DeblockError::Workspace)?;
                deblock_sample_filter(
                    &mut row[column_start..column_start + 2 * GATHER_HALF],
                    &params,
                )
                .map_err(|_| DeblockError::SampleFilter)?;
            }
            return Ok(());
        }
    }

    if lanes <= MI_SIZE
        && params.boundary == GATHER_HALF
        && dx == 0
        && dy == 1
        && y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x >= plane_ctx.x_origin
        && x.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.width)
    {
        let row_start = y - GATHER_HALF - plane_ctx.y_origin;
        let column_start = x - plane_ctx.x_origin;
        let row_range = row_start..row_start + 2 * GATHER_HALF;
        if row_range.clone().all(|row| {
            plane_ctx
                .rows
                .row(row)
                .is_some_and(|row| column_start + lanes <= row.len())
        }) {
            let mut lines = [[T::default(); 2 * GATHER_HALF]; MI_SIZE];
            for (sample_index, row) in row_range.clone().enumerate() {
                let row = plane_ctx.rows.row(row).ok_or(DeblockError::Workspace)?;
                for lane in 0..lanes {
                    lines[lane][sample_index] = row[column_start + lane];
                }
            }
            for line in &mut lines[..lanes] {
                deblock_sample_filter(line, &params).map_err(|_| DeblockError::SampleFilter)?;
            }

            if !params.prev_lossless {
                let start = params.boundary - params.max_width_neg;
                for offset in 0..params.max_width_neg {
                    let sample_index = start + offset;
                    let row = plane_ctx
                        .rows
                        .row_mut(row_start + sample_index)
                        .ok_or(DeblockError::Workspace)?;
                    for (target, line) in row[column_start..column_start + lanes]
                        .iter_mut()
                        .zip(&lines[..lanes])
                    {
                        *target = line[sample_index];
                    }
                }
            }
            if !params.curr_lossless {
                let width = params.max_width_neg.max(params.max_width_pos);
                for offset in 0..width {
                    let sample_index = params.boundary + offset;
                    let row = plane_ctx
                        .rows
                        .row_mut(row_start + sample_index)
                        .ok_or(DeblockError::Workspace)?;
                    for (target, line) in row[column_start..column_start + lanes]
                        .iter_mut()
                        .zip(&lines[..lanes])
                    {
                        *target = line[sample_index];
                    }
                }
            }
            return Ok(());
        }
    }

    for lane in 0..lanes {
        apply_sample_filter(
            plane_ctx,
            PerpLine::new(x + dy * lane, y + dx * lane, dx, dy),
            params,
        )?;
    }
    Ok(())
}

fn apply_sample_filter<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    perp: PerpLine,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let before = gather_line(plane_ctx, perp);
    let mut line = before;

    deblock_sample_filter(&mut line, &params).map_err(|_| DeblockError::SampleFilter)?;

    for (idx, (&new, &old)) in line.iter().zip(before.iter()).enumerate() {
        let changed = new.to_u16() != old.to_u16();
        if !changed {
            continue;
        }
        let (fx, fy) = perp.offset(idx as isize - params.boundary as isize)?;
        if fx >= plane_ctx.width || fy >= plane_ctx.height {
            return Err(DeblockError::Workspace);
        }
        plane_ctx.set_sample(fx, fy, new);
    }
    Ok(())
}

fn gather_line<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, '_, T>,
    perp: PerpLine,
) -> [T; 2 * GATHER_HALF] {
    let mut line = [T::default(); 2 * GATHER_HALF];
    if perp.x >= plane_ctx.x_origin.saturating_add(GATHER_HALF)
        && perp.y >= plane_ctx.y_origin
        && perp.x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && perp.dx == 1
        && perp.dy == 0
    {
        let row = perp.y - plane_ctx.y_origin;
        let start = perp.x - GATHER_HALF - plane_ctx.x_origin;
        if let Some(src) = plane_ctx
            .rows
            .row(row)
            .and_then(|row| row.get(start..start + 2 * GATHER_HALF))
        {
            line.copy_from_slice(src);
            return line;
        }
    }
    if perp.y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && perp.x >= plane_ctx.x_origin
        && perp.x < plane_ctx.width
        && perp.y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && perp.dx == 0
        && perp.dy == 1
    {
        let start = perp.y - GATHER_HALF - plane_ctx.y_origin;
        let x = perp.x - plane_ctx.x_origin;
        let row_range = start..start + 2 * GATHER_HALF;
        if row_range
            .clone()
            .all(|row| plane_ctx.rows.row(row).is_some_and(|row| x < row.len()))
        {
            for (lane, row) in line.iter_mut().zip(row_range) {
                if let Some(row) = plane_ctx.rows.row(row) {
                    *lane = row[x];
                }
            }
            return line;
        }
    }
    let max_x = plane_ctx.width.saturating_sub(1) as isize;
    let max_y = plane_ctx.height.saturating_sub(1) as isize;
    for (idx, lane) in line.iter_mut().enumerate() {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
        *lane = plane_ctx.sample(sx, sy);
    }
    line
}

const DF_DELTA_SCALE: i32 = 8;

fn deblock_level(qindex: u32, quant_delta: i32, df_delta_q: i32, bit_depth: BitDepth) -> u32 {
    let q_clamped = q_clamped(qindex, quant_delta, bit_depth);
    q_clamped.saturating_add_signed(df_delta_q.saturating_mul(DF_DELTA_SCALE))
}

fn q_clamped(qindex: u32, delta: i32, bit_depth: BitDepth) -> u32 {
    if qindex == 0 && delta <= 0 {
        return 0;
    }
    let adjusted = qindex.saturating_add_signed(delta);
    adjusted.clamp(1, max_quantizer_index(bit_depth))
}

fn adaptive_strength(lvl: u32, bit_depth: BitDepth) -> (i32, i32) {
    let q_ind = deblock_side_threshold_index(lvl, bit_depth);
    let side_threshold = SIDE_THRESHOLDS[q_ind];
    deblock_adaptive_filter_strength(lvl, side_threshold, bit_depth)
}

fn combine_strengths(curr_q: i32, prev_q: i32, curr_side: i32, prev_side: i32) -> (i32, i32) {
    let q_thr = if curr_q != 0 && prev_q != 0 {
        (curr_q + prev_q + 1) >> 1
    } else {
        curr_q.max(prev_q)
    };
    let side = if curr_side != 0 && prev_side != 0 {
        (curr_side + prev_side + 1) >> 1
    } else {
        curr_side.max(prev_side)
    };
    (q_thr, side)
}

fn plane_index_to_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

const NO_BLOCK_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct MiCell {
    base: u32,
    overlay: u32,
    chroma_transform: u32,
}

impl Default for MiCell {
    fn default() -> Self {
        Self {
            base: NO_BLOCK_INDEX,
            overlay: NO_BLOCK_INDEX,
            chroma_transform: NO_BLOCK_INDEX,
        }
    }
}

#[derive(Clone)]
struct MiGrid<'a> {
    mi_cols: usize,
    fully_covered: bool,
    base_blocks: &'a [DeblockBlock],
    overlay_blocks: &'a [DeblockBlock],
    cells: Vec<MiCell>,
    candidates: Vec<u8>,
}

impl MiGrid<'_> {
    fn get_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.cells.get(row * self.mi_cols + col)?;
        let block = if cell.overlay != NO_BLOCK_INDEX {
            self.overlay_blocks.get(cell.overlay as usize)?
        } else {
            self.base_blocks.get(cell.base as usize)?
        };
        let chroma_transform = if cell.chroma_transform != NO_BLOCK_INDEX {
            Some(self.overlay_blocks.get(cell.chroma_transform as usize)?)
        } else {
            None
        };
        Some(EdgeBlock {
            block,
            chroma_transform,
        })
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn is_candidate(
        &self,
        row: usize,
        col: usize,
        pass: usize,
        allow_sub_pu: bool,
        plane_sub_x: usize,
        plane_sub_y: usize,
    ) -> bool {
        let candidate = if pass == 0 {
            VERTICAL_TX_CANDIDATE
        } else {
            HORIZONTAL_TX_CANDIDATE
        };
        let index = row * self.mi_cols + col;
        let Some(&current) = self.candidates.get(index) else {
            return true;
        };
        if !self.fully_covered {
            let Some(cell) = self.cells.get(index) else {
                return true;
            };
            if cell.base == NO_BLOCK_INDEX && cell.overlay == NO_BLOCK_INDEX {
                return true;
            }
        }
        if current & candidate != 0 || allow_sub_pu && current & SUB_PU_CANDIDATE != 0 {
            return true;
        }
        if pass == 0 && plane_sub_x != 0 && col != 0 {
            return self.candidates[index - 1] & VERTICAL_TX_CANDIDATE != 0;
        }
        if pass == 1 && plane_sub_y != 0 && row != 0 {
            return self.candidates[index - self.mi_cols] & HORIZONTAL_TX_CANDIDATE != 0;
        }
        false
    }
}

/// Bounds retained deblock MI-grid scratch: at most this many `(cells,
/// candidates)` buffer pairs, and only those whose cell capacity fits a large
/// frame, so a big frame cannot pin its high-water allocation in the pool. The
/// base grid plus up to two chroma overlays are live per deblock call.
const MAX_RETAINED_DEBLOCK_GRIDS: usize = 4;
const MAX_RETAINED_DEBLOCK_CELLS: usize = 1 << 22;
static RETAINED_DEBLOCK_GRIDS: std::sync::Mutex<Vec<(Vec<MiCell>, Vec<u8>)>> =
    std::sync::Mutex::new(Vec::new());

/// Takes a `(cells, candidates)` scratch pair from the retained pool, reusing a
/// prior deblock call's allocation when available.
fn take_deblock_grid_scratch() -> (Vec<MiCell>, Vec<u8>) {
    RETAINED_DEBLOCK_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default()
}

/// Returns a `(cells, candidates)` scratch pair to the retained pool, dropping
/// it when the pool is full or the buffers are oversized.
fn recycle_deblock_grid_scratch(mut cells: Vec<MiCell>, mut candidates: Vec<u8>) {
    if cells.capacity() == 0 || cells.capacity() > MAX_RETAINED_DEBLOCK_CELLS {
        return;
    }
    let mut pool = RETAINED_DEBLOCK_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pool.len() < MAX_RETAINED_DEBLOCK_GRIDS {
        cells.clear();
        candidates.clear();
        pool.push((cells, candidates));
    }
}

impl MiGrid<'_> {
    /// Consumes the grid, returning its `(cells, candidates)` buffers for reuse.
    fn into_scratch(self) -> (Vec<MiCell>, Vec<u8>) {
        (self.cells, self.candidates)
    }
}

fn build_mi_grid(
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
) -> Result<MiGrid<'_>, DeblockError> {
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(DeblockError::Workspace)?;
    let (mut cells, mut candidates) = take_deblock_grid_scratch();
    cells.clear();
    cells
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Workspace)?;
    cells.resize(count, MiCell::default());
    candidates.clear();
    candidates
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Workspace)?;
    candidates.resize(count, 0);

    for (block_index, block) in blocks.iter().enumerate() {
        let block_index = mi_block_index(block_index)?;
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    cells[rr * mi_cols + cc].base = block_index;
                }
            }
        }
        mark_block_candidates(&mut candidates, block, mi_rows, mi_cols);
    }
    let fully_covered = cells.iter().all(|cell| cell.base != NO_BLOCK_INDEX);
    Ok(MiGrid {
        mi_cols,
        fully_covered,
        base_blocks: blocks,
        overlay_blocks: &[],
        cells,
        candidates,
    })
}

fn overlay_mi_grid<'a>(
    base: &MiGrid<'a>,
    blocks: &'a [DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
) -> Result<MiGrid<'a>, DeblockError> {
    let (mut cells, mut candidates) = take_deblock_grid_scratch();
    cells.clone_from(&base.cells);
    candidates.clone_from(&base.candidates);
    let mut grid = MiGrid {
        mi_cols: base.mi_cols,
        fully_covered: base.fully_covered,
        base_blocks: base.base_blocks,
        overlay_blocks: blocks,
        cells,
        candidates,
    };
    for (block_index, block) in blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| !block.chroma_transform_only)
    {
        let block_index = mi_block_index(block_index)?;
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc].overlay = block_index;
                }
            }
        }
        mark_block_candidates(&mut grid.candidates, block, mi_rows, mi_cols);
    }
    for (block_index, block) in blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.chroma_transform_only)
    {
        let block_index = mi_block_index(block_index)?;
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc].chroma_transform = block_index;
                }
            }
        }
        mark_block_candidates(&mut grid.candidates, block, mi_rows, mi_cols);
    }
    if !grid.fully_covered {
        grid.fully_covered = grid
            .cells
            .iter()
            .all(|cell| cell.base != NO_BLOCK_INDEX || cell.overlay != NO_BLOCK_INDEX);
    }
    Ok(grid)
}

fn mi_block_index(index: usize) -> Result<u32, DeblockError> {
    let index = u32::try_from(index).map_err(|_| DeblockError::Workspace)?;
    if index == NO_BLOCK_INDEX {
        return Err(DeblockError::Workspace);
    }
    Ok(index)
}

fn mark_block_candidates(
    candidates: &mut [u8],
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
) {
    let row_end = block.r.saturating_add(block.n4h).min(mi_rows);
    let col_end = block.c.saturating_add(block.n4w).min(mi_cols);
    let row_start = block.r.min(row_end);
    let col_start = block.c.min(col_end);

    for row in row_start..row_end {
        mark_vertical_candidate(candidates, row, col_start, mi_cols);
        mark_vertical_candidate(candidates, row, col_end, mi_cols);
    }
    for col in col_start..col_end {
        mark_horizontal_candidate(candidates, row_start, col, mi_rows, mi_cols);
        mark_horizontal_candidate(candidates, row_end, col, mi_rows, mi_cols);
    }
    if block.sub_pu_size.is_some() {
        for row in row_start..row_end {
            let start = row * mi_cols + col_start;
            let end = row * mi_cols + col_end;
            for candidate in &mut candidates[start..end] {
                *candidate |= SUB_PU_CANDIDATE;
            }
        }
    }
}

fn mark_vertical_candidate(candidates: &mut [u8], row: usize, col: usize, mi_cols: usize) {
    if col < mi_cols {
        candidates[row * mi_cols + col] |= VERTICAL_TX_CANDIDATE;
    }
}

fn mark_horizontal_candidate(
    candidates: &mut [u8],
    row: usize,
    col: usize,
    mi_rows: usize,
    mi_cols: usize,
) {
    if row < mi_rows {
        candidates[row * mi_cols + col] |= HORIZONTAL_TX_CANDIDATE;
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DeblockError {
    #[error("deblocking MI ({row}, {col}) is not covered by any decoded block")]
    UncoveredMi { row: usize, col: usize },
    #[error("deblocking filter-choice primitive rejected its inputs")]
    FilterChoice,
    #[error("deblocking sample-filter primitive rejected its inputs")]
    SampleFilter,
    #[error("deblocking workspace sample access went out of bounds")]
    Workspace,
}

#[cfg(test)]
#[path = "deblock_tests.rs"]
mod tests;
