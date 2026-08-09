// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{Select, Simd, cmp::SimdOrd, cmp::SimdPartialOrd, num::SimdInt, num::SimdUint};

use splot_core::headers::frame::{CcsoPlaneParams, FrameHeaderCore, ccso_quant_step};
use splot_recon::{BitDepth, PlaneId, ReconSample};

const MI_SIZE: usize = 4;
#[cfg(test)]
use splot_recon::{CurrentFrameWorkspace, PlaneRect};

use super::{
    cdef::CdefFrame,
    source::{FramePlane, StripePlane},
};

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

pub(crate) struct CcsoFrameConfig {
    planes: [Option<CcsoPlaneConfig>; CCSO_PLANES],
}

struct CcsoPlaneConfig {
    sub_x: usize,
    sub_y: usize,
    blk_w: usize,
    blk_h: usize,
    ccso_luma_log2: u32,
    bo_only: bool,
    edge_clf: bool,
    quant_step: i32,
    max_edge_interval: usize,
    max_band: usize,
    sample_offsets: [(isize, isize); 2],
    max_sample: i32,
    band_shift: u8,
    offset_lut: Vec<i32>,
    offset_lut_simd: [[u8; 16]; 5],
}

pub(crate) fn prepare_ccso(
    core: &FrameHeaderCore,
    grid: &CcsoUnitGrid,
    bit_depth: BitDepth,
    subsampling: (usize, usize),
) -> Result<CcsoFrameConfig, CcsoError> {
    let mut planes = std::array::from_fn(|_| None);
    if !grid.active() {
        return Ok(CcsoFrameConfig { planes });
    }
    let Some(params) = core.ccso_params.as_ref() else {
        return Ok(CcsoFrameConfig { planes });
    };
    for (plane, prepared) in planes.iter_mut().enumerate() {
        if !grid.plane_enabled[plane] {
            continue;
        }
        let params = params.planes.get(plane).ok_or(CcsoError::Params)?;
        if !params.ccso_planes {
            continue;
        }
        *prepared = Some(prepare_ccso_plane(
            plane,
            params,
            grid,
            bit_depth,
            subsampling,
        )?);
    }
    Ok(CcsoFrameConfig { planes })
}

fn prepare_ccso_plane(
    plane: usize,
    params: &CcsoPlaneParams,
    grid: &CcsoUnitGrid,
    bit_depth: BitDepth,
    subsampling: (usize, usize),
) -> Result<CcsoPlaneConfig, CcsoError> {
    let (sub_x, sub_y) = if plane_id(plane) == PlaneId::Y {
        (0, 0)
    } else {
        subsampling
    };
    let ccso_luma_log2 = grid.ccso_luma_size_log2();
    let shift_x = ccso_luma_log2
        .checked_sub(u32::try_from(sub_x).map_err(|_| CcsoError::Geometry)?)
        .ok_or(CcsoError::Geometry)?;
    let shift_y = ccso_luma_log2
        .checked_sub(u32::try_from(sub_y).map_err(|_| CcsoError::Geometry)?)
        .ok_or(CcsoError::Geometry)?;
    let max_band_log2 = params.ccso_max_band_log2.ok_or(CcsoError::Params)?;
    let ext_filter = params.ccso_ext_filter.ok_or(CcsoError::Params)?;
    let bo_only = params.ccso_bo_only.ok_or(CcsoError::Params)?;
    let edge_clf = params.ccso_edge_clf.ok_or(CcsoError::Params)?;
    let scale_idx = params.ccso_scale_idx.ok_or(CcsoError::Params)?;
    let quant_idx = params.ccso_quant_idx.ok_or(CcsoError::Params)?;
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
    let offset_lut = ccso_offset_lut(params, expected_offsets)?;
    let mut offset_lut_simd = [[0u8; 16]; 5];
    if offset_lut.len() > offset_lut_simd.len() * 16 {
        return Err(CcsoError::Params);
    }
    for (index, &offset) in offset_lut.iter().enumerate() {
        let value = i8::try_from(offset).map_err(|_| CcsoError::Params)?;
        offset_lut_simd[index / 16][index % 16] = value as u8;
    }
    Ok(CcsoPlaneConfig {
        sub_x,
        sub_y,
        blk_w: 1usize.checked_shl(shift_x).ok_or(CcsoError::Geometry)?,
        blk_h: 1usize.checked_shl(shift_y).ok_or(CcsoError::Geometry)?,
        ccso_luma_log2,
        bo_only,
        edge_clf,
        quant_step: i32::from(ccso_quant_step(scale_idx, quant_idx)),
        max_edge_interval,
        max_band,
        sample_offsets: ccso_sample_offsets(ext_filter)?,
        max_sample: i32::from(bit_depth.max_sample()),
        band_shift: bit_depth
            .bits()
            .checked_sub(max_band_log2)
            .ok_or(CcsoError::Params)?,
        offset_lut,
        offset_lut_simd,
    })
}

