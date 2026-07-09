// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{CcsoPlaneParams, FrameHeaderCore, ccso_quant_step};
use splot_parallel::prelude::*;
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};

const CCSO_PLANES: usize = 3;
const CCSO_OFFSET: [i32; 8] = [0, 1, -1, 3, -3, 7, -7, -10];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CcsoUnitGrid {
    active: bool,
    shift: u32,
    plane_enabled: [bool; CCSO_PLANES],
    blocks: [Vec<u8>; CCSO_PLANES],
    grid_rows: usize,
    grid_cols: usize,
}

impl CcsoUnitGrid {
    pub(crate) fn new(
        active: bool,
        shift: u32,
        plane_enabled: [bool; CCSO_PLANES],
        blocks: [Vec<u8>; CCSO_PLANES],
        grid_rows: usize,
        grid_cols: usize,
    ) -> Result<Self, CcsoError> {
        if !active {
            return Ok(Self {
                active,
                shift,
                plane_enabled,
                blocks,
                grid_rows,
                grid_cols,
            });
        }
        let cells = grid_rows
            .checked_mul(grid_cols)
            .ok_or(CcsoError::Geometry)?;
        if blocks.iter().any(|plane| plane.len() != cells) {
            return Err(CcsoError::Geometry);
        }
        Ok(Self {
            active,
            shift,
            plane_enabled,
            blocks,
            grid_rows,
            grid_cols,
        })
    }

    const fn active(&self) -> bool {
        self.active
    }

    const fn ccso_luma_size_log2(&self) -> u32 {
        self.shift + 2
    }

    pub(crate) const fn shift(&self) -> u32 {
        self.shift
    }

    pub(crate) const fn grid_rows(&self) -> usize {
        self.grid_rows
    }

    pub(crate) const fn grid_cols(&self) -> usize {
        self.grid_cols
    }

    pub(crate) fn plane_blocks(&self, plane: usize) -> Option<&[u8]> {
        self.blocks.get(plane).map(Vec::as_slice)
    }

    fn block_value(&self, plane: usize, unit_row: usize, unit_col: usize) -> u8 {
        if unit_row >= self.grid_rows || unit_col >= self.grid_cols {
            return 0;
        }
        unit_row
            .checked_mul(self.grid_cols)
            .and_then(|start| start.checked_add(unit_col))
            .and_then(|index| self.blocks.get(plane).and_then(|grid| grid.get(index)))
            .copied()
            .unwrap_or(0)
    }
}

