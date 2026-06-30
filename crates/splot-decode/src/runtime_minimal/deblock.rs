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

            for r in (0..mi_rows).step_by(plane_pass.row_step) {
                for c in (0..mi_cols).step_by(plane_pass.col_step) {
                    deblock_filter_edge(
                        workspace,
                        plane_grid,
                        plane_pass.edge_context(r, c, mi_rows, mi_cols),
                    )?;
                }
            }
        }
    }

    Ok(())
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

    fn edge_context(self, row: usize, col: usize, mi_rows: usize, mi_cols: usize) -> EdgeContext {
        EdgeContext {
            plane: self.plane,
            plane_id: self.plane_id,
            pass: self.pass,
            row,
            col,
            mi_rows,
            mi_cols,
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
    plane_id: PlaneId,
    pass: usize,
    row: usize,
    col: usize,
    mi_rows: usize,
    mi_cols: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    df_delta_q: i32,
    quant_delta: i32,
    bit_depth: BitDepth,
}

fn deblock_filter_edge<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    grid: &MiGrid,
    ctx: EdgeContext,
) -> Result<(), DeblockError> {
    let EdgeContext {
        plane,
        plane_id,
        pass,
        row,
        col,
        mi_rows,
        mi_cols,
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

    let skip = false;
    let is_sub_pu_edge = false;

    let x_r = x_p - base_x;
    let y_r = y_p - base_y;
    let is_block_edge = (pass == 0 && x_r == 0) || (pass == 1 && y_r == 0);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;

    let (curr_q, curr_side) = adaptive_strength(
        deblock_level(curr.qindex, quant_delta, df_delta_q, bit_depth),
        bit_depth,
    );
    let (prev_q, prev_side) = adaptive_strength(
        deblock_level(prev.qindex, quant_delta, df_delta_q, bit_depth),
        bit_depth,
    );

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

    let plane_width = (mi_cols * MI_SIZE) >> plane_sub_x;
    let plane_height = (mi_rows * MI_SIZE) >> plane_sub_y;
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
        workspace,
        plane_id,
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
        apply_sample_filter(
            workspace,
            plane_id,
            PerpLine::new(px, py, dx, dy),
            sample_params,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn choose_filter_width<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
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

    let s = gather_line(workspace, plane_id, PerpLine::new(x_p, y_p, dx, dy))?;
    let end = MI_SIZE - 1;
    let t_x = x_p + dy * end;
    let t_y = y_p + dx * end;
    let t = gather_line(workspace, plane_id, PerpLine::new(t_x, t_y, dx, dy))?;

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
    workspace: &mut CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    perp: PerpLine,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let mut line = gather_line(workspace, plane_id, perp)?;
    let before = line.clone();

    deblock_sample_filter(&mut line, &params).map_err(|_| DeblockError::SampleFilter)?;

    for (idx, (&new, &old)) in line.iter().zip(before.iter()).enumerate() {
        let changed = new.to_u16() != old.to_u16();
        if !changed {
            continue;
        }
        let (fx, fy) = perp.offset(idx as isize - params.boundary as isize)?;
        workspace
            .set_reconstructed_sample(plane_id, fx, fy, new)
            .map_err(|_| DeblockError::Workspace)?;
    }
    Ok(())
}

fn gather_line<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    perp: PerpLine,
) -> Result<Vec<T>, DeblockError> {
    let total = 2 * GATHER_HALF;
    let plane = workspace
        .plane(plane_id)
        .map_err(|_| DeblockError::Workspace)?;
    let max_x = plane.storage_size().width().saturating_sub(1) as isize;
    let max_y = plane.storage_size().height().saturating_sub(1) as isize;
    let mut line = Vec::new();
    line.try_reserve_exact(total)
        .map_err(|_| DeblockError::Workspace)?;
    for idx in 0..total {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
        let sample = workspace
            .reconstructed_sample(plane_id, sx, sy)
            .map_err(|_| DeblockError::Workspace)?;
        line.push(sample);
    }
    Ok(line)
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
        let info = block_info(*block);
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
        let info = block_info(*block);
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc] = Some(info);
                }
            }
        }
    }
}

const fn block_info(block: DeblockBlock) -> MiBlockInfo {
    MiBlockInfo {
        base_row: block.r,
        base_col: block.c,
        luma_tx: block.luma_tx,
        chroma_tx: block.chroma_tx,
        qindex: block.qindex,
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
    use super::super::test_support::yuv420_workspace;
    use super::*;

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

    fn assert_smoothed_step(p0: u8, q0: u8, reason: &str) {
        assert!(
            (100..=108).contains(&p0) && (100..=108).contains(&q0),
            "smoothing stays within the step band: p0={p0} q0={q0}"
        );
        assert!(p0 > 100 || q0 < 108, "{reason}: p0={p0} q0={q0}");
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
        apply_sample_filter(
            &mut workspace,
            PlaneId::Y,
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
