// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 7.17.7 per-line deblocking of one edge, kept as the oracle the
//! batched chunk kernels are tested against.

pub(super) use splot_core::tables::conversion::{Q_FIRST, Q_THRESH_MULTS, W_MULT};
pub(super) use splot_recon::{
    DeblockFilterChoice, DeblockSampleFilter, deblock_filter_choice, deblock_filter_choice_strided,
    deblock_sample_filter, deblock_sample_filter_strided, deblock_sample_filter_strided_4,
};
pub(super) use std::num::NonZeroUsize;

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_tip_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
    lanes: usize,
    q_thr: i32,
    side: i32,
    (max_width_neg, max_width_pos): (usize, usize),
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

#[allow(clippy::too_many_arguments)]
pub(super) fn choose_filter_width<T: ReconSample>(
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
        && x_p >= GATHER_HALF
        && x_p <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y_p >= plane_ctx.y_origin
        && y_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y_p >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y_p <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.width);
    if horizontal || vertical {
        let (samples, stride) = plane_ctx.rows.contiguous();
        let first_boundary = (y_p - plane_ctx.y_origin) * stride + x_p;
        let perpendicular_stride = if horizontal { 1 } else { stride };
        let lane_stride = if horizontal { stride } else { 1 };
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

#[derive(Clone, Copy)]
pub(super) struct PerpLine {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) dx: usize,
    pub(super) dy: usize,
}

impl PerpLine {
    pub(super) const fn new(x: usize, y: usize, dx: usize, dy: usize) -> Self {
        Self { x, y, dx, dy }
    }

    pub(super) fn offset(self, offset: isize) -> Result<(usize, usize), DeblockError> {
        let fx = (self.x as isize + offset * self.dx as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        let fy = (self.y as isize + offset * self.dy as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        Ok((fx, fy))
    }
}

pub(super) fn apply_edge_samples<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    perp: PerpLine,
    lanes: usize,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let PerpLine { x, y, dx, dy } = perp;
    let lanes = lanes.min(if dx == 1 {
        plane_ctx.height.saturating_sub(y)
    } else {
        plane_ctx.width.saturating_sub(x)
    });
    if lanes == 0 {
        return Ok(());
    }
    let horizontal = dx == 1
        && dy == 0
        && x >= GATHER_HALF
        && x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y >= plane_ctx.y_origin
        && y.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.width);
    if lanes <= MI_SIZE && params.boundary == GATHER_HALF && (horizontal || vertical) {
        let y_origin = plane_ctx.y_origin;
        let (samples, stride) = plane_ctx.rows.contiguous_mut();
        let boundary = (y - y_origin) * stride + x;
        let perpendicular_stride = if horizontal { 1 } else { stride };
        let lane_stride = if horizontal { stride } else { 1 };
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
    for lane in 0..lanes {
        apply_sample_filter(
            plane_ctx,
            PerpLine::new(x + dy * lane, y + dx * lane, dx, dy),
            params,
        )?;
    }
    Ok(())
}

pub(super) fn apply_sample_filter<T: ReconSample>(
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
        plane_ctx.set_sample(fx, fy, new)?;
    }
    Ok(())
}

pub(super) fn gather_line<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, '_, T>,
    perp: PerpLine,
) -> [T; 2 * GATHER_HALF] {
    let mut line = [T::default(); 2 * GATHER_HALF];
    if perp.x >= GATHER_HALF
        && perp.y >= plane_ctx.y_origin
        && perp.x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && perp.dx == 1
        && perp.dy == 0
    {
        let row = perp.y - plane_ctx.y_origin;
        let start = perp.x - GATHER_HALF;
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
        && perp.x < plane_ctx.width
        && perp.y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && perp.dx == 0
        && perp.dy == 1
    {
        let start = perp.y - GATHER_HALF - plane_ctx.y_origin;
        let x = perp.x;
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
    let min_y = plane_ctx.y_origin as isize;
    let max_y = (plane_ctx.y_origin + plane_ctx.band_rows).saturating_sub(1) as isize;
    for (idx, lane) in line.iter_mut().enumerate() {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(min_y, max_y) as usize;
        *lane = plane_ctx.sample(sx, sy);
    }
    line
}

/// Chooses and filters the edge `(x_p, y_p, edge)` line by line on clamped
/// gathers.
pub(super) fn filter_edge_lines<T: ReconSample, const PASS: usize>(
    (x_p, y_p, edge): (usize, usize, DeblockEdge),
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let (dx, dy) = if PASS == 0 { (1, 0) } else { (0, 1) };
    let (max_width_neg, max_width_pos) = (edge.max_width_neg(), edge.max_width_pos());
    let (q_thr, side) = (i32::from(edge.q_thr()), i32::from(edge.side()));
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
    let sample_params = DeblockSampleFilter {
        boundary: GATHER_HALF,
        q_thr,
        max_width_neg: eff_neg,
        max_width_pos: eff_pos,
        q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
        w_mult_neg: W_MULT[eff_neg - 1],
        w_mult_pos: W_MULT[eff_pos - 1],
        prev_lossless: edge.prev_lossless(),
        curr_lossless: edge.curr_lossless(),
        bit_depth,
    };
    apply_edge_samples(
        plane_ctx,
        PerpLine::new(x_p, y_p, dx, dy),
        MI_SIZE,
        sample_params,
    )
}