pub(crate) fn ccso_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    curr_luma: &[u16],
    core: &FrameHeaderCore,
    grid: &CcsoUnitGrid,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    bit_depth: BitDepth,
) -> Result<(), CcsoError> {
    if !grid.active() {
        return Ok(());
    }
    let Some(params) = core.ccso_params.as_ref() else {
        return Ok(());
    };
    let luma_size = workspace.info().coded_luma_size();
    let luma_width = luma_size.width();
    let luma_height = luma_size.height();
    if curr_luma.len()
        < luma_width
            .checked_mul(luma_height)
            .ok_or(CcsoError::Geometry)?
    {
        return Err(CcsoError::Geometry);
    }

    for plane in 0..CCSO_PLANES {
        if !grid.plane_enabled[plane] {
            continue;
        }
        let Some(plane_params) = params.planes.get(plane) else {
            return Err(CcsoError::Params);
        };
        if !plane_params.ccso_planes {
            continue;
        }
        ccso_plane(
            workspace,
            curr_luma,
            luma_width,
            luma_height,
            plane,
            plane_params,
            grid,
            lossless_grid,
            bit_depth,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ccso_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    curr_luma: &[u16],
    luma_width: usize,
    luma_height: usize,
    plane: usize,
    params: &CcsoPlaneParams,
    grid: &CcsoUnitGrid,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    bit_depth: BitDepth,
) -> Result<(), CcsoError> {
    let plane_id = plane_id(plane);
    let (sub_x, sub_y) = match plane_id {
        PlaneId::Y => (0, 0),
        PlaneId::U | PlaneId::V => {
            let pixel_format = workspace.info().pixel_format();
            (
                usize::from(pixel_format.subsampling_x()),
                usize::from(pixel_format.subsampling_y()),
            )
        }
    };
    let plane_size = workspace
        .plane(plane_id)
        .map_err(|_| CcsoError::Workspace)?
        .storage_size();
    let plane_width = plane_size.width();
    let plane_height = plane_size.height();
    let ccso_luma_log2 = grid.ccso_luma_size_log2();
    let shift_x = ccso_luma_log2
        .checked_sub(u32::try_from(sub_x).map_err(|_| CcsoError::Geometry)?)
        .ok_or(CcsoError::Geometry)?;
    let shift_y = ccso_luma_log2
        .checked_sub(u32::try_from(sub_y).map_err(|_| CcsoError::Geometry)?)
        .ok_or(CcsoError::Geometry)?;
    let blk_w = 1usize.checked_shl(shift_x).ok_or(CcsoError::Geometry)?;
    let blk_h = 1usize.checked_shl(shift_y).ok_or(CcsoError::Geometry)?;
    let max_band_log2 = params.ccso_max_band_log2.ok_or(CcsoError::Params)?;
    let ext_filter = params.ccso_ext_filter.ok_or(CcsoError::Params)?;
    let bo_only = params.ccso_bo_only.ok_or(CcsoError::Params)?;
    let edge_clf = params.ccso_edge_clf.ok_or(CcsoError::Params)?;
    let scale_idx = params.ccso_scale_idx.ok_or(CcsoError::Params)?;
    let quant_idx = params.ccso_quant_idx.ok_or(CcsoError::Params)?;
    let quant_step = i32::from(ccso_quant_step(scale_idx, quant_idx));
    let max_edge_interval = if bo_only {
        1usize
    } else if edge_clf {
        2
    } else {
        3
    };
    let max_band = 1usize
        .checked_shl(u32::from(max_band_log2))
        .ok_or(CcsoError::Geometry)?;
    let expected_offsets = max_edge_interval
        .checked_mul(max_edge_interval)
        .and_then(|count| count.checked_mul(max_band))
        .ok_or(CcsoError::Geometry)?;
    if params.ccso_offset_idx.len() != expected_offsets {
        return Err(CcsoError::Params);
    }
    let sample_offsets = ccso_sample_offsets(ext_filter)?;
    let max_sample = i32::from(bit_depth.max_sample());
    let band_shift = bit_depth
        .bits()
        .checked_sub(max_band_log2)
        .ok_or(CcsoError::Params)?;

    let mut units = Vec::new();
    for y in (0..plane_height).step_by(blk_h) {
        for x in (0..plane_width).step_by(blk_w) {
            let unit_row = (y << sub_y) >> ccso_luma_log2;
            let unit_col = (x << sub_x) >> ccso_luma_log2;
            if grid.block_value(plane, unit_row, unit_col) == 0 {
                continue;
            }
            units.push((x, y));
        }
    }
    if units.is_empty() {
        return Ok(());
    }
    if luma_width == 0 || luma_height == 0 {
        return Err(CcsoError::Geometry);
    }

    let offset_lut = ccso_offset_lut(params, expected_offsets)?;
    let source = workspace
        .plane(plane_id)
        .map_err(|_| CcsoError::Workspace)?;
    let plane_stride = source.stride_samples();
    let plane_samples = source.samples();
    let max_luma_x = luma_width - 1;
    let compute = |&(x, y): &(usize, usize)| -> Result<(PlaneRect, Vec<T>), CcsoError> {
        let y_end = plane_height.min(y.saturating_add(blk_h));
        let x_end = plane_width.min(x.saturating_add(blk_w));
        let width = x_end - x;
        let mut filtered = Vec::new();
        filtered
            .try_reserve_exact(width.checked_mul(y_end - y).ok_or(CcsoError::Geometry)?)
            .map_err(|_| CcsoError::Geometry)?;
        for y3 in y..y_end {
            let y_luma = y3 << sub_y;
            let center_row = clamped_luma_row(curr_luma, luma_width, luma_height, y_luma as isize)?;
            let offset_rows = if bo_only {
                None
            } else {
                Some((
                    clamped_luma_row(
                        curr_luma,
                        luma_width,
                        luma_height,
                        y_luma as isize + sample_offsets[0].1,
                    )?,
                    clamped_luma_row(
                        curr_luma,
                        luma_width,
                        luma_height,
                        y_luma as isize + sample_offsets[1].1,
                    )?,
                ))
            };
            let row_start = y3.checked_mul(plane_stride).ok_or(CcsoError::Workspace)?;
            let plane_row = plane_samples
                .get(row_start..row_start.checked_add(x_end).ok_or(CcsoError::Workspace)?)
                .ok_or(CcsoError::Workspace)?;
            for x3 in x..x_end {
                let x_luma = x3 << sub_x;
                let center = center_row[x_luma.min(max_luma_x)];
                let band = usize::from(center >> band_shift);
                let (cls0, cls1) = match offset_rows {
                    None => (0usize, 0usize),
                    Some((row0, row1)) => {
                        let sx0 = (x_luma as isize + sample_offsets[0].0)
                            .clamp(0, max_luma_x as isize)
                            as usize;
                        let sx1 = (x_luma as isize + sample_offsets[1].0)
                            .clamp(0, max_luma_x as isize)
                            as usize;
                        (
                            ccso_score(
                                i32::from(row0[sx0]) - i32::from(center),
                                quant_step,
                                edge_clf,
                            ),
                            ccso_score(
                                i32::from(row1[sx1]) - i32::from(center),
                                quant_step,
                                edge_clf,
                            ),
                        )
                    }
                };
                if cls0 >= max_edge_interval || cls1 >= max_edge_interval || band >= max_band {
                    return Err(CcsoError::Params);
                }
                let sample = plane_row.get(x3).ok_or(CcsoError::Workspace)?;
                if lossless_grid
                    .is_some_and(|grid| grid.plane_sample_lossless(plane_id, x3, y3, sub_x, sub_y))
                {
                    filtered.push(*sample);
                    continue;
                }
                let offset = *offset_lut
                    .get((cls0 * max_edge_interval + cls1) * max_band + band)
                    .ok_or(CcsoError::Params)?;
                let value = (i32::from(sample.to_u16()) + offset).clamp(0, max_sample);
                filtered.push(T::try_from_u16(value as u16).map_err(|_| CcsoError::Sample)?);
            }
        }
        let rect = PlaneRect::new(x, y, width, y_end - y).map_err(|_| CcsoError::Geometry)?;
        Ok((rect, filtered))
    };
    let outputs: Vec<(PlaneRect, Vec<T>)> = if splot_parallel::on_multiworker_pool() {
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let outputs = units
            .par_iter()
            .map(|unit| {
                tally.note_worker();
                compute(unit)
            })
            .collect::<Result<_, _>>()?;
        crate::timing::report_detail(
            "ccso_units",
            timer,
            &format!(
                "plane={} units={} threads={} workers_used={}",
                plane_id.index(),
                units.len(),
                splot_parallel::current_pool_width(),
                tally.workers_used()
            ),
        );
        outputs
    } else {
        units.iter().map(compute).collect::<Result<_, _>>()?
    };
    for (rect, filtered) in outputs {
        workspace
            .write_rect(plane_id, rect, &filtered, rect.width())
            .map_err(|_| CcsoError::Workspace)?;
    }
    Ok(())
}

fn ccso_offset_lut(
    params: &CcsoPlaneParams,
    expected_offsets: usize,
) -> Result<Vec<i32>, CcsoError> {
    let scale = i32::from(params.ccso_scale_idx.ok_or(CcsoError::Params)?) + 1;
    let mut lut = Vec::new();
    lut.try_reserve_exact(expected_offsets)
        .map_err(|_| CcsoError::Geometry)?;
    for &offset_idx in &params.ccso_offset_idx {
        let base = CCSO_OFFSET
            .get(usize::from(offset_idx))
            .copied()
            .ok_or(CcsoError::Params)?;
        lut.push(base * scale);
    }
    Ok(lut)
}

fn clamped_luma_row(
    curr_luma: &[u16],
    width: usize,
    height: usize,
    y: isize,
) -> Result<&[u16], CcsoError> {
    let sy = y.clamp(0, height.saturating_sub(1) as isize) as usize;
    let start = sy.checked_mul(width).ok_or(CcsoError::Geometry)?;
    curr_luma
        .get(start..start.checked_add(width).ok_or(CcsoError::Geometry)?)
        .ok_or(CcsoError::Geometry)
}

fn ccso_score(diff: i32, quant_step: i32, edge_clf: bool) -> usize {
    if diff > quant_step && !edge_clf {
        2
    } else {
        usize::from(diff >= -quant_step)
    }
}

fn ccso_sample_offsets(ext_filter: u8) -> Result<[(isize, isize); 2], CcsoError> {
    match ext_filter {
        0 => Ok([(0, -1), (0, 1)]),
        1 => Ok([(-1, 0), (1, 0)]),
        2 => Ok([(-1, -1), (1, 1)]),
        3 => Ok([(1, -1), (-1, 1)]),
        4 => Ok([(-2, -1), (2, 1)]),
        5 => Ok([(-2, 1), (2, -1)]),
        6 => Ok([(2, 0), (-2, 0)]),
        _ => Err(CcsoError::Params),
    }
}

const fn plane_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CcsoError {
    #[error("CCSO geometry is inconsistent")]
    Geometry,
    #[error("CCSO parameters are inconsistent")]
    Params,
    #[error("CCSO workspace access failed")]
    Workspace,
    #[error("CCSO sample conversion failed")]
    Sample,
}

#[cfg(test)]
#[path = "ccso_tests.rs"]
mod tests;
