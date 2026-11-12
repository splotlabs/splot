// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Full-pel compound cells of a motion grid predicted together: a row's runs
//! of equal cells, or a whole grid of one cell, as one block each.

use super::*;

/// Predicts each run of full-width, full-pel cells in one grid row that share
/// their motion and clipping-bound source as one block of at most 64 samples,
/// and returns a mask of the cells it predicted. With `second`, the same runs
/// are predicted from those views into that output too (the V plane of a
/// shared U pass), and the second mask holds its cells.
///
/// Each cell's bounds are the run's first cell's unclamped bounds shifted by
/// the cell's offset, so when the first cell reads inside its bounds and the
/// whole run reads inside the plane, no cell clamps a column. Rows clamp
/// identically.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn predict_fullpel_runs<T: ReconSample, O: CompoundAverageOutput>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cells: &[MotionCell],
    refine: (&[[Mv; 2]], usize, usize),
    row_index: usize,
    [row, subblock_w, height]: [usize; 3],
    (sub_x, sub_y): (u32, u32),
    (bit_depth, interp, subblock_area): (
        splot_recon::BitDepth,
        InterpolationFilter,
        Option<(usize, usize)>,
    ),
    (uniform_everywhere, implicit_mask, cwp_weight, frame): (bool, bool, i16, (usize, usize)),
    output: &mut [O],
    output_stride: usize,
    mut second: Option<SecondPlane<'_, '_, T, O>>,
) -> Result<[u64; 2]> {
    let columns = (prediction.block_w / subblock_w).min(64);
    let row_cells = cells
        .get(row_index..row_index + columns)
        .unwrap_or_default();
    let candidate = |column: usize| refine.0.get((row_index + column) * refine.1);
    let mut merged = [0u64; 2];
    let mut start = 0;
    while start + 1 < row_cells.len() {
        let cell = row_cells[start];
        let mut run = 1;
        if row_cells[start + 1] == cell && fullpel_motion(cell.mvs, sub_x, sub_y) {
            while start + run < row_cells.len()
                && (run + 1) * subblock_w <= 64
                && row_cells[start + run] == cell
                && candidate(start + run) == candidate(start)
            {
                run += 1;
            }
        }
        let col = start * subblock_w;
        if run > 1
            && let Some(params) = fullpel_rect_params(
                prediction,
                cell,
                cell_clip_area(
                    refine,
                    cell,
                    row_index + start,
                    subblock_area,
                    (sub_x, sub_y),
                ),
                [col, row, run * subblock_w, height],
                (sub_x, sub_y),
                (bit_depth, interp),
                (uniform_everywhere, implicit_mask, cwp_weight, frame),
            )
        {
            let run_cells = ((1 << run) - 1) << start;
            if O::predict_fast(
                &prediction.views[0],
                &params[0],
                &prediction.views[1],
                &params[1],
                cwp_weight,
                &mut output[col..],
                output_stride,
            )? {
                merged[0] |= run_cells;
            }
            if let Some((views, output, output_stride)) = &mut second
                && O::predict_fast(
                    &views[0],
                    &params[0],
                    &views[1],
                    &params[1],
                    cwp_weight,
                    &mut output[col..],
                    *output_stride,
                )?
            {
                merged[1] |= run_cells;
            }
        }
        start += run;
    }
    Ok(merged)
}

/// The area whose § 7.13.3.5 bounds clip grid cell `cell_index`, as
/// [`compound_optflow_bounds`] picks it, or `None` when only the plane clips
/// the cell.
fn cell_clip_area(
    (candidates, step, unit_size): (&[[Mv; 2]], usize, usize),
    cell: MotionCell,
    cell_index: usize,
    subblock_area: Option<(usize, usize)>,
    (sub_x, sub_y): (u32, u32),
) -> Option<ClipArea> {
    match candidates.get(cell_index * step) {
        Some(candidates) => Some(ClipArea {
            size: [unit_size >> sub_x, unit_size >> sub_y],
            centre: *candidates,
            shrinks: true,
        }),
        None => subblock_area.map(|(width, height)| ClipArea {
            size: [width, height],
            centre: cell.base_mvs,
            shrinks: false,
        }),
    }
}

/// The areas that tile a grid plane from its origin, each clipping its cells
/// by bounds centred on `centre`. A refine-MV unit `shrinks` to the block at
/// its right and bottom edges; a subblock area keeps its size.
#[derive(Clone, Copy)]
struct ClipArea {
    size: [usize; 2],
    centre: [Mv; 2],
    shrinks: bool,
}

