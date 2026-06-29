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
    IntraRectBlockSize, IntraSquareBlockSize, PlaneId, PlaneRect, ReconError, ReconSample, Result,
};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Extracts left and above in-storage edges for a square block.
    ///
    /// The helper only reads edges adjacent to the requested plane-local square
    /// when they are inside workspace storage. It does not decide AV2 block,
    /// tile, superblock, or palette/CfL availability semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target square is out
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
    /// The helper only reads edges adjacent to the requested plane-local
    /// rectangle when they are inside workspace storage. It does not decide AV2
    /// block, tile, superblock, subsampled-DC, palette, or CfL availability
    /// semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle is
    /// out of bounds, or edge scratch allocation fails.
    pub fn intra_dc_edges_for_rect(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        let rect = block_rect(x, y, size)?;
        self.plane(plane)?.intra_dc_edges_for_rect(rect)
    }

    /// Predicts square DC intra samples into the workspace.
    ///
    /// This is a convenience wrapper over [`Self::predict_intra_dc_rect`]. Edge
    /// extraction is limited to in-storage left/above neighbors and does not
    /// model AV2 availability.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
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
    /// This computes the constant DC sample from in-storage left/above neighbor
    /// sums and fills the target rectangle. Edge extraction is limited to
    /// in-storage neighbors and does not model AV2 availability.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
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
    /// This uses the AV2 §7.13.2.11 sampled-sum process over in-storage left
    /// and/or above neighbors when those neighbors exist. If neither edge is in
    /// storage, the helper writes the AV2 midpoint. The helper does not
    /// synthesize partial missing-edge fallback samples or decide §7.13.2.1
    /// dispatch, tile-boundary, MRL, `UV_CFL_PRED`, transform, or residual
    /// semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
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
    /// This helper first writes AV2 §7.13.2.10 rectangular DC prediction and
    /// then applies the AV2 §7.13.2.12 IBP DC modifier using only in-storage
    /// left and/or above neighbors that already exist. It does not synthesize
    /// §7.13.2.1 fallback samples or decide AV2 `enable_ibp`, `useDip`, mode,
    /// tile-boundary, transform, residual, runtime output, or reference-refresh
    /// semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
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
    fn intra_dc_edges_for_rect(&self, rect: PlaneRect) -> Result<CurrentFrameIntraEdges<T>> {
        let nominal = rect;
        let rect = self.clamp_rect_to_storage(rect)?;

        let left = if nominal.x() == 0 {
            None
        } else {
            let mut left = Vec::new();
            left.try_reserve_exact(nominal.height()).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane: self.plane,
                    context: "left intra edge",
                }
            })?;
            for row in rect.y()..rect.y() + rect.height() {
                let index = self.sample_index(rect.x() - 1, row)?;
                left.push(self.samples[index]);
            }
            extend_edge_to_nominal(&mut left, nominal.height());
            Some(left)
        };

        let above = if nominal.y() == 0 {
            None
        } else {
            let row = rect.y() - 1;
            let range = self.row_range(row, rect.x(), rect.width())?;
            let mut above = Vec::new();
            above.try_reserve_exact(nominal.width()).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane: self.plane,
                    context: "above intra edge",
                }
            })?;
            // splot-copy-ok: materialize bounded above-edge scratch (block-width) for intra prediction
            above.extend_from_slice(&self.samples[range]);
            extend_edge_to_nominal(&mut above, nominal.width());
            Some(above)
        };

        Ok(CurrentFrameIntraEdges::new(left, above))
    }

    fn intra_dc_edge_sums_for_rect(&self, rect: PlaneRect) -> Result<(Option<u64>, Option<u64>)> {
        self.ensure_rect(rect)?;

        let left = if rect.x() == 0 {
            None
        } else {
            let mut sum = 0u64;
            for row in rect.y()..rect.y() + rect.height() {
                let index = self.sample_index(rect.x() - 1, row)?;
                sum += u64::from(self.samples[index].to_u16());
            }
            Some(sum)
        };

        let above = if rect.y() == 0 {
            None
        } else {
            let row = rect.y() - 1;
            let range = self.row_range(row, rect.x(), rect.width())?;
            Some(
                self.samples[range]
                    .iter()
                    .map(|sample| u64::from(sample.to_u16()))
                    .sum(),
            )
        };

        Ok((left, above))
    }

    fn intra_dc_subsampled_edge_sums_for_rect(
        &self,
        rect: PlaneRect,
    ) -> Result<(Option<DcEdgeSum>, Option<DcEdgeSum>)> {
        self.ensure_rect(rect)?;

        let left = if rect.x() == 0 {
            None
        } else {
            let mut sum = 0u64;
            let mut count = 0u64;
            for edge_index in (0..rect.height()).step_by(subsampled_step(rect.height())) {
                let row = rect.y() + edge_index;
                let index = self.sample_index(rect.x() - 1, row)?;
                sum = sum
                    .checked_add(u64::from(self.samples[index].to_u16()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace subsampled intra DC left sample sum",
                    })?;
                count = count.checked_add(1).ok_or(ReconError::ArithmeticOverflow {
                    context: "workspace subsampled intra DC left sample count",
                })?;
            }
            Some(DcEdgeSum { sum, count })
        };

        let above = if rect.y() == 0 {
            None
        } else {
            let row = rect.y() - 1;
            let range = self.row_range(row, rect.x(), rect.width())?;
            let mut sum = 0u64;
            let mut count = 0u64;
            for edge_index in (0..rect.width()).step_by(subsampled_step(rect.width())) {
                sum = sum
                    .checked_add(u64::from(self.samples[range.start + edge_index].to_u16()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace subsampled intra DC above sample sum",
                    })?;
                count = count.checked_add(1).ok_or(ReconError::ArithmeticOverflow {
                    context: "workspace subsampled intra DC above sample count",
                })?;
            }
            Some(DcEdgeSum { sum, count })
        };

        Ok((left, above))
    }

    fn predict_intra_ibp_dc_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        bit_depth: crate::BitDepth,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        let edges = self.intra_dc_edges_for_rect(rect)?;
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

/// Edge-extends an in-frame edge to the block's full nominal length by replicating
/// the LAST in-frame sample.
///
/// A transform overhanging the frame bottom (left column) or frame right (above
/// row) reads fewer in-frame samples than the block's nominal height/width. AVM
/// fills the out-of-frame tail with the last in-frame sample
/// (`av2/common/reconintra.c:1191-1195`, `avm_memset16(&edge[i], edge[i-1], …)`),
/// so the prediction primitives receive a full-length edge. The clamped origin is
/// always in-frame, so `edge` holds at least one sample whenever `nominal_len > 0`;
/// an empty `edge` (nothing to replicate) is left unchanged.
fn extend_edge_to_nominal<T: ReconSample>(edge: &mut Vec<T>, nominal_len: usize) {
    let Some(&last) = edge.last() else {
        return;
    };
    while edge.len() < nominal_len {
        edge.push(last);
    }
}
