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

use crate::bitstream::tile_payload::GeneralIntraResidualError;
use crate::pipeline::reconstruct::{OneSidedEdgeFilter, TwoSidedMiddleEdgeFilters};

/// Tile-wide per-MI `YModes`-smoothness grid (§7.13.2.16 `is_smooth`): a cell
/// is `true` when the block covering it coded a SMOOTH / SMOOTH_V / SMOOTH_H
/// luma mode. Inter blocks record `false` (their `YModes` entry is an inter
/// mode, 05:10655), matching AVM's raw-mode `is_smooth` check.
pub(crate) struct TileYSmoothGrid {
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<bool>,
}

impl TileYSmoothGrid {
    /// Builds an all-`false` grid, or `None` when the dimensions overflow.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let cells = mi_rows.checked_mul(mi_cols)?;
        Some(Self {
            mi_rows,
            mi_cols,
            cells: vec![false; cells],
        })
    }

    /// Records a decoded block's luma smoothness into every covered MI cell.
    pub(crate) fn record(&mut self, r: usize, c: usize, n4w: usize, n4h: usize, smooth: bool) {
        for row in r..(r + n4h).min(self.mi_rows) {
            for col in c..(c + n4w).min(self.mi_cols) {
                self.cells[row * self.mi_cols + col] = smooth;
            }
        }
    }

    /// §7.13.2.15/16 neighbour smoothness for the block at luma MI
    /// (`mi_col`, `mi_row`): the cell above the origin and the cell to its
    /// left. Off-grid neighbours contribute `false`.
    pub(crate) fn block_smoothness(&self, mi_col: usize, mi_row: usize) -> (bool, bool) {
        let (col, row) = (mi_col as isize, mi_row as isize);
        (self.at(col, row - 1), self.at(col - 1, row))
    }

    /// Reads the smoothness at (`col`, `row`); off-grid reads are `false`.
    fn at(&self, col: isize, row: isize) -> bool {
        if col < 0 || row < 0 {
            return false;
        }
        let (col, row) = (col as usize, row as usize);
        if col >= self.mi_cols || row >= self.mi_rows {
            return false;
        }
        self.cells[row * self.mi_cols + col]
    }
}

/// The per-block §7.13.2.7 inputs threaded through the residual pipeline.
/// The smoothness pair is resolved once at the coding block's MI origin —
/// §7.13.2.15/16 read `YModes[MiRow - 1][MiCol]` / `[MiRow][MiCol - 1]` of
/// the BLOCK, and every transform unit of the block reuses it.
#[derive(Clone, Copy)]
pub(crate) struct IntraEdgeCtx {
    /// §5.3 `enable_ibp`.
    pub(crate) enable_ibp: bool,
    /// §5.3 `enable_intra_edge_filter`.
    pub(crate) enable_intra_edge_filter: bool,
    /// §7.13.2.15 `is_smooth` of the block's above LUMA neighbour.
    pub(crate) above_smooth: bool,
    /// §7.13.2.16 `is_smooth` of the block's left LUMA neighbour.
    pub(crate) left_smooth: bool,
    /// §7.13.2.15 `is_smooth` of the block's above CHROMA neighbour (its
    /// reconstructed `UVMode` is SMOOTH / SMOOTH_V / SMOOTH_H), for the chroma
    /// §7.13.2.17 `filterType`. Off-grid neighbours contribute `false`.
    pub(crate) above_chroma_smooth: bool,
    /// §7.13.2.16 `is_smooth` of the block's left CHROMA neighbour.
    pub(crate) left_chroma_smooth: bool,
}

/// The pure §7.13.2.7 step-1 shape of one read edge: which edge, its
/// §7.13.2.17 strength inputs, the far-extension need, and whether the
/// §7.13.2.14 corner blend fires.
struct EdgeSpec {
    above: bool,
    filter_type: bool,
    angle_delta: i32,
    need_far: bool,
    corner_applies: bool,
}

/// §7.13.2.7 step-1 spec for the PRIMARY read edge of a one-sided leaf
/// (07:5619-5644): per-edge filter types under `applyIbp` (with the ±180
/// angle wraps and the widened far needs), the OR'd filter type otherwise.
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

/// §7.13.2.7 step-1 spec for the IBP SECONDARY (opposite) read edge of a
/// `useIBP` leaf: the zone-1 primary reads above, so the secondary reads the
/// LEFT edge with `filterTypeLeft` / `angleLeft` / `needBottom` — and
/// symmetrically for zone-3 (07:5628-5636).
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