/// The parameters that predict the rectangle `[col, row, width, height]` of a
/// grid plane, whose cells all hold the full-pel `cell` and are clipped by
/// areas of one size centred on one motion, as one block; `None` when that
/// could change a sample, or the rectangle's weights are not uniform.
///
/// Along an axis the rectangle keeps inside one area, every cell shares that
/// area's bounds. Along any other axis the areas are the first one moved by
/// whole areas, so when the first area's reach lies inside its bounds and the
/// rectangle's reach inside the plane, no cell's clip moves a sample there.
#[allow(clippy::too_many_arguments)]
fn fullpel_rect_params<T: ReconSample>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cell: MotionCell,
    area: Option<ClipArea>,
    [col, row, width, height]: [usize; 4],
    (sub_x, sub_y): (u32, u32),
    (bit_depth, interp): (splot_recon::BitDepth, InterpolationFilter),
    (uniform_everywhere, implicit_mask, cwp_weight, frame): (bool, bool, i16, (usize, usize)),
) -> Option<[SubpelPredictParams; 2]> {
    if !fullpel_motion(cell.mvs, sub_x, sub_y) {
        return None;
    }
    let origin = [prediction.plane_x + col, prediction.plane_y + row].map(|value| value as i32);
    let scalings: [PlaneScaling; 2] = core::array::from_fn(|reference| {
        prediction.scalings[reference].with_prescaled_mv(
            origin[0],
            origin[1],
            cell.mvs[reference][0],
            cell.mvs[reference][1],
            sub_x,
            sub_y,
        )
    });
    if !(uniform_everywhere
        || super::compound_average_weights_are_uniform(
            implicit_mask,
            cwp_weight,
            width,
            height,
            prediction.scalings,
            Some(scalings),
            frame,
        ))
    {
        return None;
    }
    let block = [prediction.block_w, prediction.block_h];
    let (rect, extent) = ([col, row], [width, height]);
    let params = |reference: usize| -> Option<SubpelPredictParams> {
        let (plane, view) = (
            &prediction.scalings[reference],
            &prediction.views[reference],
        );
        let start = [scalings[reference].start_x, scalings[reference].start_y];
        let mut bounds = [0, 0, plane.last_x, plane.last_y];
        if let Some(area) = area {
            let offsets = [0, 1].map(|axis| rect[axis] % area.size[axis]);
            let size = [0, 1].map(|axis| {
                let size = area.size[axis];
                if area.shrinks {
                    size.min(block[axis] - (rect[axis] - offsets[axis]))
                } else {
                    size
                }
            });
            let area_bounds = super::refinemv::unscaled_area_bounds(
                [0, 1].map(|axis| (origin[axis] - offsets[axis] as i32) << 4),
                size,
                area.centre[reference],
                [sub_x, sub_y],
                plane,
            );
            let area_bounds = [
                area_bounds.first_x,
                area_bounds.first_y,
                area_bounds.last_x,
                area_bounds.last_y,
            ];
            let last = [
                plane.last_x.min(view.width() as i32 - 1),
                plane.last_y.min(view.height() as i32 - 1),
            ];
            for axis in 0..2 {
                let (offset, extent) = (offsets[axis], extent[axis]);
                let (first, end) = (
                    (start[axis] >> 10) - offset as i32,
                    (start[axis] >> 10) + extent as i32 - 1,
                );
                let touched = (offset + extent).div_ceil(area.size[axis]) * area.size[axis];
                if offset + extent <= size[axis] {
                    bounds[axis] = area_bounds[axis];
                    bounds[axis + 2] = area_bounds[axis + 2];
                } else if size[axis] != area.size[axis]
                    || area.shrinks && rect[axis] - offset + touched > block[axis]
                    || first < area_bounds[axis]
                    || first + size[axis] as i32 - 1 > area_bounds[axis + 2]
                    || end > last[axis]
                    || axis == 1 && view.readable_row(end as usize).is_none()
                {
                    return None;
                }
            }
        }
        Some(SubpelPredictParams {
            interp,
            w: width,
            h: height,
            start_x: start[0],
            start_y: start[1],
            step_x: 1 << 10,
            step_y: 1 << 10,
            first_x: bounds[0],
            first_y: bounds[1],
            last_x: bounds[2],
            last_y: bounds[3],
            bit_depth,
        })
    };
    Some([params(0)?, params(1)?])
}

/// The parameters that predict a grid of equal cells with equal refine-MV
/// candidates as one block; see [`fullpel_rect_params`].
#[allow(clippy::too_many_arguments)]
pub(super) fn uniform_fullpel_params<T: ReconSample>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cells: &[MotionCell],
    refine: (&[[Mv; 2]], usize, usize),
    (sub_x, sub_y): (u32, u32),
    (bit_depth, interp, subblock_area): (
        splot_recon::BitDepth,
        InterpolationFilter,
        Option<(usize, usize)>,
    ),
    weights: (bool, bool, i16, (usize, usize)),
) -> Option<[SubpelPredictParams; 2]> {
    let cell = *cells.first()?;
    let candidates = refine.0.get(..cells.len() * refine.1)?;
    if cells.iter().any(|other| *other != cell)
        || candidates.iter().any(|other| *other != candidates[0])
    {
        return None;
    }
    fullpel_rect_params(
        prediction,
        cell,
        cell_clip_area(refine, cell, 0, subblock_area, (sub_x, sub_y)),
        [0, 0, prediction.block_w, prediction.block_h],
        (sub_x, sub_y),
        (bit_depth, interp),
        weights,
    )
}
