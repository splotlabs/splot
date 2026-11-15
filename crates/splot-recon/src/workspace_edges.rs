// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Edge and row-view helpers for current-frame workspaces.

use super::CurrentFramePlane;
use crate::{IntraDcEdge, IntraDcEdges, PlaneRect, ReconError, ReconSample, Result};

const CURRENT_FRAME_INTRA_EDGE_CAPACITY: usize = 64;

/// Owned edge samples read from a current-frame workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentFrameIntraEdges<T: ReconSample> {
    left: Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>,
    above: Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>,
}

impl<T: ReconSample> Default for CurrentFrameIntraEdges<T> {
    fn default() -> Self {
        Self {
            left: None,
            above: None,
        }
    }
}

impl<T: ReconSample> CurrentFrameIntraEdges<T> {
    /// Returns left edge samples when they were inside workspace storage.
    pub fn left_samples(&self) -> Option<&[T]> {
        self.left.as_ref().map(|(samples, len)| &samples[..*len])
    }

    /// Returns above edge samples when they were inside workspace storage.
    pub fn above_samples(&self) -> Option<&[T]> {
        self.above.as_ref().map(|(samples, len)| &samples[..*len])
    }

    /// Borrows the owned edges as DC prediction inputs.
    pub fn as_dc_edges(&self) -> IntraDcEdges<'_, T> {
        IntraDcEdges::new(self.left_samples(), self.above_samples())
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    /// The DC edges of `nominal` into caller storage, so the sample arrays
    /// are written where they are read.
    pub(super) fn dc_edges_for_rect_into(
        &self,
        nominal: PlaneRect,
        edges: &mut CurrentFrameIntraEdges<T>,
    ) -> Result<()> {
        edges.left = None;
        edges.above = None;
        let rect = self.clamp_rect_to_storage(nominal)?;
        self.dc_edge_samples(nominal, rect, IntraDcEdge::Left, &mut edges.left)?;
        self.dc_edge_samples(nominal, rect, IntraDcEdge::Above, &mut edges.above)
    }

    fn dc_edge_samples(
        &self,
        nominal: PlaneRect,
        rect: PlaneRect,
        edge: IntraDcEdge,
        slot: &mut Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>,
    ) -> Result<()> {
        if !dc_edge_available(nominal, edge) {
            return Ok(());
        }

        let nominal_len = dc_edge_len(nominal, edge);
        if nominal_len > CURRENT_FRAME_INTRA_EDGE_CAPACITY {
            return Err(ReconError::ArithmeticOverflow {
                context: "workspace intra edge inline capacity",
            });
        }

        let (samples, _) = slot.insert((
            [T::default(); CURRENT_FRAME_INTRA_EDGE_CAPACITY],
            nominal_len,
        ));
        let actual_len = match edge {
            IntraDcEdge::Left => {
                let column = self.left_column_start(rect)?;
                let stride = self.stride_samples();
                for (index, slot) in samples[..rect.height()].iter_mut().enumerate() {
                    *slot = self.samples[column + index * stride];
                }
                rect.height()
            }
            IntraDcEdge::Above => {
                let range = self.row_range(rect.y() - 1, rect.x(), rect.width())?;
                // splot-copy-ok: materialize bounded above-edge scratch for intra prediction
                samples[..rect.width()].copy_from_slice(&self.samples[range]);
                rect.width()
            }
        };
        if actual_len < nominal_len {
            let last = samples[actual_len - 1];
            samples[actual_len..nominal_len].fill(last);
        }
        Ok(())
    }

    /// Returns the storage index of sample `(rect.x() - 1, rect.y())`, after one
    /// check that all `rect.height()` rows of the left column are stored.
    fn left_column_start(&self, rect: PlaneRect) -> Result<usize> {
        let row = self.band_row(rect.y(), rect.height())?;
        Ok(row * self.stride_samples() + rect.x() - 1)
    }
}

const fn dc_edge_available(rect: PlaneRect, edge: IntraDcEdge) -> bool {
    match edge {
        IntraDcEdge::Left => rect.x() > 0,
        IntraDcEdge::Above => rect.y() > 0,
    }
}

const fn dc_edge_len(rect: PlaneRect, edge: IntraDcEdge) -> usize {
    match edge {
        IntraDcEdge::Left => rect.height(),
        IntraDcEdge::Above => rect.width(),
    }
}