/// Assembles a per-unit [`OneSidedEdgeFilter`] from an [`EdgeSpec`]: the
/// §7.13.2.17 strength, the §7.13.2.7 `numPx` storage clamp, and the
/// §7.13.2.14 `corner_opposite` sample. Availability is frame geometry: the
/// production walk has every prior block reconstructed, and at frame edges
/// the corner reads the same §7.13.2.1 fallback sample the edge builders use
/// (the above edge's `x == 0` corner falls back to the block column itself).
#[allow(clippy::too_many_arguments)]
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
        .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
    let corner_opposite = if spec.corner_applies && (w + h) >= 24 {
        let (sx, sy) = if spec.above {
            (x.checked_sub(1).unwrap_or(x), y)
        } else {
            (x, y.checked_sub(1).unwrap_or(y))
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

/// Which §7.13.2.7 read edge a per-unit filter resolution targets.
#[derive(Clone, Copy)]
pub(crate) enum UnitEdgeRole {
    /// The PRIMARY read edge of a one-sided leaf (07:5619-5644).
    Primary {
        /// §7.13.2.7 `applyIbp` (per-edge filter types + widened far needs).
        apply_ibp: bool,
    },
    /// The IBP SECONDARY (opposite) read edge of a `useIBP` leaf.
    IbpSecondary,
}

/// The unit's §7.13.2.1 edge availability: the §7.13.2.7 strength filter
/// only runs over an edge with real samples, and the §7.13.2.14 corner
/// blend needs both edges.
#[derive(Clone, Copy)]
pub(crate) struct UnitEdges {
    /// The unit has an above row to read.
    pub(crate) above: bool,
    /// The unit has a left column to read.
    pub(crate) left: bool,
}

impl UnitEdges {
    const fn read_edge_available(self, above: bool) -> bool {
        if above { self.above } else { self.left }
    }
}

/// Resolves a per-unit §7.13.2.7 edge filter for a one-sided luma unit, or
/// the no-op default when `enable_intra_edge_filter == 0` or the read edge
/// has no real samples.
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
    assemble_unit_edge_filter(workspace, PlaneId::Y, &spec, x, y, w, h)
}

/// Resolves the per-unit §7.13.2.7 filters for BOTH edges of a zone-2
/// (`90 < pAngle < 180`) luma unit, or the no-op defaults when
/// `enable_intra_edge_filter == 0`. `applyIbp` (`enable_ibp` and a
/// non-4x4 unit — not zone-gated, 07:5611-5613) keeps the PER-EDGE
/// §7.13.2.15/16 filter types; otherwise the OR'd `filterType` seeds both
/// edges. `angleAbove = pAngle - 90`, `angleLeft = pAngle - 180` (in-range
/// for zone-2 either way), no far spans, and the §7.13.2.14 corner fires
/// at `(w + h) >= 24` (07:5619-5644).
#[allow(clippy::too_many_arguments)]
pub(crate) fn unit_middle_edge_filters<T: ReconSample>(
    ctx: IntraEdgeCtx,
    workspace: &CurrentFrameWorkspace<T>,
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
        assemble_unit_edge_filter(workspace, PlaneId::Y, &above_spec, x, y, w, h)?
    } else {
        OneSidedEdgeFilter::default()
    };
    let left = if edges.left {
        assemble_unit_edge_filter(workspace, PlaneId::Y, &left_spec, x, y, w, h)?
    } else {
        OneSidedEdgeFilter::default()
    };
    Ok(TwoSidedMiddleEdgeFilters { above, left })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn grid_records_and_reads_cells() {
        let mut grid = TileYSmoothGrid::new(4, 4).unwrap();
        grid.record(1, 1, 2, 2, true);
        assert!(grid.at(1, 1));
        assert!(grid.at(2, 2));
        assert!(!grid.at(0, 0));
        assert!(!grid.at(-1, 1));
        assert!(!grid.at(1, 4));
    }

    #[test]
    fn one_sided_spec_ors_smoothness_without_ibp() {
        let spec = one_sided_read_edge_spec(false, true, 45, false);
        assert!(spec.above);
        assert!(spec.filter_type, "zone-1 without IBP ORs above|left");
        assert_eq!(spec.angle_delta, -45);
        assert!(spec.need_far);
        assert!(!spec.corner_applies, "one-sided zone-1 lacks needLeft");
    }

    #[test]
    fn one_sided_spec_keeps_per_edge_smoothness_under_ibp() {
        let spec = one_sided_read_edge_spec(false, true, 45, true);
        assert!(!spec.filter_type, "applyIbp keeps the per-edge above type");
        assert!(spec.corner_applies, "applyIbp forces needAbove && needLeft");
        let secondary = ibp_secondary_edge_spec(false, true, 45);
        assert!(!secondary.above);
        assert!(secondary.filter_type, "secondary reads the left type");
        assert_eq!(secondary.angle_delta, 45 - 180 + 180);
        assert!(secondary.need_far, "zone-1 secondary needs the bottom span");
    }

    #[test]
    fn zone3_secondary_reads_above_with_wrapped_angle() {
        let secondary = ibp_secondary_edge_spec(true, false, 203);
        assert!(secondary.above);
        assert!(secondary.filter_type);
        assert_eq!(secondary.angle_delta, 203 - 90 - 180);
        assert!(secondary.need_far, "pAngle > 180 needs the right span");
    }
}
