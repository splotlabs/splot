// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Current-frame workspace directional-angle intra prediction helpers.

use super::{CurrentFramePlane, CurrentFrameWorkspace, block_rect};
use crate::{
    BitDepth, IntraDirectionalAngle, IntraDirectionalAngleEdge, IntraDirectionalAngleIdifEdges,
    IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraRectBlockSize, PlaneId, PlaneRect, ReconError,
    ReconSample, Result, predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_into,
};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Predicts one-sided directional-angle intra samples into the workspace.
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
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_directional_angle_rect(rect, size, angle, bit_depth)
    }

    /// Predicts middle directional-angle intra samples into the workspace.
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
        let edge_kind = angle.required_edge();
        let edge = self.directional_angle_edge_samples(
            rect,
            edge_kind,
            edge_len,
            angle.p_angle(),
            directional_edge_context(edge_kind),
        )?;
        if self.plane == PlaneId::Y {
            let corner = self.one_sided_directional_corner(rect, edge_kind)?;
            let idif_edge = extend_one_sided_idif_edge(corner, &edge)?;
            let edges = match edge_kind {
                IntraDirectionalAngleEdge::Above => {
                    IntraDirectionalAngleIdifEdges::above(&idif_edge)
                }
                IntraDirectionalAngleEdge::Left => IntraDirectionalAngleIdifEdges::left(&idif_edge),
            };
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                size,
                angle,
                edges,
                &mut self.samples[output_start..],
                self.stride_samples,
            )
        } else {
            let edges = super::workspace_edges::directional_angle_edges(edge_kind, &edge);
            predict_intra_directional_angle_rect_into(
                bit_depth,
                size,
                angle,
                edges,
                &mut self.samples[output_start..],
                self.stride_samples,
            )
        }
    }

    fn one_sided_directional_corner(
        &self,
        rect: PlaneRect,
        edge: IntraDirectionalAngleEdge,
    ) -> Result<T> {
        let (x, y) = match edge {
            IntraDirectionalAngleEdge::Above => (rect.x().saturating_sub(1), rect.y() - 1),
            IntraDirectionalAngleEdge::Left => (rect.x() - 1, rect.y().saturating_sub(1)),
        };
        self.reconstructed_sample(x, y)
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
        let left = self.middle_directional_angle_edge_samples(
            rect,
            IntraDirectionalAngleEdge::Left,
            angle.p_angle(),
            "middle directional angle left edge",
        )?;
        let above = self.middle_directional_angle_edge_samples(
            rect,
            IntraDirectionalAngleEdge::Above,
            angle.p_angle(),
            "middle directional angle above edge",
        )?;

        let output_start = self.sample_index(rect.x(), rect.y())?;
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
}

const fn directional_edge_context(edge: IntraDirectionalAngleEdge) -> &'static str {
    match edge {
        IntraDirectionalAngleEdge::Above => "directional angle above edge",
        IntraDirectionalAngleEdge::Left => "directional angle left edge",
    }
}

fn directional_angle_edge_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "workspace directional angle prepared edge length",
        })
}

fn extend_middle_idif_edges<T: ReconSample>(left: &[T], above: &[T]) -> Result<(Vec<T>, Vec<T>)> {
    Ok((extend_one_idif_edge(left)?, extend_one_idif_edge(above)?))
}

fn extend_one_sided_idif_edge<T: ReconSample>(corner: T, edge: &[T]) -> Result<Vec<T>> {
    let last = *edge.last().ok_or(ReconError::ArithmeticOverflow {
        context: "workspace one-sided directional angle IDIF edge sample",
    })?;
    let out_len = edge
        .len()
        .checked_add(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "workspace one-sided directional angle IDIF edge length",
        })?;
    let mut out = Vec::new();
    out.try_reserve_exact(out_len)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "workspace one-sided directional angle IDIF edge allocation",
        })?;
    out.push(corner);
    out.push(corner);
    // splot-copy-ok: build the IDIF edge extension in bounded scratch storage
    out.extend_from_slice(edge);
    out.push(last);
    out.push(last);
    Ok(out)
}

fn extend_one_idif_edge<T: ReconSample>(edge: &[T]) -> Result<Vec<T>> {
    let corner = *edge.first().ok_or(ReconError::ArithmeticOverflow {
        context: "workspace middle directional angle IDIF edge corner",
    })?;
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
    out.push(corner);
    // splot-copy-ok: build the IDIF edge extension in bounded scratch storage
    out.extend_from_slice(edge);
    out.push(last);
    out.push(last);
    Ok(out)
}

#[cfg(test)]
#[path = "workspace_intra_directional_angle_tests.rs"]
mod tests;
