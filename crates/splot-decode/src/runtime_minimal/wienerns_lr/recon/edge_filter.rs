// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! §7.13.2.7 intra edge-filter / corner-filter resolution for the full-recon sink.
//!
//! Splits the cohesive §7.13.2.7 step-1 filter assembly out of the parent
//! reconstruction bridge: it resolves the §7.13.2.17 strength, the §7.13.2.14
//! corner blend, and the §7.13.2.18 sweep span into a [`OneSidedEdgeFilter`] /
//! [`TwoSidedMiddleEdgeFilters`] for the directional intra primitives, or DEFERS
//! when a required opposite-corner sample is uncovered.

use splot_recon::{PlaneId, ReconSample};

use super::super::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use super::full_recon::{EdgeOrientation, OneSidedEdgeSpec};
use super::{
    OneSidedEdgeFilter, TwoSidedMiddleEdgeFilters, WienerNsLrReconSink, intra_edge_filter_strength,
    luma_sample_origin,
};
use crate::Result;
use crate::tile_payload::IntraYMode;
use splot_core::span::ByteOffset;

impl<T: ReconSample> WienerNsLrReconSink<T> {
    /// Resolves the §7.13.2.7 step-1 filters for BOTH edges of a zone-2 leaf into a
    /// [`TwoSidedMiddleEdgeFilters`], or `None` to DEFER. For zone-2 `applyIbp == 0`
    /// (IBP only fires for `p_angle < 90 || p_angle > 180`), so `filterType =
    /// is_smooth(above) | is_smooth(left)` seeds BOTH edges, `angleAbove = p_angle -
    /// 90`, `angleLeft = p_angle - 180`, and `need_right == need_bottom == false` (no
    /// far span in the filter). The §7.13.2.14 corner fires when `(w + h) >= 24`.
    pub(super) fn resolve_two_sided_middle_edge_filters(
        &self,
        mi_col: usize,
        mi_row: usize,
        w: u32,
        h: u32,
        p_angle: i32,
        tile_offset: ByteOffset,
    ) -> Result<Option<TwoSidedMiddleEdgeFilters>> {
        if !self.enable_intra_edge_filter {
            return Ok(Some(TwoSidedMiddleEdgeFilters {
                above: OneSidedEdgeFilter::default(),
                left: OneSidedEdgeFilter::default(),
            }));
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let smooth = |mode: Option<IntraYMode>| mode.is_some_and(IntraYMode::is_smooth);
        let above_smooth = smooth(
            mi_row
                .checked_sub(1)
                .and_then(|r| coverage.y_mode_at(mi_col, r)),
        );
        let left_smooth = smooth(
            mi_col
                .checked_sub(1)
                .and_then(|c| coverage.y_mode_at(c, mi_row)),
        );
        let filter_type = above_smooth || left_smooth;
        let corner_applies = (w + h) >= 24;
        let above_spec = OneSidedEdgeSpec {
            orientation: EdgeOrientation::Above,
            filter_type,
            angle_delta: p_angle - 90,
            need_far: false,
        };
        let left_spec = OneSidedEdgeSpec {
            orientation: EdgeOrientation::Left,
            filter_type,
            angle_delta: p_angle - 180,
            need_far: false,
        };
        let Some(above) = self.assemble_one_sided_edge_filter(
            above_spec,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )?
        else {
            return Ok(None);
        };
        let Some(left) = self.assemble_one_sided_edge_filter(
            left_spec,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(TwoSidedMiddleEdgeFilters { above, left }))
    }

    /// Resolves the §7.13.2.7 step-1 edge-filter / corner-filter inputs for a
    /// one-sided IDIF leaf into an [`OneSidedEdgeFilter`], or `None` to DEFER.
    ///
    /// When `enable_intra_edge_filter == 0` the §7.13.2.7 step is entirely skipped,
    /// so the default no-op filter is returned (the raw §7.13.2.1 edge feeds the
    /// §7.13.2.8 prediction unchanged). Otherwise the per-edge §7.13.2.17 strength
    /// is derived from the REAL §7.13.2.15/16 `is_smooth` neighbour modes recorded
    /// in the coverage map:
    /// * `applyIbp == 1`: `filterTypeAbove = is_smooth(above)`, `filterTypeLeft =
    ///   is_smooth(left)` (the per-edge pick), with the apply-IBP `angleAbove`/
    ///   `angleLeft` ±180 wrap and the `needRight`/`needBottom` ORs;
    /// * `applyIbp == 0`: `filterType = is_smooth(above) | is_smooth(left)` seeded
    ///   into both edges.
    ///
    /// An off-grid neighbour contributes `is_smooth == 0` (matching AVM's `ab ?
    /// is_smooth : 0`). The §7.13.2.14 corner filter fires when `needAbove &&
    /// needLeft && (w + h) >= 24`; its `corner_opposite` is the reconstructed
    /// OPPOSITE-edge `[0]` sample (`LeftCol[0]` zone-1 / `AboveRow[0]` zone-3), which
    /// MUST be covered — DEFER when it is off-grid or uncovered (the corner would
    /// read a fill value). `numPx` clamps the read span to the plane storage
    /// (`Min(w, maxX - x + 1)` / `Min(h, maxY - y + 1)`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_one_sided_edge_filter(
        &self,
        mi_col: usize,
        mi_row: usize,
        w: u32,
        h: u32,
        p_angle: i32,
        apply_ibp: bool,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        if !self.enable_intra_edge_filter {
            return Ok(Some(OneSidedEdgeFilter::default()));
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let smooth = |mode: Option<IntraYMode>| mode.is_some_and(IntraYMode::is_smooth);
        let above_smooth = smooth(
            mi_row
                .checked_sub(1)
                .and_then(|r| coverage.y_mode_at(mi_col, r)),
        );
        let left_smooth = smooth(
            mi_col
                .checked_sub(1)
                .and_then(|c| coverage.y_mode_at(c, mi_row)),
        );
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
        let corner_applies = need_above && need_left && (w + h) >= 24;
        let read_edge = if zone1 {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Above,
                filter_type: filter_type_above,
                angle_delta: angle_above,
                need_far: need_right,
            }
        } else {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Left,
                filter_type: filter_type_left,
                angle_delta: angle_left,
                need_far: need_bottom,
            }
        };
        self.assemble_one_sided_edge_filter(
            read_edge,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )
    }

    /// Resolves the §7.13.2.7 step-1 filter for the IBP SECONDARY (opposite) edge of
    /// a `useIBP` one-sided leaf: a zone-1 leaf (primary reads above) blends with a
    /// secondary §7.13.2.8 prediction at `secondAngle = pAngle + 180` reading the
    /// LEFT edge, so the left edge must be filtered with `filterTypeLeft` / `angleLeft`
    /// / `needBottom` — and symmetrically for zone-3. Mirrors the per-edge AVM filter
    /// (`av2_build_intra_predictors_high`, the `apply_ibp` branch filtering BOTH
    /// edges) so the secondary predictor reads the same filtered opposite column AVM
    /// does. Returns `None` (defer) when the corner's opposite sample is uncovered.
    pub(super) fn resolve_ibp_secondary_edge_filter(
        &self,
        mi_col: usize,
        mi_row: usize,
        w: u32,
        h: u32,
        p_angle: i32,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        if !self.enable_intra_edge_filter {
            return Ok(Some(OneSidedEdgeFilter::default()));
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let smooth = |mode: Option<IntraYMode>| mode.is_some_and(IntraYMode::is_smooth);
        let above_smooth = smooth(
            mi_row
                .checked_sub(1)
                .and_then(|r| coverage.y_mode_at(mi_col, r)),
        );
        let left_smooth = smooth(
            mi_col
                .checked_sub(1)
                .and_then(|c| coverage.y_mode_at(c, mi_row)),
        );
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
        let corner_applies = (w + h) >= 24;
        let secondary_edge = if zone1 {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Left,
                filter_type: left_smooth,
                angle_delta: angle_left,
                need_far: need_bottom,
            }
        } else {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Above,
                filter_type: above_smooth,
                angle_delta: angle_above,
                need_far: need_right,
            }
        };
        self.assemble_one_sided_edge_filter(
            secondary_edge,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )
    }

    /// Assembles a [`OneSidedEdgeFilter`] for one edge (above or left) from its
    /// resolved §7.13.2.7 spec: the §7.13.2.17 strength, the §7.13.2.7 `numPx`
    /// storage clamp, and the §7.13.2.14 corner's opposite-edge `[0]` sample read
    /// diagonally — an above edge reads `LeftCol[0] = CurrFrame[y][x-1]`, a left
    /// edge reads `AboveRow[0] = CurrFrame[y-1][x]`.
    /// Shared by the read-edge ([`Self::resolve_one_sided_edge_filter`]) and the
    /// IBP secondary-edge ([`Self::resolve_ibp_secondary_edge_filter`]) resolution.
    /// Returns `None` (defer) when the corner fires but its opposite sample is
    /// off-grid or uncovered. In full-recon mode the "uncovered by this sink" half of
    /// that gate is dropped (a genuinely off-frame corner still defers): the
    /// decode-order-prior opposite sample is always written, so only the gated path's
    /// conservative coverage check is skipped.
    #[allow(clippy::too_many_arguments)]
    fn assemble_one_sided_edge_filter(
        &self,
        edge: OneSidedEdgeSpec,
        corner_applies: bool,
        w: u32,
        h: u32,
        mi_col: usize,
        mi_row: usize,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let (strength_a, strength_b, primary, secondary) = match edge.orientation {
            EdgeOrientation::Above => (w, h, w, h),
            EdgeOrientation::Left => (h, w, h, w),
        };
        let strength = intra_edge_filter_strength(
            strength_a,
            strength_b,
            u8::from(edge.filter_type),
            edge.angle_delta,
        );
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        let plane = self.workspace.plane(PlaneId::Y)?;
        let storage = plane.storage_size();
        let (origin, max_axis) = match edge.orientation {
            EdgeOrientation::Above => (x, storage.width()),
            EdgeOrientation::Left => (y, storage.height()),
        };
        let in_block = (max_axis.saturating_sub(origin)).min(primary as usize);
        let num_px = in_block
            .checked_add(if edge.need_far { secondary as usize } else { 0 })
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_one_sided_edge_filter_numpx_overflow",
                )
            })?;
        let corner_opposite = if corner_applies {
            let (opp_col, opp_row, sample_x, sample_y) = match edge.orientation {
                EdgeOrientation::Above => (
                    mi_col.checked_sub(1),
                    Some(mi_row),
                    x.checked_sub(1),
                    Some(y),
                ),
                EdgeOrientation::Left => (
                    Some(mi_col),
                    mi_row.checked_sub(1),
                    Some(x),
                    y.checked_sub(1),
                ),
            };
            let (Some(opp_col), Some(opp_row)) = (opp_col, opp_row) else {
                return Ok(None);
            };
            if coverage.off_grid(opp_col, opp_row)
                || (!self.full_recon && !coverage.is_covered(opp_col, opp_row))
            {
                return Ok(None);
            }
            let (Some(sx), Some(sy)) = (sample_x, sample_y) else {
                return Ok(None);
            };
            Some(
                self.workspace
                    .reconstructed_sample(PlaneId::Y, sx, sy)?
                    .to_u16(),
            )
        } else {
            None
        };
        Ok(Some(OneSidedEdgeFilter {
            strength,
            num_px,
            corner_opposite,
        }))
    }
}
