// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{DeblockingFilterParams, QuantizationParams};
use splot_core::tables::conversion::{
    Q_FIRST, Q_THRESH_MULTS, SIDE_THRESHOLDS, TX_HEIGHT, TX_WIDTH, W_MULT,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DeblockFilterChoice, DeblockSampleFilter, PlaneId,
    ReconSample, deblock_adaptive_filter_strength, deblock_filter_choice, deblock_filter_max_width,
    deblock_sample_filter, deblock_side_threshold_index, max_quantizer_index,
};

const MI_SIZE: usize = 4;

const SB_SIZE: usize = 64;

/// Deblocking geometry for one decoded leaf block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeblockBlock {
    /// Luma MI row.
    pub(crate) r: usize,
    /// Luma MI column.
    pub(crate) c: usize,
    /// Width in luma 4x4 units.
    pub(crate) n4w: usize,
    /// Height in luma 4x4 units.
    pub(crate) n4h: usize,
    /// Luma transform index.
    pub(crate) luma_tx: usize,
    /// Chroma transform index, if present.
    pub(crate) chroma_tx: Option<usize>,
    /// Current luma AC qindex for this transform block.
    pub(crate) qindex: u32,
    /// Whether this decoded block used the skip residual path.
    pub(crate) skip: bool,
}

/// Frame-level quantizer-index deltas used by chroma deblocking.
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
    luma_tx: usize,
    chroma_tx: Option<usize>,
    qindex: u32,
    skip: bool,
}

impl MiBlockInfo {
    const fn from_block(block: DeblockBlock) -> Self {
        let DeblockBlock {
            r,
            c,
            n4w: _,
            n4h: _,
            luma_tx,
            chroma_tx,
            qindex,
            skip,
        } = block;
        Self {
            base_row: r,
            base_col: c,
            luma_tx,
            chroma_tx,
            qindex,
            skip,
        }
    }
}

/// Applies AV2 § 7.17 deblocking in place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deblock_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    blocks: &[DeblockBlock],
    chroma_blocks: [&[DeblockBlock]; 2],
    mi_rows: usize,
    mi_cols: usize,
    filter: DeblockingFilterParams,
    quant_deltas: DeblockQuantDeltas,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    if filter.apply_deblocking_filter == [false; 4] {
        return Ok(());
    }

    let grid = build_mi_grid(blocks, mi_rows, mi_cols)?;
    let mut chroma_grids = [
        build_mi_grid(blocks, mi_rows, mi_cols)?,
        build_mi_grid(blocks, mi_rows, mi_cols)?,
    ];
    overlay_mi_grid(&mut chroma_grids[0], chroma_blocks[0], mi_rows, mi_cols);
    overlay_mi_grid(&mut chroma_grids[1], chroma_blocks[1], mi_rows, mi_cols);

    for plane in 0..3 {
        let plane_grid = if plane == 0 {
            &grid
        } else {
            &chroma_grids[plane - 1]
        };
        for pass in 0..2usize {
            let Some(plane_pass) = PlanePass::active(plane, pass, filter, quant_deltas, bit_depth)
            else {
                continue;
            };
            deblock_plane_pass(workspace, plane_grid, plane_pass, mi_rows, mi_cols)?;
        }
    }

    Ok(())
}

fn deblock_plane_pass<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    grid: &MiGrid,
    plane_pass: PlanePass,
    mi_rows: usize,
    mi_cols: usize,
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
    let mut ctx = PlaneCtx::new(view.samples_mut(), stride, width, height)?;
    let mut strengths = StrengthCache::default();
    for r in (0..mi_rows).step_by(plane_pass.row_step) {
        for c in (0..mi_cols).step_by(plane_pass.col_step) {
            deblock_filter_edge(
                &mut ctx,
                grid,
                plane_pass.edge_context(r, c),
                &mut strengths,
            )?;
        }
    }
    Ok(())
}

