// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Current-frame workspace DC intra prediction helpers.

use super::{CurrentFrameIntraEdges, CurrentFramePlane, CurrentFrameWorkspace, block_rect};
use crate::intra::predict_intra_dc_rect_into;
use crate::intra_dc_math::{DcEdgeSum, predict_intra_dc_rect_value_from_sums};
use crate::intra_dc_subsampled::{
    predict_intra_dc_subsampled_rect_value_from_sums, subsampled_step,
};
use crate::intra_ibp_dc::apply_intra_ibp_dc_rect;
use crate::{
    IntraDcEdge, IntraRectBlockSize, IntraSquareBlockSize, PlaneId, PlaneRect, ReconSample, Result,
};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Extracts left and above in-storage edges for a square block.
    ///
    /// Reads only adjacent in-storage edges; AV2 edge availability remains
    /// caller-owned.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] when the plane is absent, the target square is out
    /// of bounds, or edge scratch allocation fails.
    pub fn intra_dc_edges_for_square(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        self.intra_dc_edges_for_rect(plane, x, y, size.into())
    }

    /// Extracts left and above in-storage edges for a rectangular block.
    ///
    /// Reads only adjacent in-storage edges; AV2 edge availability remains
    /// caller-owned.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] when the plane is absent, the target rectangle is
    /// out of bounds, or edge scratch allocation fails.
    pub fn intra_dc_edges_for_rect(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        let rect = block_rect(x, y, size)?;
        self.plane(plane)?.dc_edges_for_rect(rect)
    }

    /// Predicts square DC intra samples into the workspace.
    ///
    /// Convenience wrapper over [`Self::predict_intra_dc_rect`].
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] for invalid target geometry, absent planes,
    /// or invalid prediction inputs.
    pub fn predict_intra_dc_square(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
    ) -> Result<()> {
        self.predict_intra_dc_rect(plane, x, y, size.into())
    }

    /// Predicts rectangular DC intra samples into the workspace.
    ///
    /// Computes the constant sample from adjacent in-storage edge sums.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] for invalid target geometry, absent planes,
    /// or invalid prediction inputs.
    pub fn predict_intra_dc_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        let (left_sum, above_sum) = self.plane(plane)?.intra_dc_edge_sums_for_rect(rect)?;
        let sample = predict_intra_dc_rect_value_from_sums(bit_depth, size, left_sum, above_sum)?;

        self.plane_mut(plane)?.fill_rect(rect, sample)
    }

    /// Predicts rectangular subsampled DC intra samples into the workspace.
    ///
    /// Applies AV2 §7.13.2.11 sampled sums over adjacent in-storage edges. With
    /// no in-storage edge, it writes the midpoint sample.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] for invalid target geometry, absent planes,
    /// or invalid prediction inputs.
    pub fn predict_intra_dc_subsampled_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        let (left, above) = self
            .plane(plane)?
            .intra_dc_subsampled_edge_sums_for_rect(rect)?;
        let sample = predict_intra_dc_subsampled_rect_value_from_sums(bit_depth, left, above)?;

        self.plane_mut(plane)?.fill_rect(rect, sample)
    }

    /// Predicts rectangular IBP DC intra samples into the workspace.
    ///
    /// Writes AV2 §7.13.2.10 DC prediction, then applies the §7.13.2.12 IBP DC
    /// modifier using adjacent in-storage edges.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] for invalid target geometry, absent planes,
    /// invalid prediction inputs, or edge scratch allocation failure.
    pub fn predict_intra_ibp_dc_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_ibp_dc_rect(rect, size, bit_depth)
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn intra_dc_edge_sums_for_rect(&self, rect: PlaneRect) -> Result<(Option<u64>, Option<u64>)> {
        Ok((
            self.dc_edge_sum_for_rect(rect, IntraDcEdge::Left)?,
            self.dc_edge_sum_for_rect(rect, IntraDcEdge::Above)?,
        ))
    }

    fn intra_dc_subsampled_edge_sums_for_rect(
        &self,
        rect: PlaneRect,
    ) -> Result<(Option<DcEdgeSum>, Option<DcEdgeSum>)> {
        Ok((
            self.dc_edge_sampled_sum_for_rect(
                rect,
                IntraDcEdge::Left,
                subsampled_step(rect.height()),
            )?,
            self.dc_edge_sampled_sum_for_rect(
                rect,
                IntraDcEdge::Above,
                subsampled_step(rect.width()),
            )?,
        ))
    }

    fn predict_intra_ibp_dc_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        bit_depth: crate::BitDepth,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        let edges = self.dc_edges_for_rect(rect)?;
        let output_start = self.sample_index(rect.x(), rect.y())?;
        predict_intra_dc_rect_into(
            bit_depth,
            size,
            edges.as_dc_edges(),
            &mut self.samples[output_start..],
            self.stride_samples,
        )?;
        apply_intra_ibp_dc_rect(
            bit_depth,
            size,
            edges.as_dc_edges(),
            &mut self.samples[output_start..],
            self.stride_samples,
        )
    }
}
