// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! §7.13.2.7 intra edge-filter derivation for the per-unit residual pipeline.
//!
//! The full-recon sink resolves the same step-1 filters from its coverage
//! grids (`wienerns_lr/recon/edge_filter.rs`, DEFER-on-uncovered semantics);
//! this module serves the production per-unit path, where every prior block
//! is reconstructed, so availability comes from frame geometry and the
//! §7.13.2.15/16 neighbour smoothness comes from a tile-wide per-MI grid.

use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};
use std::ops::Range;

use crate::bitstream::tile_payload::GeneralIntraResidualError;
use crate::pipeline::reconstruct::{OneSidedEdgeFilter, TwoSidedMiddleEdgeFilters};
use crate::prediction::{TileGridConstructionError, tile_grid_dimensions};

#[derive(Default)]
pub(crate) struct TileSmoothGrid {
    origin_row: usize,
    origin_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<bool>,
}

pub(crate) type TileYSmoothGrid = TileSmoothGrid;

pub(crate) type TileChromaSmoothGrid = TileSmoothGrid;

impl TileSmoothGrid {
    /// A grid laid out for one tile, for tests that want a fresh one.
    #[cfg(test)]
    pub(crate) fn new_for_tile(
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
    ) -> Result<Self, TileGridConstructionError> {
        let mut grid = Self::default();
        grid.reset_for_tile(mi_rows, mi_cols)?;
        Ok(grid)
    }

    /// Lays this grid out for another tile, keeping its cells.
    ///
    /// The decoder holds one grid per plane group for the life of the stream,
    /// so a steady-state tile clears the cells the last one left rather than
    /// sizing new ones.
    pub(crate) fn reset_for_tile(
        &mut self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
    ) -> Result<(), TileGridConstructionError> {
        let (rows, cols, cell_count) = tile_grid_dimensions(&mi_rows, &mi_cols)?;
        self.cells.clear();
        self.cells
            .try_reserve_exact(cell_count)
            .map_err(|_| TileGridConstructionError::Allocation)?;
        self.cells.resize(cell_count, false);
        self.origin_row = mi_rows.start;
        self.origin_col = mi_cols.start;
        self.mi_rows = rows;
        self.mi_cols = cols;
        Ok(())
    }

    pub(crate) fn record(&mut self, r: usize, c: usize, n4w: usize, n4h: usize, smooth: bool) {
        let row_start = r.max(self.origin_row);
        let col_start = c.max(self.origin_col);
        let row_end = r
            .saturating_add(n4h)
            .min(self.origin_row.saturating_add(self.mi_rows));
        let col_end = c
            .saturating_add(n4w)
            .min(self.origin_col.saturating_add(self.mi_cols));
        for row in row_start..row_end {
            for col in col_start..col_end {
                self.cells[(row - self.origin_row) * self.mi_cols + col - self.origin_col] = smooth;
            }
        }
    }

    pub(crate) fn block_smoothness(&self, mi_col: usize, mi_row: usize) -> (bool, bool) {
        let (col, row) = (mi_col as isize, mi_row as isize);
        (self.at(col, row - 1), self.at(col - 1, row))
    }

