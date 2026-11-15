// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Current-frame workspace DC intra prediction helpers.

use super::{CurrentFrameIntraEdges, CurrentFrameWorkspace, block_rect};
use crate::{IntraRectBlockSize, PlaneId, ReconSample, Result};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Extracts left and above in-storage edges for a rectangular block.
    ///
    /// Reads only adjacent in-storage edges; AV2 edge availability remains
    /// caller-owned.
    ///
    /// # Errors
    /// Returns [`crate::ReconError`] when the plane is absent or the target rectangle is
    /// out of bounds.
    pub fn intra_dc_edges_for_rect(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        let mut edges = CurrentFrameIntraEdges::default();
        self.intra_dc_edges_for_rect_into(plane, x, y, size, &mut edges)?;
        Ok(edges)
    }

    /// [`Self::intra_dc_edges_for_rect`] into caller storage, which avoids
    /// moving the sample arrays after they are written.
    ///
    /// # Errors
    /// As [`Self::intra_dc_edges_for_rect`].
    pub fn intra_dc_edges_for_rect_into(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        edges: &mut CurrentFrameIntraEdges<T>,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        self.plane(plane)?.dc_edges_for_rect_into(rect, edges)
    }
}