/// Direct sample access for one plane pass, with the bounds proof hoisted out
/// of the per-sample loops: `width <= stride` and `stride * height <=
/// samples.len()` are validated once, so every clamped `(x, y)` with
/// `x < width, y < height` indexes in bounds.
struct PlaneCtx<'a, T: ReconSample> {
    samples: &'a mut [T],
    stride: usize,
    width: usize,
    height: usize,
}

impl<'a, T: ReconSample> PlaneCtx<'a, T> {
    fn new(
        samples: &'a mut [T],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self, DeblockError> {
        let required = stride.checked_mul(height).ok_or(DeblockError::Workspace)?;
        if width > stride || required > samples.len() {
            return Err(DeblockError::Workspace);
        }
        Ok(Self {
            samples,
            stride,
            width,
            height,
        })
    }
}

/// Per-pass memo for [`adaptive_strength`]; `quant_delta` / `df_delta_q` /
/// `bit_depth` are pass constants, so the key is the block `qindex` (a handful
/// of distinct values per frame, hence the linear scan).
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
}

impl PlanePass {
    fn active(
        plane: usize,
        pass: usize,
        filter: DeblockingFilterParams,
        quant_deltas: DeblockQuantDeltas,
        bit_depth: BitDepth,
    ) -> Option<Self> {
        let apply_index = if plane == 0 { pass } else { plane + 1 };
        if !filter.apply_deblocking_filter[apply_index] {
            return None;
        }

        let (plane_sub_x, plane_sub_y) = if plane == 0 { (0, 0) } else { (1, 1) };
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
        })
    }

    fn edge_context(self, row: usize, col: usize) -> EdgeContext {
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
}

fn deblock_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, T>,
    grid: &MiGrid,
    ctx: EdgeContext,
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
    } = ctx;

    let (dx, dy) = if pass == 0 { (1usize, 0usize) } else { (0, 1) };

    let x = col * MI_SIZE;
    let y = row * MI_SIZE;

    let sb_edge = pass == 1 && y.is_multiple_of(SB_SIZE);

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

    let base_row = curr.base_row;
    let base_col = curr.base_col;
    let base_y = (base_row * MI_SIZE) >> plane_sub_y;
    let base_x = (base_col * MI_SIZE) >> plane_sub_x;

    let Some(tx_sz) = plane_tx(plane, curr) else {
        return Ok(());
    };
    let Some(prev_tx_sz) = plane_tx(plane, prev) else {
        return Ok(());
    };

    let tx_col_base = curr.base_col;
    let tx_row_base = curr.base_row;
    let prev_tx_col_base = prev.base_col;
    let prev_tx_row_base = prev.base_row;

    let skip = curr.skip;
    let is_sub_pu_edge = false;

    let x_r = x_p - base_x;
    let y_r = y_p - base_y;
    let is_block_edge = (pass == 0 && x_r == 0) || (pass == 1 && y_r == 0);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;

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

    let filter_size = if pass == 0 {
        TX_WIDTH[tx_sz].min(TX_WIDTH[prev_tx_sz])
    } else {
        TX_HEIGHT[tx_sz].min(TX_HEIGHT[prev_tx_sz])
    };
    let mut filter_size = usize::try_from(filter_size).unwrap_or(0);

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
        prev_lossless: false,
        curr_lossless: false,
        bit_depth,
    };

    for i in 0..MI_SIZE {
        let px = x_p + dy * i;
        let py = y_p + dx * i;
        apply_sample_filter(plane_ctx, PerpLine::new(px, py, dx, dy), sample_params)?;
    }

    Ok(())
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
        plane_ctx.samples[fy * plane_ctx.stride + fx] = new;
    }
    Ok(())
}

fn gather_line<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, T>,
    perp: PerpLine,
) -> [T; 2 * GATHER_HALF] {
    let max_x = plane_ctx.width.saturating_sub(1) as isize;
    let max_y = plane_ctx.height.saturating_sub(1) as isize;
    let mut line = [T::default(); 2 * GATHER_HALF];
    for (idx, lane) in line.iter_mut().enumerate() {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
        *lane = plane_ctx.samples[sy * plane_ctx.stride + sx];
    }
    line
}

