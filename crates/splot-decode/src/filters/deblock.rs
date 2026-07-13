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
    deblock_filter_max_width, deblock_sample_filter, deblock_side_threshold_index,
    max_quantizer_index,
};

const MI_SIZE: usize = 4;

const SB_SIZE: usize = 64;

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
struct MiBlockInfo {
    base_row: usize,
    base_col: usize,
    luma_prediction: DeblockPredictionUnit,
    chroma_prediction: DeblockPredictionUnit,
    chroma_base_row: usize,
    chroma_base_col: usize,
    luma_tx: usize,
    chroma_tx: Option<usize>,
    sub_pu_size: Option<DeblockSubPuSize>,
    qindex: u32,
    skip: bool,
    lossless: bool,
}

impl MiBlockInfo {
    const fn from_block(block: DeblockBlock) -> Self {
        let DeblockBlock {
            r,
            c,
            luma_prediction,
            chroma_prediction,
            chroma_base_r,
            chroma_base_c,
            n4w: _,
            n4h: _,
            luma_tx,
            chroma_tx,
            sub_pu_size,
            chroma_transform_only: _,
            qindex,
            skip,
            lossless,
        } = block;
        Self {
            base_row: r,
            base_col: c,
            luma_prediction,
            chroma_prediction,
            chroma_base_row: chroma_base_r,
            chroma_base_col: chroma_base_c,
            luma_tx,
            chroma_tx,
            sub_pu_size,
            qindex,
            skip,
            lossless,
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
        let chroma_grid = (plane != 0).then(|| {
            let mut chroma_grid = grid.clone();
            overlay_mi_grid(&mut chroma_grid, chroma_blocks[plane - 1], mi_rows, mi_cols);
            chroma_grid
        });
        let plane_grid = match chroma_grid.as_ref() {
            Some(chroma_grid) => chroma_grid,
            None => &grid,
        };
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
    }

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
    plane_ctx: &mut PlaneCtx<'_, T>,
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
    if splot_parallel::on_worker_pool()
        && (plane_pass.pass == 0 && covered_rows <= height
            || plane_pass.pass == 1 && covered_cols <= width)
    {
        let workers = splot_parallel::current_pool_width();
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let full = PlaneCtx::new(samples, stride, width, height)?;
        let bands: Vec<(usize, usize, PlaneCtx<'_, T>)> = if plane_pass.pass == 0 {
            let plane_units = height.div_ceil(MI_SIZE);
            let units_per_band = plane_units.div_ceil(workers * 4).max(1);
            let mut bands = Vec::new();
            let mut rows = full.rows.into_iter();
            let mut y_origin = 0;
            loop {
                let band: Vec<&mut [T]> = rows.by_ref().take(units_per_band * MI_SIZE).collect();
                if band.is_empty() {
                    break;
                }
                let unit_start = y_origin / MI_SIZE;
                bands.push((
                    unit_start,
                    unit_start + units_per_band,
                    PlaneCtx::band(band, width, height, 0, y_origin),
                ));
                y_origin += units_per_band * MI_SIZE;
            }
            bands
        } else {
            let plane_units = width.div_ceil(MI_SIZE);
            let units_per_band = plane_units.div_ceil(workers * 2).max(1);
            let band_count = plane_units.div_ceil(units_per_band);
            let mut columns: Vec<Vec<&mut [T]>> = (0..band_count)
                .map(|_| Vec::with_capacity(height))
                .collect();
            for row in full.rows {
                for (band, chunk) in column_chunks(row, units_per_band * MI_SIZE).enumerate() {
                    columns[band].push(chunk);
                }
            }
            columns
                .into_iter()
                .enumerate()
                .map(|(band, rows)| {
                    let unit_start = band * units_per_band;
                    let x_origin = unit_start * MI_SIZE;
                    (
                        unit_start,
                        unit_start + units_per_band,
                        PlaneCtx::band(rows, width, height, x_origin, 0),
                    )
                })
                .collect()
        };
        let band_count = bands.len();
        let result = bands
            .into_par_iter()
            .try_for_each(|(unit_start, unit_end, mut ctx)| {
                tally.note_worker();
                let mut strengths = StrengthCache::default();
                if plane_pass.pass == 0 {
                    for unit in unit_start..unit_end {
                        let r = unit * plane_pass.row_step;
                        if r >= mi_rows {
                            break;
                        }
                        for c in (0..mi_cols).step_by(plane_pass.col_step) {
                            deblock_filter_edge(
                                &mut ctx,
                                grid,
                                plane_pass.edge_context(r, c, tile_info),
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
                            deblock_filter_edge(
                                &mut ctx,
                                grid,
                                plane_pass.edge_context(r, c, tile_info),
                                disable_loopfilters_across_tiles,
                                &mut strengths,
                            )?;
                        }
                    }
                }
                Ok(())
            });
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
        return result;
    }

    let mut ctx = PlaneCtx::new(samples, stride, width, height)?;
    let mut strengths = StrengthCache::default();
    for r in (0..mi_rows).step_by(plane_pass.row_step) {
        for c in (0..mi_cols).step_by(plane_pass.col_step) {
            deblock_filter_edge(
                &mut ctx,
                grid,
                plane_pass.edge_context(r, c, tile_info),
                disable_loopfilters_across_tiles,
                &mut strengths,
            )?;
        }
    }
    Ok(())
}

struct PlaneCtx<'a, T: ReconSample> {
    rows: Vec<&'a mut [T]>,
    width: usize,
    height: usize,
    x_origin: usize,
    y_origin: usize,
}

impl<'a, T: ReconSample> PlaneCtx<'a, T> {
    fn new(
        samples: &'a mut [T],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self, DeblockError> {
        let required = stride.checked_mul(height).ok_or(DeblockError::Workspace)?;
        if width > stride || stride == 0 || required > samples.len() {
            return Err(DeblockError::Workspace);
        }
        Ok(Self {
            rows: samples.chunks_mut(stride).take(height).collect(),
            width,
            height,
            x_origin: 0,
            y_origin: 0,
        })
    }

