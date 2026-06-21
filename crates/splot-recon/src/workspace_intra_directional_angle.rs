// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Current-frame workspace directional-angle intra prediction helpers.

use super::{CurrentFramePlane, CurrentFrameWorkspace, block_rect};
use crate::{
    BitDepth, IntraDirectionalAngle, IntraDirectionalAngleEdge, IntraDirectionalAngleEdges,
    IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraRectBlockSize, PlaneId, PlaneRect, ReconError,
    ReconSample, Result, predict_intra_directional_angle_rect_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_into,
};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Predicts one-sided directional-angle intra samples into the workspace.
    ///
    /// This chroma/no-IDIF helper uses fully available in-storage prepared
    /// edges for AV2 §7.13.2.8 pAngles `45`, `67`, and `203`. It rejects
    /// [`PlaneId::Y`] until luma IDIF is implemented. It does not synthesize
    /// AV2 §7.13.2.1 fallback samples or decide AV2 edge availability, MRL,
    /// angle-delta, wide-angle, directional-IBP, tile-boundary, or superblock
    /// semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
    /// missing in-storage prepared edges, invalid edge samples, or invalid
    /// prediction inputs.
    pub fn predict_intra_directional_angle_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        angle: IntraDirectionalAngle,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        reject_luma_directional_angle(plane, angle.p_angle(), rect)?;
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_directional_angle_rect(rect, size, angle, bit_depth)
    }

    /// Predicts middle directional-angle intra samples into the workspace.
    ///
    /// Uses fully available in-storage logical `AboveRow[-1..w)` and
    /// `LeftCol[-1..h)` edges for AV2 §7.13.2.8 pAngles `113`, `135`, and `157`.
    /// Slice index zero maps to the logical `-1` sample. Chroma planes use the
    /// `enableIdif == 0` bilinear branch; the luma plane ([`PlaneId::Y`]) uses
    /// the `enableIdif == 1` §7.13.2.8 IDIF 4-tap filter, building the wider
    /// `AboveRow[-2..=w+1]` / `LeftCol[-2..=h+1]` edges via the §7.13.2.8 middle
    /// edge extension (`Edge[-2] = Edge[-1]`, `Edge[side] = Edge[side+1] =
    /// Edge[side-1]`). It does not synthesize AV2 §7.13.2.1 fallback samples or
    /// decide AV2 edge availability, MRL, angle-delta, wide-angle,
    /// directional-IBP, tile-boundary, or superblock semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
    /// missing in-storage prepared edges, invalid edge samples, or invalid
    /// prediction inputs.
    pub fn predict_intra_middle_directional_angle_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        angle: IntraMiddleDirectionalAngle,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_middle_directional_angle_rect(rect, size, angle, bit_depth, plane)
    }
}