pub(crate) fn ccso_stripe<T: ReconSample>(
    frame: &mut CdefFrame<'_, T>,
    grid: &CcsoUnitGrid,
    config: &CcsoFrameConfig,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    tile_starts: Option<(&[u32], &[u32])>,
) -> Result<(), CcsoError> {
    let luma = frame.deblocked(PlaneId::Y).ok_or(CcsoError::Workspace)?;
    let destinations = [
        Some(&mut frame.filtered_y),
        frame.filtered_u.as_mut(),
        frame.filtered_v.as_mut(),
    ];
    for (plane, (prepared, destination)) in config.planes.iter().zip(destinations).enumerate() {
        let Some(prepared) = prepared else {
            continue;
        };
        let destination = destination.ok_or(CcsoError::Workspace)?;
        ccso_apply(
            destination,
            luma,
            plane,
            prepared,
            grid,
            lossless_grid,
            tile_starts,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
    let pixel_format = workspace.info().pixel_format();
    let subsampling = (
        usize::from(pixel_format.subsampling_x()),
        usize::from(pixel_format.subsampling_y()),
    );
    let prepared = prepare_ccso_plane(plane, params, grid, bit_depth, subsampling)?;
    let source = FramePlane::new(workspace, plane_id).ok_or(CcsoError::Workspace)?;
    let width = source.width();
    let height = source.frame_height();
    let mut filtered = StripePlane::copy_from(source, 0, height).map_err(|error| match error {
        crate::filters::source::StripeCopyError::Allocation(_) => CcsoError::Allocation,
        crate::filters::source::StripeCopyError::Geometry => CcsoError::Workspace,
    })?;
    ccso_apply(
        &mut filtered,
        FramePlane::window(curr_luma, luma_width, luma_height, 0, luma_height)
            .ok_or(CcsoError::Workspace)?,
        plane,
        &prepared,
        grid,
        lossless_grid,
        None,
    )?;
    let samples = filtered
        .samples()
        .iter()
        .map(|&sample| T::try_from_u16(sample).map_err(|_| CcsoError::Workspace))
        .collect::<Result<Vec<_>, _>>()?;
    workspace
        .write_rect(
            plane_id,
            PlaneRect::new(0, 0, width, height).map_err(|_| CcsoError::Geometry)?,
            &samples,
            width,
        )
        .map_err(|_| CcsoError::Workspace)
}

#[allow(clippy::too_many_arguments)]
fn ccso_apply<L: ReconSample>(
    destination: &mut StripePlane,
    curr_luma: FramePlane<'_, L>,
    plane: usize,
    config: &CcsoPlaneConfig,
    grid: &CcsoUnitGrid,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    tile_starts: Option<(&[u32], &[u32])>,
) -> Result<(), CcsoError> {
    let destination_end_y = destination.end_y().ok_or(CcsoError::Geometry)?;
    let luma_len = (curr_luma.end_y() - curr_luma.origin_y())
        .saturating_sub(1)
        .checked_mul(curr_luma.stride())
        .and_then(|start| start.checked_add(curr_luma.width()))
        .ok_or(CcsoError::Geometry)?;
    if destination.width() == 0
        || destination_end_y > destination.frame_height()
        || curr_luma.width() == 0
        || curr_luma.end_y() > curr_luma.frame_height()
        || curr_luma.stride() < curr_luma.width()
        || curr_luma.samples().len() < luma_len
    {
        return Err(CcsoError::Geometry);
    }

    let timer = crate::timing::start();
    let plane_id = plane_id(plane);
    let first_unit_y = destination.origin_y() / config.blk_h * config.blk_h;
    let frame_max_luma_x = curr_luma.width() - 1;
    let frame_max_luma_y = curr_luma.frame_height().saturating_sub(1);
    let (tile_rows, tile_cols) = match tile_starts {
        Some((rows, cols)) => (Some(rows), Some(cols)),
        None => (None, None),
    };
    for y in (first_unit_y..destination_end_y).step_by(config.blk_h) {
        for x in (0..destination.width()).step_by(config.blk_w) {
            let (min_luma_y, max_luma_y) =
                luma_tile_clamp(tile_rows, (y << config.sub_y) / MI_SIZE, frame_max_luma_y);
            let (min_luma_x, max_luma_x) =
                luma_tile_clamp(tile_cols, (x << config.sub_x) / MI_SIZE, frame_max_luma_x);
            let unit_row = (y << config.sub_y) >> config.ccso_luma_log2;
            let unit_col = (x << config.sub_x) >> config.ccso_luma_log2;
            if grid.block_value(plane, unit_row, unit_col) == 0 {
                continue;
            }
            let y_start = destination.origin_y().max(y);
            let y_end = destination_end_y.min(y.saturating_add(config.blk_h));
            let x_end = destination.width().min(x.saturating_add(config.blk_w));
            for y3 in y_start..y_end {
                let y_luma = y3 << config.sub_y;
                let center_row =
                    clamped_luma_row(curr_luma, y_luma as isize, min_luma_y, max_luma_y)?;
                let offset_rows = if config.bo_only {
                    None
                } else {
                    Some((
                        clamped_luma_row(
                            curr_luma,
                            y_luma as isize + config.sample_offsets[0].1,
                            min_luma_y,
                            max_luma_y,
                        )?,
                        clamped_luma_row(
                            curr_luma,
                            y_luma as isize + config.sample_offsets[1].1,
                            min_luma_y,
                            max_luma_y,
                        )?,
                    ))
                };
                let plane_row = destination.row_mut(y3).ok_or(CcsoError::Workspace)?;
                let mut x3 = x;
                if lossless_grid.is_none() {
                    x3 = ccso_simd_row(
                        plane_row,
                        center_row,
                        offset_rows,
                        x,
                        x_end,
                        max_luma_x,
                        config,
                    );
                }
                for x3 in x3..x_end {
                    if lossless_grid.is_some_and(|grid| {
                        grid.plane_sample_lossless(plane_id, x3, y3, config.sub_x, config.sub_y)
                    }) {
                        continue;
                    }
                    let x_luma = x3 << config.sub_x;
                    let center = center_row[x_luma.clamp(min_luma_x, max_luma_x)].to_u16();
                    let band = usize::from(center >> config.band_shift);
                    let (cls0, cls1) = match offset_rows {
                        None => (0usize, 0usize),
                        Some((row0, row1)) => {
                            let sx0 = (x_luma as isize + config.sample_offsets[0].0)
                                .clamp(min_luma_x as isize, max_luma_x as isize)
                                as usize;
                            let sx1 = (x_luma as isize + config.sample_offsets[1].0)
                                .clamp(min_luma_x as isize, max_luma_x as isize)
                                as usize;
                            (
                                ccso_score(
                                    i32::from(row0[sx0].to_u16()) - i32::from(center),
                                    config.quant_step,
                                    config.edge_clf,
                                ),
                                ccso_score(
                                    i32::from(row1[sx1].to_u16()) - i32::from(center),
                                    config.quant_step,
                                    config.edge_clf,
                                ),
                            )
                        }
                    };
                    let offset = *config
                        .offset_lut
                        .get((cls0 * config.max_edge_interval + cls1) * config.max_band + band)
                        .ok_or(CcsoError::Params)?;
                    let sample = plane_row.get_mut(x3).ok_or(CcsoError::Workspace)?;
                    *sample = (i32::from(*sample) + offset).clamp(0, config.max_sample) as u16;
                }
            }
        }
    }
    crate::timing::accumulate(crate::timing::Phase::CcsoUnits, timer);
    Ok(())
}

fn ccso_simd_row<L: ReconSample>(
    destination: &mut [u16],
    center_row: &[L],
    offset_rows: Option<(&[L], &[L])>,
    x_start: usize,
    x_end: usize,
    max_luma_x: usize,
    config: &CcsoPlaneConfig,
) -> usize {
    const LANES: usize = 16;

    if config.sub_x != 0 || x_end > destination.len() {
        return x_start;
    }
    let Some(center_row) = L::u16_slice(center_row) else {
        return x_start;
    };
    if x_end > center_row.len() {
        return x_start;
    }
    let offset_rows = match offset_rows {
        None => None,
        Some((row0, row1)) => {
            let (Some(row0), Some(row1)) = (L::u16_slice(row0), L::u16_slice(row1)) else {
                return x_start;
            };
            let min_dx = config.sample_offsets[0].0.min(config.sample_offsets[1].0);
            let max_dx = config.sample_offsets[0].0.max(config.sample_offsets[1].0);
            if x_start as isize + min_dx < 0 || x_end as isize - 1 + max_dx > max_luma_x as isize {
                return x_start;
            }
            Some((row0, row1))
        }
    };
    let zero = Simd::<u32, LANES>::splat(0);
    let one = Simd::<u32, LANES>::splat(1);
    let two = Simd::<u32, LANES>::splat(2);
    let quant_step = Simd::<i32, LANES>::splat(config.quant_step);
    let classify = |diff: Simd<i32, LANES>| {
        let at_least_low = diff.simd_ge(-quant_step);
        if config.edge_clf {
            at_least_low.select(one, zero)
        } else {
            diff.simd_gt(quant_step)
                .select(two, at_least_low.select(one, zero))
        }
    };
    let mut x = x_start;
    while x + LANES <= x_end {
        let centers = Simd::<u16, LANES>::from_slice(&center_row[x..]);
        let (class0, class1) = match offset_rows {
            None => (zero, zero),
            Some((row0, row1)) => {
                let x0 =
                    usize::try_from(x as isize + config.sample_offsets[0].0).unwrap_or_default();
                let x1 =
                    usize::try_from(x as isize + config.sample_offsets[1].0).unwrap_or_default();
                let source0 = Simd::<u16, LANES>::from_slice(&row0[x0..]).cast::<i32>();
                let source1 = Simd::<u16, LANES>::from_slice(&row1[x1..]).cast::<i32>();
                let centers = centers.cast::<i32>();
                (classify(source0 - centers), classify(source1 - centers))
            }
        };
        let band = (centers >> u16::from(config.band_shift)).cast::<u32>();
        let lut_index = (class0 * Simd::splat(config.max_edge_interval as u32) + class1)
            * Simd::splat(config.max_band as u32)
            + band;
        let lut_index = lut_index.cast::<u8>();
        let mut offset = Simd::<u8, LANES>::splat(0);
        let chunks = config.offset_lut.len().div_ceil(LANES);
        for (chunk, values) in config.offset_lut_simd.iter().take(chunks).enumerate() {
            offset |= Simd::from_array(*values)
                .swizzle_dyn(lut_index - Simd::splat((chunk * LANES) as u8));
        }
        let offset = offset.cast::<i8>().cast::<i32>();
        let samples = (Simd::<u16, LANES>::from_slice(&destination[x..]).cast::<i32>() + offset)
            .simd_max(Simd::splat(0))
            .simd_min(Simd::splat(config.max_sample))
            .cast::<u16>();
        destination[x..x + LANES].copy_from_slice(&samples.to_array());
        x += LANES;
    }
    x
}

fn ccso_offset_lut(
    params: &CcsoPlaneParams,
    expected_offsets: usize,
) -> Result<Vec<i32>, CcsoError> {
    let scale = i32::from(params.ccso_scale_idx.ok_or(CcsoError::Params)?) + 1;
    let mut lut = Vec::new();
    lut.try_reserve_exact(expected_offsets)
        .map_err(|_| CcsoError::Allocation)?;
    for &offset_idx in &params.ccso_offset_idx {
        let base = CCSO_OFFSET
            .get(usize::from(offset_idx))
            .copied()
            .ok_or(CcsoError::Params)?;
        lut.push(base * scale);
    }
    Ok(lut)
}

fn clamped_luma_row<T: ReconSample>(
    curr_luma: FramePlane<'_, T>,
    y: isize,
    min_y: usize,
    max_y: usize,
) -> Result<&'_ [T], CcsoError> {
    let sy = y.clamp(min_y as isize, max_y as isize) as usize;
    curr_luma.row(sy).ok_or(CcsoError::Geometry)
}

/// Luma-sample clamp range for the tile containing `mi`.
///
/// § 7.16 keeps CCSO's luma taps inside the current tile when the frame sets
/// `disable_loopfilters_across_tiles`; without it the range is the picture.
fn luma_tile_clamp(starts: Option<&[u32]>, mi: usize, frame_max: usize) -> (usize, usize) {
    let (start_mi, end_mi) = super::cdef::tile_span(starts, mi, usize::MAX);
    if starts.is_none() {
        return (0, frame_max);
    }
    let min = (start_mi * MI_SIZE).min(frame_max);
    let max = (end_mi * MI_SIZE).saturating_sub(1).min(frame_max);
    (min, max.max(min))
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
    #[error("CCSO lookup-table storage could not be reserved")]
    Allocation,
}

#[cfg(test)]
#[path = "ccso_tests.rs"]
mod tests;