    fn band(
        rows: Vec<&'a mut [T]>,
        width: usize,
        height: usize,
        x_origin: usize,
        y_origin: usize,
    ) -> Self {
        Self {
            rows,
            width,
            height,
            x_origin,
            y_origin,
        }
    }

    fn sample(&self, x: usize, y: usize) -> T {
        self.rows[y - self.y_origin][x - self.x_origin]
    }

    fn set_sample(&mut self, x: usize, y: usize, value: T) {
        self.rows[y - self.y_origin][x - self.x_origin] = value;
    }
}

#[derive(Default)]
struct StrengthCache {
    entries: Vec<(u32, (i32, i32))>,
}

impl StrengthCache {
    fn get(
        &mut self,
        qindex: u32,
        quant_delta: i32,
        df_delta_q: i32,
        bit_depth: BitDepth,
    ) -> (i32, i32) {
        if let Some(&(_, value)) = self.entries.iter().find(|(key, _)| *key == qindex) {
            return value;
        }
        let value = adaptive_strength(
            deblock_level(qindex, quant_delta, df_delta_q, bit_depth),
            bit_depth,
        );
        self.entries.push((qindex, value));
        value
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

    fn edge_context(self, row: usize, col: usize, tile_info: Option<&TileInfo>) -> EdgeContext {
        let tile_edge = tile_info.is_some_and(|tile_info| {
            let (starts, coordinate) = if self.pass == 0 {
                (&tile_info.mi_col_starts, col)
            } else {
                (&tile_info.mi_row_starts, row)
            };
            u32::try_from(coordinate).is_ok_and(|coordinate| {
                starts
                    .get(1..starts.len().saturating_sub(1))
                    .is_some_and(|starts| starts.contains(&coordinate))
            })
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

fn column_chunks<T>(row: &mut [T], size: usize) -> core::slice::ChunksMut<'_, T> {
    row.chunks_mut(size)
}

fn sub_pu_dimension(
    info: &MiBlockInfo,
    plane: usize,
    pass: usize,
    sub_x: usize,
    sub_y: usize,
) -> usize {
    if let Some(size) = info.sub_pu_size {
        let (dimension, subsampling) = if pass == 0 {
            (size.width, sub_x)
        } else {
            (size.height, sub_y)
        };
        return (dimension >> subsampling).max(1);
    }
    let tx = if plane == 0 {
        info.luma_prediction.default_sub_pu_tx
    } else {
        info.chroma_prediction.default_sub_pu_tx
    };
    let dimensions = if pass == 0 { &TX_WIDTH } else { &TX_HEIGHT };
    dimensions
        .get(tx)
        .and_then(|&size| usize::try_from(size).ok())
        .unwrap_or(1)
}

fn sub_pu_base(
    info: &MiBlockInfo,
    plane: usize,
    x: usize,
    y: usize,
    sub_x: usize,
    sub_y: usize,
) -> (usize, usize) {
    let prediction = if plane == 0 {
        info.luma_prediction
    } else {
        info.chroma_prediction
    };
    let block_x = (prediction.base_c * MI_SIZE) >> sub_x;
    let block_y = (prediction.base_r * MI_SIZE) >> sub_y;
    let Some(size) = info.sub_pu_size else {
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

fn deblock_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, T>,
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
        .get(row, col)
        .ok_or(DeblockError::UncoveredMi { row, col })?;
    let prev = grid
        .get(prev_row, prev_col)
        .ok_or(DeblockError::UncoveredMi {
            row: prev_row,
            col: prev_col,
        })?;

    let tx_sz = plane_tx(plane, curr);
    let prev_tx_sz = plane_tx(plane, prev);

    let (tx_col_base, tx_row_base, prev_tx_col_base, prev_tx_row_base) = if plane == 0 {
        (curr.base_col, curr.base_row, prev.base_col, prev.base_row)
    } else {
        (
            curr.chroma_base_col,
            curr.chroma_base_row,
            prev.chroma_base_col,
            prev.chroma_base_row,
        )
    };

    let prediction = if plane == 0 {
        curr.luma_prediction
    } else {
        curr.chroma_prediction
    };
    let block_y = (prediction.base_r * MI_SIZE) >> plane_sub_y;
    let block_x = (prediction.base_c * MI_SIZE) >> plane_sub_x;
    let skip = curr.skip;
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
    let curr_sub_pu_size = sub_pu_dimension(&curr, plane, pass, plane_sub_x, plane_sub_y);
    let prev_sub_pu_size = sub_pu_dimension(&prev, plane, pass, plane_sub_x, plane_sub_y);
    let curr_sub_pu_base = sub_pu_base(&curr, plane, x_p, y_p, plane_sub_x, plane_sub_y);
    let prev_sub_pu_base = sub_pu_base(
        &prev,
        plane,
        x_p.saturating_sub(dx),
        y_p.saturating_sub(dy),
        plane_sub_x,
        plane_sub_y,
    );
    let is_sub_pu_boundary = allow_df_sub_pu && curr_sub_pu_base != prev_sub_pu_base;

    let is_block_edge = (pass == 0 && x_p == block_x) || (pass == 1 && y_p == block_y);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;
    let (curr_filter_size, curr_sub_pu_edge) = if is_sub_pu_boundary {
        sub_pu_filter_dimension(curr_tx_size, curr_sub_pu_size, is_tx_edge)
    } else {
        (curr_tx_size, false)
    };
    let prev_filter_size = if is_sub_pu_boundary {
        sub_pu_filter_dimension(prev_tx_size, prev_sub_pu_size, is_tx_edge).0
    } else {
        prev_tx_size
    };
    let is_sub_pu_edge = curr_sub_pu_edge && !is_block_edge;

    let (curr_q, curr_side) = strengths.get(curr.qindex, quant_delta, df_delta_q, bit_depth);
    let (prev_q, prev_side) = strengths.get(prev.qindex, quant_delta, df_delta_q, bit_depth);

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
        prev_lossless: prev.lossless,
        curr_lossless: curr.lossless,
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
fn choose_filter_width<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, T>,
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
    plane_ctx: &mut PlaneCtx<'_, T>,
    perp: PerpLine,
    lanes: usize,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let PerpLine { x, y, dx, dy } = perp;
    if lanes == 0 {
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
        if let Some(rows) = plane_ctx.rows.get_mut(row_start..row_start + lanes)
            && rows
                .iter()
                .all(|row| column_start + 2 * GATHER_HALF <= row.len())
        {
            for row in rows {
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
        if let Some(rows) = plane_ctx
            .rows
            .get_mut(row_start..row_start + 2 * GATHER_HALF)
            && rows.iter().all(|row| column_start + lanes <= row.len())
        {
            let mut lines = [[T::default(); 2 * GATHER_HALF]; MI_SIZE];
            for (sample_index, row) in rows.iter().enumerate() {
                for lane in 0..lanes {
                    lines[lane][sample_index] = row[column_start + lane];
                }
            }
            for line in &mut lines[..lanes] {
                deblock_sample_filter(line, &params).map_err(|_| DeblockError::SampleFilter)?;
            }

            if !params.prev_lossless {
                for sample_index in params.boundary - params.max_width_neg..params.boundary {
                    for lane in 0..lanes {
                        rows[sample_index][column_start + lane] = lines[lane][sample_index];
                    }
                }
            }
            if !params.curr_lossless {
                let width = params.max_width_neg.max(params.max_width_pos);
                for sample_index in params.boundary..params.boundary + width {
                    for lane in 0..lanes {
                        rows[sample_index][column_start + lane] = lines[lane][sample_index];
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
    plane_ctx: &mut PlaneCtx<'_, T>,
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
    plane_ctx: &PlaneCtx<'_, T>,
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
            .get(row)
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
        if let Some(rows) = plane_ctx.rows.get(start..start + 2 * GATHER_HALF)
            && rows.iter().all(|row| x < row.len())
        {
            for (lane, row) in line.iter_mut().zip(rows) {
                *lane = row[x];
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

fn plane_tx(plane: usize, info: MiBlockInfo) -> usize {
    if plane == 0 {
        info.luma_tx
    } else {
        info.chroma_tx.unwrap_or(0)
    }
}

fn plane_index_to_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

#[derive(Clone)]
struct MiGrid {
    mi_cols: usize,
    cells: Vec<Option<MiBlockInfo>>,
}

impl MiGrid {
    fn get(&self, row: usize, col: usize) -> Option<MiBlockInfo> {
        self.cells.get(row * self.mi_cols + col).copied().flatten()
    }
}

fn build_mi_grid(
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
) -> Result<MiGrid, DeblockError> {
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(DeblockError::Workspace)?;
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Workspace)?;
    cells.resize(count, None);

    for block in blocks {
        let info = MiBlockInfo::from_block(*block);
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    cells[rr * mi_cols + cc] = Some(info);
                }
            }
        }
    }
    Ok(MiGrid { mi_cols, cells })
}

fn overlay_mi_grid(grid: &mut MiGrid, blocks: &[DeblockBlock], mi_rows: usize, mi_cols: usize) {
    for block in blocks.iter().filter(|block| !block.chroma_transform_only) {
        let info = MiBlockInfo::from_block(*block);
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc] = Some(info);
                }
            }
        }
    }
    for block in blocks.iter().filter(|block| block.chroma_transform_only) {
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows
                    && cc < mi_cols
                    && let Some(info) = grid.cells[rr * mi_cols + cc].as_mut()
                {
                    info.chroma_base_row = block.chroma_base_r;
                    info.chroma_base_col = block.chroma_base_c;
                    info.chroma_tx = block.chroma_tx;
                }
            }
        }
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