/// Rejects luma for the one-sided directional-angle path, which still has no
/// §7.13.2.8 IDIF coverage (the IDIF 4-tap is implemented only for the middle
/// pAngles `113`/`135`/`157`; the one-sided pAngles `45`/`67`/`203` remain a
/// separate later brick).
fn reject_luma_directional_angle(plane: PlaneId, p_angle: u16, rect: PlaneRect) -> Result<()> {
    if matches!(plane, PlaneId::Y) {
        Err(
            ReconError::WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
                plane,
                p_angle,
                rect,
            },
        )
    } else {
        Ok(())
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn predict_intra_directional_angle_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        angle: IntraDirectionalAngle,
        bit_depth: BitDepth,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        let edge_len = directional_angle_edge_len(size)?;

        let output_start = self.sample_index(rect.x(), rect.y())?;
        match angle.required_edge() {
            IntraDirectionalAngleEdge::Above => {
                let above = self.directional_angle_above_edge(rect, edge_len, angle.p_angle())?;
                predict_intra_directional_angle_rect_into(
                    bit_depth,
                    size,
                    angle,
                    IntraDirectionalAngleEdges::above(&above),
                    &mut self.samples[output_start..],
                    self.stride_samples,
                )
            }
            IntraDirectionalAngleEdge::Left => {
                let left = self.directional_angle_left_edge(rect, edge_len, angle.p_angle())?;
                predict_intra_directional_angle_rect_into(
                    bit_depth,
                    size,
                    angle,
                    IntraDirectionalAngleEdges::left(&left),
                    &mut self.samples[output_start..],
                    self.stride_samples,
                )
            }
        }
    }

    fn predict_intra_middle_directional_angle_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        angle: IntraMiddleDirectionalAngle,
        bit_depth: BitDepth,
        plane: PlaneId,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        let left = self.middle_directional_angle_left_edge(rect, angle.p_angle())?;
        let above = self.middle_directional_angle_above_edge(rect, angle.p_angle())?;

        let output_start = self.sample_index(rect.x(), rect.y())?;
        // §7.13.2.8 `enableIdif = plane == 0`: luma uses the IDIF 4-tap; chroma
        // uses the bilinear branch. For D135 (`shift == 0` everywhere) both reduce
        // to a sample copy, but the other middle angles (`shift != 0`) differ.
        if matches!(plane, PlaneId::Y) {
            let (left_idif, above_idif) = extend_middle_idif_edges(&left, &above)?;
            predict_intra_middle_directional_angle_rect_idif_into(
                bit_depth,
                size,
                angle,
                IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
                &mut self.samples[output_start..],
                self.stride_samples,
            )
        } else {
            predict_intra_middle_directional_angle_rect_into(
                bit_depth,
                size,
                angle,
                IntraMiddleDirectionalAngleEdges::both(&left, &above),
                &mut self.samples[output_start..],
                self.stride_samples,
            )
        }
    }

    fn directional_angle_above_edge(
        &self,
        rect: PlaneRect,
        len: usize,
        p_angle: u16,
    ) -> Result<Vec<T>> {
        if rect.y() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Above,
                rect,
            ));
        }
        let end_x = rect
            .x()
            .checked_add(len)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "workspace directional angle above edge span",
            })?;
        if end_x > self.storage_size.width() {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Above,
                rect,
            ));
        }

        let range = self.row_range(rect.y() - 1, rect.x(), len)?;
        let mut above = Vec::new();
        above
            .try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: self.plane,
                context: "directional angle above edge",
            })?;
        // splot-copy-ok: materialize bounded above-edge scratch for directional-angle prediction
        above.extend_from_slice(&self.samples[range]);
        Ok(above)
    }

    fn directional_angle_left_edge(
        &self,
        rect: PlaneRect,
        len: usize,
        p_angle: u16,
    ) -> Result<Vec<T>> {
        if rect.x() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Left,
                rect,
            ));
        }
        let end_y = rect
            .y()
            .checked_add(len)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "workspace directional angle left edge span",
            })?;
        if end_y > self.storage_size.height() {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Left,
                rect,
            ));
        }

        let mut left = Vec::new();
        left.try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: self.plane,
                context: "directional angle left edge",
            })?;
        for row in rect.y()..end_y {
            left.push(self.samples[self.sample_index(rect.x() - 1, row)?]);
        }
        Ok(left)
    }

    fn middle_directional_angle_above_edge(&self, rect: PlaneRect, p_angle: u16) -> Result<Vec<T>> {
        if rect.y() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Above,
                rect,
            ));
        }
        if rect.x() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Left,
                rect,
            ));
        }

        let len = rect
            .width()
            .checked_add(1)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "workspace middle directional angle above edge length",
            })?;
        let range = self.row_range(rect.y() - 1, rect.x() - 1, len)?;
        let mut above = Vec::new();
        above
            .try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: self.plane,
                context: "middle directional angle above edge",
            })?;
        // splot-copy-ok: materialize bounded logical AboveRow[-1..w) scratch
        above.extend_from_slice(&self.samples[range]);
        Ok(above)
    }

    fn middle_directional_angle_left_edge(&self, rect: PlaneRect, p_angle: u16) -> Result<Vec<T>> {
        if rect.x() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Left,
                rect,
            ));
        }
        if rect.y() == 0 {
            return Err(self.directional_angle_edge_unavailable(
                p_angle,
                IntraDirectionalAngleEdge::Above,
                rect,
            ));
        }

        let len = rect
            .height()
            .checked_add(1)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "workspace middle directional angle left edge length",
            })?;
        let end_y = rect
            .y()
            .checked_add(rect.height())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "workspace middle directional angle left edge span",
            })?;
        let mut left = Vec::new();
        left.try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: self.plane,
                context: "middle directional angle left edge",
            })?;
        for row in (rect.y() - 1)..end_y {
            left.push(self.samples[self.sample_index(rect.x() - 1, row)?]);
        }
        Ok(left)
    }

    const fn directional_angle_edge_unavailable(
        &self,
        p_angle: u16,
        edge: IntraDirectionalAngleEdge,
        rect: PlaneRect,
    ) -> ReconError {
        ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: self.plane,
            p_angle,
            edge,
            rect,
        }
    }
}

fn directional_angle_edge_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "workspace directional angle prepared edge length",
        })
}

/// Extends a logical `Edge[-1..side)` slice (length `side + 1`, index 0 = the
/// `-1` corner) to the IDIF logical range `Edge[-2..=side+1]` (length
/// `side + 4`, index 0 = logical `-2`) per AV2 §7.13.2.8: `Edge[minBase - 1] =
/// Edge[minBase]` (`Edge[-2] = Edge[-1]`) and, for the middle branch
/// (`90 < pAngle < 180`), `Edge[side] = Edge[side + 1] = Edge[side - 1]`.
fn extend_middle_idif_edges<T: ReconSample>(left: &[T], above: &[T]) -> Result<(Vec<T>, Vec<T>)> {
    Ok((extend_one_idif_edge(left)?, extend_one_idif_edge(above)?))
}

fn extend_one_idif_edge<T: ReconSample>(edge: &[T]) -> Result<Vec<T>> {
    // `edge` is `Edge[-1..side)` of length `side + 1`: index 0 = `-1`, index
    // `k + 1` = logical `k`. The minimum is `-1` (one sample), so `side >= 0`.
    let corner = *edge.first().ok_or(ReconError::ArithmeticOverflow {
        context: "workspace middle directional angle IDIF edge corner",
    })?;
    // Logical `side - 1` is the last in-block sample (the slice's last element)
    // when `side >= 1`; with `side == 0` the spec extension repeats the corner.
    let last = *edge.last().unwrap_or(&corner);
    let out_len = edge
        .len()
        .checked_add(3)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "workspace middle directional angle IDIF edge length",
        })?;
    let mut out = Vec::new();
    out.try_reserve_exact(out_len)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "workspace middle directional angle IDIF edge allocation",
        })?;
    // Logical `-2` = `Edge[-1]` (the corner).
    out.push(corner);
    // splot-copy-ok: prepend the §7.13.2.8 corner extension before the in-range edge
    out.extend_from_slice(edge);
    // Logical `side` and `side + 1` both repeat `Edge[side - 1]` (the last sample).
    out.push(last);
    out.push(last);
    Ok(out)
}

#[cfg(test)]
#[path = "workspace_intra_directional_angle_tests.rs"]
mod tests;