    fn at(&self, col: isize, row: isize) -> bool {
        if col < 0 || row < 0 {
            return false;
        }
        let (col, row) = (col as usize, row as usize);
        let Some(col) = col.checked_sub(self.origin_col) else {
            return false;
        };
        let Some(row) = row.checked_sub(self.origin_row) else {
            return false;
        };
        if col >= self.mi_cols || row >= self.mi_rows {
            return false;
        }
        self.cells[row * self.mi_cols + col]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct IntraEdgeCtx {
    pub(crate) enable_ibp: bool,
    pub(crate) enable_intra_edge_filter: bool,
    pub(crate) above_smooth: bool,
    pub(crate) left_smooth: bool,
    pub(crate) chroma_above_smooth: bool,
    pub(crate) chroma_left_smooth: bool,
}

impl IntraEdgeCtx {
    pub(crate) const fn chroma(self) -> Self {
        Self {
            above_smooth: self.chroma_above_smooth,
            left_smooth: self.chroma_left_smooth,
            ..self
        }
    }
}

struct EdgeSpec {
    above: bool,
    filter_type: bool,
    angle_delta: i32,
    need_far: bool,
    corner_applies: bool,
}

fn one_sided_read_edge_spec(
    above_smooth: bool,
    left_smooth: bool,
    p_angle: i32,
    apply_ibp: bool,
) -> EdgeSpec {
    let zone1 = p_angle < 90;
    let (mut need_above, mut need_left) = if zone1 { (true, false) } else { (false, true) };
    let (mut filter_type_above, mut filter_type_left) = (above_smooth, left_smooth);
    let mut angle_above = p_angle - 90;
    let mut angle_left = p_angle - 180;
    let mut need_right = zone1;
    let mut need_bottom = !zone1;
    if apply_ibp {
        need_above = true;
        need_left = true;
        need_right |= p_angle > 180;
        need_bottom |= p_angle < 90;
        if angle_above > 90 {
            angle_above -= 180;
        }
        if angle_left < -90 {
            angle_left += 180;
        }
    } else {
        let filter_type = above_smooth || left_smooth;
        filter_type_above = filter_type;
        filter_type_left = filter_type;
    }
    let corner_gate = need_above && need_left;
    if zone1 {
        EdgeSpec {
            above: true,
            filter_type: filter_type_above,
            angle_delta: angle_above,
            need_far: need_right,
            corner_applies: corner_gate,
        }
    } else {
        EdgeSpec {
            above: false,
            filter_type: filter_type_left,
            angle_delta: angle_left,
            need_far: need_bottom,
            corner_applies: corner_gate,
        }
    }
}

fn ibp_secondary_edge_spec(above_smooth: bool, left_smooth: bool, p_angle: i32) -> EdgeSpec {
    let zone1 = p_angle < 90;
    let mut angle_above = p_angle - 90;
    let mut angle_left = p_angle - 180;
    if angle_above > 90 {
        angle_above -= 180;
    }
    if angle_left < -90 {
        angle_left += 180;
    }
    let need_right = zone1 || p_angle > 180;
    let need_bottom = !zone1 || p_angle < 90;
    if zone1 {
        EdgeSpec {
            above: false,
            filter_type: left_smooth,
            angle_delta: angle_left,
            need_far: need_bottom,
            corner_applies: true,
        }
    } else {
        EdgeSpec {
            above: true,
            filter_type: above_smooth,
            angle_delta: angle_above,
            need_far: need_right,
            corner_applies: true,
        }
    }
}

fn assemble_unit_edge_filter<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    spec: &EdgeSpec,
    x: usize,
    y: usize,
    w: u32,
    h: u32,
) -> core::result::Result<OneSidedEdgeFilter, GeneralIntraResidualError> {
    let (strength_a, strength_b, primary, secondary) = if spec.above {
        (w, h, w, h)
    } else {
        (h, w, h, w)
    };
    let strength = crate::filters::wienerns_lr::recon::intra_edge_filter_strength(
        strength_a,
        strength_b,
        u8::from(spec.filter_type),
        spec.angle_delta,
    );
    let storage = workspace.plane(plane_id)?.storage_size();
    let (origin, max_axis) = if spec.above {
        (x, storage.width())
    } else {
        (y, storage.height())
    };
    let in_block = (max_axis.saturating_sub(origin)).min(primary as usize);
    let num_px = in_block
        .checked_add(if spec.need_far { secondary as usize } else { 0 })
        .and_then(|v| v.checked_add(1))
        .ok_or(GeneralIntraResidualError::InvalidReconstructionState {
            context: "directional edge sample count",
        })?;
    let corner_opposite = if spec.corner_applies && (w + h) >= 24 {
        let (sx, sy) = if spec.above {
            (x.saturating_sub(1), y)
        } else {
            (x, y.saturating_sub(1))
        };
        Some(workspace.reconstructed_sample(plane_id, sx, sy)?.to_u16())
    } else {
        None
    };
    Ok(OneSidedEdgeFilter {
        strength,
        num_px,
        corner_opposite,
    })
}

#[derive(Clone, Copy)]
pub(crate) enum UnitEdgeRole {
    Primary { apply_ibp: bool },
    IbpSecondary,
}

#[derive(Clone, Copy)]
pub(crate) struct UnitEdges {
    pub(crate) above: bool,
    pub(crate) left: bool,
}

impl UnitEdges {
    const fn read_edge_available(self, above: bool) -> bool {
        if above { self.above } else { self.left }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn unit_edge_filter<T: ReconSample>(
    ctx: IntraEdgeCtx,
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: i32,
    role: UnitEdgeRole,
    edges: UnitEdges,
    x: usize,
    y: usize,
    w: u32,
    h: u32,
) -> core::result::Result<OneSidedEdgeFilter, GeneralIntraResidualError> {
    unit_edge_filter_for_plane(ctx, workspace, PlaneId::Y, p_angle, role, edges, x, y, w, h)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn unit_edge_filter_for_plane<T: ReconSample>(
    ctx: IntraEdgeCtx,
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    p_angle: i32,
    role: UnitEdgeRole,
    edges: UnitEdges,
    x: usize,
    y: usize,
    w: u32,
    h: u32,
) -> core::result::Result<OneSidedEdgeFilter, GeneralIntraResidualError> {
    if !ctx.enable_intra_edge_filter {
        return Ok(OneSidedEdgeFilter::default());
    }
    let (above_smooth, left_smooth) = (ctx.above_smooth, ctx.left_smooth);
    let mut spec = match role {
        UnitEdgeRole::Primary { apply_ibp } => {
            one_sided_read_edge_spec(above_smooth, left_smooth, p_angle, apply_ibp)
        }
        UnitEdgeRole::IbpSecondary => ibp_secondary_edge_spec(above_smooth, left_smooth, p_angle),
    };
    if !edges.read_edge_available(spec.above) {
        return Ok(OneSidedEdgeFilter::default());
    }
    spec.corner_applies = spec.corner_applies && edges.above && edges.left;
    assemble_unit_edge_filter(workspace, plane_id, &spec, x, y, w, h)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn unit_middle_edge_filters<T: ReconSample>(
    ctx: IntraEdgeCtx,
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    p_angle: i32,
    apply_ibp: bool,
    edges: UnitEdges,
    x: usize,
    y: usize,
    w: u32,
    h: u32,
) -> core::result::Result<TwoSidedMiddleEdgeFilters, GeneralIntraResidualError> {
    if !ctx.enable_intra_edge_filter {
        return Ok(TwoSidedMiddleEdgeFilters {
            above: OneSidedEdgeFilter::default(),
            left: OneSidedEdgeFilter::default(),
        });
    }
    let ored = ctx.above_smooth || ctx.left_smooth;
    let (filter_type_above, filter_type_left) = if apply_ibp {
        (ctx.above_smooth, ctx.left_smooth)
    } else {
        (ored, ored)
    };
    let corner_applies = edges.above && edges.left;
    let above_spec = EdgeSpec {
        above: true,
        filter_type: filter_type_above,
        angle_delta: p_angle - 90,
        need_far: false,
        corner_applies,
    };
    let left_spec = EdgeSpec {
        above: false,
        filter_type: filter_type_left,
        angle_delta: p_angle - 180,
        need_far: false,
        corner_applies,
    };
    let above = if edges.above {
        assemble_unit_edge_filter(workspace, plane_id, &above_spec, x, y, w, h)?
    } else {
        OneSidedEdgeFilter::default()
    };
    let left = if edges.left {
        assemble_unit_edge_filter(workspace, plane_id, &left_spec, x, y, w, h)?
    } else {
        OneSidedEdgeFilter::default()
    };
    Ok(TwoSidedMiddleEdgeFilters { above, left })
}

#[cfg(test)]
#[path = "intra_edge_tests.rs"]
mod tests;