const DF_DELTA_SCALE: i32 = 8;

fn deblock_level(qindex: u32, quant_delta: i32, df_delta_q: i32, bit_depth: BitDepth) -> u32 {
    let q_clamped = q_clamped(qindex, quant_delta, bit_depth);
    let level = i64::from(q_clamped) + i64::from(df_delta_q) * i64::from(DF_DELTA_SCALE);
    if level <= 0 {
        0
    } else if level > i64::from(u32::MAX) {
        u32::MAX
    } else {
        level as u32
    }
}

fn q_clamped(qindex: u32, delta: i32, bit_depth: BitDepth) -> u32 {
    if qindex == 0 && delta <= 0 {
        return 0;
    }
    let max = i64::from(max_quantizer_index(bit_depth));
    (i64::from(qindex) + i64::from(delta)).clamp(1, max) as u32
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

fn plane_tx(plane: usize, info: MiBlockInfo) -> Option<usize> {
    if plane == 0 {
        Some(info.luma_tx)
    } else {
        info.chroma_tx
    }
}

fn plane_index_to_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

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
    for block in blocks {
        let info = MiBlockInfo::from_block(*block);
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc] = Some(info);
                }
            }
        }
    }
}

/// Errors from deblocking orchestration.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeblockError {
    /// A visited MI position is uncovered.
    #[error("deblocking MI ({row}, {col}) is not covered by any decoded block")]
    UncoveredMi { row: usize, col: usize },
    /// Filter-choice rejected its inputs.
    #[error("deblocking filter-choice primitive rejected its inputs")]
    FilterChoice,
    /// Sample-filter rejected its inputs.
    #[error("deblocking sample-filter primitive rejected its inputs")]
    SampleFilter,
    /// Workspace sample access went out of range.
    #[error("deblocking workspace sample access went out of bounds")]
    Workspace,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::test_support::{yuv420_workspace, yuv420_workspace_with};
    use super::*;

    fn with_plane_ctx<T: ReconSample, R>(
        ws: &mut CurrentFrameWorkspace<T>,
        plane: PlaneId,
        f: impl FnOnce(&mut PlaneCtx<'_, T>) -> R,
    ) -> R {
        let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
        let mut frame = ws.as_frame_mut();
        let view = frame.plane_mut(plane).unwrap();
        let stride = view.stride_samples();
        let mut ctx = PlaneCtx::new(view.samples_mut(), stride, width, height).unwrap();
        f(&mut ctx)
    }

    fn deblock_blocks(mi_rows: usize, mi_cols: usize) -> Vec<DeblockBlock> {
        let mut blocks = Vec::new();
        for r in (0..mi_rows).step_by(8) {
            for c in (0..mi_cols).step_by(8) {
                blocks.push(DeblockBlock {
                    r,
                    c,
                    n4w: 8,
                    n4h: 8,
                    luma_tx: 3,
                    chroma_tx: Some(2),
                    qindex: 100,
                    skip: false,
                });
            }
        }
        blocks
    }

    const fn filter(apply_deblocking_filter: [bool; 4]) -> DeblockingFilterParams {
        DeblockingFilterParams::new(apply_deblocking_filter, [false; 4], [0; 4])
    }

    fn fill_rect(
        ws: &mut CurrentFrameWorkspace<u8>,
        plane: PlaneId,
        x_range: core::ops::Range<usize>,
        y_range: core::ops::Range<usize>,
        sample: u8,
    ) {
        for y in y_range {
            for x in x_range.clone() {
                ws.set_reconstructed_sample(plane, x, y, sample).unwrap();
            }
        }
    }

    fn run_deblock(
        ws: &mut CurrentFrameWorkspace<u8>,
        blocks: &[DeblockBlock],
        mi_rows: usize,
        mi_cols: usize,
        apply_deblocking_filter: [bool; 4],
    ) {
        deblock_general_intra_frame(
            ws,
            blocks,
            [&[], &[]],
            mi_rows,
            mi_cols,
            filter(apply_deblocking_filter),
            DeblockQuantDeltas::ZERO,
            BitDepth::Eight,
        )
        .unwrap();
    }

    fn edge_test_grid(curr_skip: bool) -> MiGrid {
        let mut cells = vec![None; 4 * 16];
        cells[4] = Some(MiBlockInfo {
            base_row: 0,
            base_col: 2,
            luma_tx: 3,
            chroma_tx: None,
            qindex: 100,
            skip: false,
        });
        cells[5] = Some(MiBlockInfo {
            base_row: 0,
            base_col: 0,
            luma_tx: 3,
            chroma_tx: None,
            qindex: 100,
            skip: curr_skip,
        });
        MiGrid { mi_cols: 16, cells }
    }

    fn assert_smoothed_step(p0: u8, q0: u8, reason: &str) {
        assert!(
            (100..=108).contains(&p0) && (100..=108).contains(&q0),
            "smoothing stays within the step band: p0={p0} q0={q0}"
        );
        assert!(p0 > 100 || q0 < 108, "{reason}: p0={p0} q0={q0}");
    }

    fn yuv420_workspace_10bit(
        width: usize,
        height: usize,
        fill: u16,
    ) -> CurrentFrameWorkspace<u16> {
        yuv420_workspace_with(BitDepth::Ten, width, height, fill)
    }

    fn splat_asymmetric<T: ReconSample>(
        ws: &mut CurrentFrameWorkspace<T>,
        plane: PlaneId,
        max_sample: u16,
    ) {
        let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let coords = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
        for (x, y) in coords {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let value = ((state >> 33) as u16) % (max_sample + 1);
            ws.set_reconstructed_sample(plane, x, y, T::try_from_u16(value).unwrap())
                .unwrap();
        }
    }

    fn reference_gather<T: ReconSample>(
        ws: &CurrentFrameWorkspace<T>,
        plane: PlaneId,
        perp: PerpLine,
    ) -> Vec<T> {
        let (width, height) = coded_plane_dimensions(ws, plane).unwrap();
        let max_x = width.saturating_sub(1) as isize;
        let max_y = height.saturating_sub(1) as isize;
        (0..2 * GATHER_HALF)
            .map(|idx| {
                let offset = idx as isize - GATHER_HALF as isize;
                let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
                let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
                ws.reconstructed_sample(plane, sx, sy).unwrap()
            })
            .collect()
    }

    fn reference_apply<T: ReconSample>(
        ws: &mut CurrentFrameWorkspace<T>,
        plane: PlaneId,
        perp: PerpLine,
        params: DeblockSampleFilter,
    ) {
        let before = reference_gather(ws, plane, perp);
        let mut line = before.clone();
        deblock_sample_filter(&mut line, &params).unwrap();
        let changed: Vec<(usize, T)> = line
            .iter()
            .zip(before.iter())
            .enumerate()
            .filter(|(_, (new, old))| new.to_u16() != old.to_u16())
            .map(|(idx, (&new, _))| (idx, new))
            .collect();
        for (idx, new) in changed {
            let (fx, fy) = perp
                .offset(idx as isize - params.boundary as isize)
                .unwrap();
            ws.set_reconstructed_sample(plane, fx, fy, new).unwrap();
        }
    }

    fn edge_and_corner_perps(width: usize, height: usize) -> Vec<PerpLine> {
        let mut perps = Vec::new();
        for &(x, y) in &[
            (GATHER_HALF, GATHER_HALF),
            (4, 0),
            (width - 4, height - 1),
            (width / 2, height / 2),
            (0, height - 4),
            (width - 1, 4),
        ] {
            perps.push(PerpLine::new(x, y, 1, 0));
            perps.push(PerpLine::new(x, y, 0, 1));
        }
        perps
    }

    fn assert_gather_and_apply_match_accessor_reference<T: ReconSample + core::fmt::Debug + Eq>(
        mut direct: CurrentFrameWorkspace<T>,
        mut reference: CurrentFrameWorkspace<T>,
        bit_depth: BitDepth,
    ) {
        let plane = PlaneId::Y;
        let max_sample = bit_depth.max_sample();
        splat_asymmetric(&mut direct, plane, max_sample);
        splat_asymmetric(&mut reference, plane, max_sample);
        let (width, height) = coded_plane_dimensions(&direct, plane).unwrap();

        for perp in edge_and_corner_perps(width, height) {
            let got = with_plane_ctx(&mut direct, plane, |ctx| gather_line(ctx, perp));
            assert_eq!(
                got.to_vec(),
                reference_gather(&reference, plane, perp),
                "gather at ({}, {}) d=({}, {})",
                perp.x,
                perp.y,
                perp.dx,
                perp.dy
            );
        }

        let params = DeblockSampleFilter {
            boundary: GATHER_HALF,
            q_thr: 60,
            max_width_neg: 4,
            max_width_pos: 4,
            q_thresh_mult: 25,
            w_mult_neg: 28,
            w_mult_pos: 28,
            prev_lossless: false,
            curr_lossless: false,
            bit_depth,
        };
        for &(x, y, dx, dy) in &[
            (GATHER_HALF, 2usize, 1usize, 0usize),
            (width / 2, height / 2, 1, 0),
            (width / 2, height / 2, 0, 1),
            (width - GATHER_HALF, height - 3, 1, 0),
            (4, GATHER_HALF, 0, 1),
        ] {
            let perp = PerpLine::new(x, y, dx, dy);
            with_plane_ctx(&mut direct, plane, |ctx| {
                apply_sample_filter(ctx, perp, params).unwrap();
            });
            reference_apply(&mut reference, plane, perp, params);
        }
        assert_eq!(
            direct.samples(plane).unwrap(),
            reference.samples(plane).unwrap(),
            "direct-slice apply must match the accessor-based reference"
        );
    }

    #[test]
    fn gather_and_apply_match_accessor_reference_8bit() {
        assert_gather_and_apply_match_accessor_reference(
            yuv420_workspace(34, 22, 0),
            yuv420_workspace(34, 22, 0),
            BitDepth::Eight,
        );
    }

    #[test]
    fn gather_and_apply_match_accessor_reference_10bit() {
        assert_gather_and_apply_match_accessor_reference(
            yuv420_workspace_10bit(34, 22, 0),
            yuv420_workspace_10bit(34, 22, 0),
            BitDepth::Ten,
        );
    }

    #[test]
    fn strength_cache_matches_direct_computation() {
        for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
            for &(quant_delta, df_delta_q) in &[(0i32, 0i32), (-6, 3), (12, -2)] {
                let mut cache = StrengthCache::default();
                for qindex in (0u32..=300).chain([1000, u32::MAX]) {
                    let direct = adaptive_strength(
                        deblock_level(qindex, quant_delta, df_delta_q, bit_depth),
                        bit_depth,
                    );
                    assert_eq!(
                        cache.get(qindex, quant_delta, df_delta_q, bit_depth),
                        direct,
                        "first lookup qindex={qindex}"
                    );
                    assert_eq!(
                        cache.get(qindex, quant_delta, df_delta_q, bit_depth),
                        direct,
                        "cached lookup qindex={qindex}"
                    );
                }
            }
        }
    }

    #[test]
    fn q_clamped_zero_delta_matches_spec() {
        for q in [0u32, 1, 100, 255] {
            assert_eq!(q_clamped(q, 0, BitDepth::Eight), q, "q_clamped({q}, 0)");
        }
    }

    #[test]
    fn adaptive_strength_for_lvl_100_8bit() {
        let (q_thr, side) = adaptive_strength(100, BitDepth::Eight);
        assert_eq!(side, 1, "side threshold for lvl 100 (8-bit)");
        assert!(q_thr > 0, "qThr must be positive for a nonzero level");
    }

    #[test]
    fn combine_strengths_averages_then_maxes() {
        assert_eq!(
            combine_strengths(3, 5, 2, 4),
            ((3 + 5 + 1) >> 1, (2 + 4 + 1) >> 1)
        );
        assert_eq!(combine_strengths(0, 5, 0, 4), (5, 4));
        assert_eq!(combine_strengths(3, 0, 2, 0), (3, 2));
    }

    #[test]
    fn empty_apply_pattern_is_a_no_op() {
        let mut workspace = yuv420_workspace(64, 64, 100);
        deblock_general_intra_frame(
            &mut workspace,
            &[],
            [&[], &[]],
            16,
            16,
            filter([false; 4]),
            DeblockQuantDeltas::ZERO,
            BitDepth::Eight,
        )
        .unwrap();
        assert!(
            workspace
                .samples(PlaneId::Y)
                .unwrap()
                .iter()
                .all(|&s| s == 100),
            "no-op deblock leaves the workspace untouched"
        );
    }

    #[test]
    fn unchanged_border_taps_do_not_require_in_frame_write_coordinates() {
        let mut workspace = yuv420_workspace(16, 16, 100);
        with_plane_ctx(&mut workspace, PlaneId::Y, |ctx| {
            apply_sample_filter(
                ctx,
                PerpLine::new(4, 0, 1, 0),
                DeblockSampleFilter {
                    boundary: GATHER_HALF,
                    q_thr: 1,
                    max_width_neg: GATHER_HALF,
                    max_width_pos: GATHER_HALF,
                    q_thresh_mult: 1,
                    w_mult_neg: 1,
                    w_mult_pos: 1,
                    prev_lossless: true,
                    curr_lossless: true,
                    bit_depth: BitDepth::Eight,
                },
            )
            .unwrap();
        });
    }

    #[test]
    fn deblock_bounds_use_coded_plane_storage_for_partial_edge_frame() {
        let workspace = yuv420_workspace(18, 14, 100);
        assert_eq!(
            coded_plane_dimensions(&workspace, PlaneId::Y).unwrap(),
            (18, 14)
        );
        assert_eq!(
            coded_plane_dimensions(&workspace, PlaneId::U).unwrap(),
            (9, 7)
        );
        assert_eq!(
            coded_plane_dimensions(&workspace, PlaneId::V).unwrap(),
            (9, 7)
        );
    }

    #[test]
    fn mi_grid_covers_decoded_blocks() {
        let blocks = [DeblockBlock {
            r: 0,
            c: 0,
            n4w: 8,
            n4h: 8,
            luma_tx: 3,
            chroma_tx: Some(2),
            qindex: 100,
            skip: false,
        }];
        let grid = build_mi_grid(&blocks, 16, 16).unwrap();
        assert!(grid.get(0, 0).is_some(), "top-left MI is covered");
        assert!(
            grid.get(7, 7).is_some(),
            "bottom-right of the 8x8 footprint is covered"
        );
        assert!(
            grid.get(8, 8).is_none(),
            "an MI outside the block is uncovered"
        );
        let info = grid.get(0, 0).unwrap();
        assert_eq!((info.base_row, info.base_col), (0, 0));
        assert_eq!(plane_tx(0, info), Some(3), "luma tx index");
        assert_eq!(plane_tx(1, info), Some(2), "chroma tx index");
    }

    #[test]
    fn skip_suppresses_internal_tx_edge_filtering() {
        let mut skipped = yuv420_workspace(64, 16, 100);
        fill_rect(&mut skipped, PlaneId::Y, 20..64, 0..16, 108);
        with_plane_ctx(&mut skipped, PlaneId::Y, |ctx| {
            deblock_filter_edge(
                ctx,
                &edge_test_grid(true),
                EdgeContext {
                    plane: 0,
                    pass: 0,
                    row: 0,
                    col: 5,
                    plane_sub_x: 0,
                    plane_sub_y: 0,
                    df_delta_q: 0,
                    quant_delta: 0,
                    bit_depth: BitDepth::Eight,
                },
                &mut StrengthCache::default(),
            )
            .unwrap();
        });
        assert_eq!(
            skipped.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
            100,
            "skipped internal edge leaves the previous tap unchanged"
        );
        assert_eq!(
            skipped.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
            108,
            "skipped internal edge leaves the current tap unchanged"
        );

        let mut coded = yuv420_workspace(64, 16, 100);
        fill_rect(&mut coded, PlaneId::Y, 20..64, 0..16, 108);
        with_plane_ctx(&mut coded, PlaneId::Y, |ctx| {
            deblock_filter_edge(
                ctx,
                &edge_test_grid(false),
                EdgeContext {
                    plane: 0,
                    pass: 0,
                    row: 0,
                    col: 5,
                    plane_sub_x: 0,
                    plane_sub_y: 0,
                    df_delta_q: 0,
                    quant_delta: 0,
                    bit_depth: BitDepth::Eight,
                },
                &mut StrengthCache::default(),
            )
            .unwrap();
        });
        assert_smoothed_step(
            coded.reconstructed_sample(PlaneId::Y, 19, 0).unwrap(),
            coded.reconstructed_sample(PlaneId::Y, 20, 0).unwrap(),
            "coded internal edge still filters",
        );
    }

    #[test]
    fn luma_vertical_pass_filters_the_x64_block_edge() {
        let mut ws = yuv420_workspace(128, 64, 100);
        fill_rect(&mut ws, PlaneId::Y, 64..128, 0..64, 108);
        let blocks = deblock_blocks(16, 32);
        run_deblock(&mut ws, &blocks, 16, 32, [true, false, false, false]);
        let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
        assert_eq!(at(10, 32), 100, "left interior untouched");
        assert_eq!(at(120, 32), 108, "right interior untouched");
        assert_eq!(at(31, 32), 100, "x=32 within-region edge untouched");
        assert_eq!(at(32, 32), 100, "x=32 within-region edge untouched");
        assert_eq!(at(95, 32), 108, "x=96 within-region edge untouched");
        assert_eq!(at(96, 32), 108, "x=96 within-region edge untouched");
        assert_smoothed_step(
            at(63, 32),
            at(64, 32),
            "luma-vertical pass must change the x=64 edge",
        );
    }

    #[test]
    fn luma_horizontal_pass_filters_the_y64_superblock_edge() {
        let mut ws = yuv420_workspace(128, 128, 100);
        fill_rect(&mut ws, PlaneId::Y, 0..128, 64..128, 108);
        let blocks = deblock_blocks(32, 32);
        run_deblock(&mut ws, &blocks, 32, 32, [false, true, false, false]);
        let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
        assert_eq!(at(64, 10), 100, "top interior untouched");
        assert_eq!(at(64, 120), 108, "bottom interior untouched");
        assert_eq!(at(64, 31), 100, "y=32 within-region edge untouched");
        assert_eq!(at(64, 96), 108, "y=96 within-region edge untouched");
        assert_smoothed_step(
            at(64, 63),
            at(64, 64),
            "luma-horizontal sbEdge pass must change the y=64 edge",
        );
        assert_eq!(
            at(64, 59),
            100,
            "sbEdge caps the upward extent (row 59 unchanged)"
        );
    }

    #[test]
    fn chroma_pass_filters_the_chroma_block_edge() {
        let mut ws = yuv420_workspace(128, 64, 100);
        fill_rect(&mut ws, PlaneId::U, 32..64, 0..32, 108);
        let blocks = deblock_blocks(16, 32);
        run_deblock(&mut ws, &blocks, 16, 32, [false, false, true, false]);
        let u = |x, y| ws.reconstructed_sample(PlaneId::U, x, y).unwrap();
        assert_eq!(u(8, 16), 100, "left chroma interior untouched");
        assert_eq!(u(60, 16), 108, "right chroma interior untouched");
        assert_smoothed_step(
            u(31, 16),
            u(32, 16),
            "chroma pass must change the chroma x=32 edge",
        );
        assert_eq!(
            ws.reconstructed_sample(PlaneId::V, 31, 16).unwrap(),
            100,
            "V plane untouched (apply[3] == false)"
        );
    }
}
