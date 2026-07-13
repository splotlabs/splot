// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Edge and row-view helpers for current-frame workspaces.

use core::ops::Range;

use super::CurrentFramePlane;
use crate::intra_dc_math::DcEdgeSum;
use crate::{
    IntraDcEdge, IntraDcEdges, IntraDirectionalAngleEdge, IntraDirectionalAngleEdges, PlaneRect,
    ReconError, ReconSample, Result,
};

const CURRENT_FRAME_INTRA_EDGE_CAPACITY: usize = 64;

/// Iterator over checked workspace rectangle rows.
#[derive(Clone, Debug)]
pub struct WorkspaceRectRows<'a, T: ReconSample> {
    plane: &'a CurrentFramePlane<T>,
    rect: PlaneRect,
    next_row: usize,
}

impl<'a, T: ReconSample> WorkspaceRectRows<'a, T> {
    pub(super) const fn new(plane: &'a CurrentFramePlane<T>, rect: PlaneRect) -> Self {
        Self {
            plane,
            rect,
            next_row: 0,
        }
    }
}

impl<'a, T: ReconSample> Iterator for WorkspaceRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.rect.height() {
            return None;
        }

        let row = self.rect.y() + self.next_row;
        let start = row * self.plane.stride_samples + self.rect.x();
        let end = start + self.rect.width();
        self.next_row += 1;
        Some(&self.plane.samples[start..end])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.rect.height() - self.next_row;
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for WorkspaceRectRows<'_, T> {}

/// Owned edge samples read from a current-frame workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentFrameIntraEdges<T: ReconSample> {
    left: Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>,
    above: Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>,
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
    pub(super) fn dc_edges_for_rect(
        &self,
        nominal: PlaneRect,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        let rect = self.clamp_rect_to_storage(nominal)?;
        Ok(CurrentFrameIntraEdges {
            left: self.dc_edge_samples(nominal, rect, IntraDcEdge::Left)?,
            above: self.dc_edge_samples(nominal, rect, IntraDcEdge::Above)?,
        })
    }

    pub(super) fn dc_edge_sum_for_rect(
        &self,
        rect: PlaneRect,
        edge: IntraDcEdge,
    ) -> Result<Option<u32>> {
        self.fold_dc_edge_samples(rect, edge, 1, 0_u32, |sum, _, sample| {
            sum.checked_add(u32::from(sample.to_u16()))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: dc_sum_context(edge),
                })
        })
    }

    pub(super) fn dc_edge_sampled_sum_for_rect(
        &self,
        rect: PlaneRect,
        edge: IntraDcEdge,
        step: usize,
    ) -> Result<Option<DcEdgeSum>> {
        self.fold_dc_edge_samples(
            rect,
            edge,
            step,
            DcEdgeSum { sum: 0, count: 0 },
            |sampled, _, sample| {
                let sum = sampled.sum.checked_add(u32::from(sample.to_u16())).ok_or(
                    ReconError::ArithmeticOverflow {
                        context: dc_sampled_sum_context(edge),
                    },
                )?;
                let count = sampled
                    .count
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: dc_sampled_count_context(edge),
                    })?;
                Ok(DcEdgeSum { sum, count })
            },
        )
    }

    pub(super) fn directional_angle_edge_samples(
        &self,
        rect: PlaneRect,
        edge: IntraDirectionalAngleEdge,
        len: usize,
        p_angle: u16,
        context: &'static str,
    ) -> Result<Vec<T>> {
        self.ensure_rect(rect)?;
        match edge {
            IntraDirectionalAngleEdge::Above => {
                if rect.y() == 0 {
                    return Err(self.directional_angle_edge_unavailable(p_angle, edge, rect));
                }
                let end_x = rect
                    .x()
                    .checked_add(len)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace directional angle above edge span",
                    })?;
                if end_x > self.storage_size.width() {
                    return Err(self.directional_angle_edge_unavailable(p_angle, edge, rect));
                }
                self.above_edge_samples(rect.y() - 1, rect.x(), len, len, context)
            }
            IntraDirectionalAngleEdge::Left => {
                if rect.x() == 0 {
                    return Err(self.directional_angle_edge_unavailable(p_angle, edge, rect));
                }
                let end_y = rect
                    .y()
                    .checked_add(len)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace directional angle left edge span",
                    })?;
                if end_y > self.storage_size.height() {
                    return Err(self.directional_angle_edge_unavailable(p_angle, edge, rect));
                }
                self.left_edge_samples(rect.x() - 1, rect.y()..end_y, len, context)
            }
        }
    }

    pub(super) fn middle_directional_angle_edge_samples(
        &self,
        rect: PlaneRect,
        edge: IntraDirectionalAngleEdge,
        p_angle: u16,
        context: &'static str,
    ) -> Result<Vec<T>> {
        self.ensure_rect(rect)?;
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

        match edge {
            IntraDirectionalAngleEdge::Above => {
                let len = rect
                    .width()
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace middle directional angle above edge length",
                    })?;
                self.above_edge_samples(rect.y() - 1, rect.x() - 1, len, len, context)
            }
            IntraDirectionalAngleEdge::Left => {
                let len = rect
                    .height()
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "workspace middle directional angle left edge length",
                    })?;
                let end_y =
                    rect.y()
                        .checked_add(rect.height())
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "workspace middle directional angle left edge span",
                        })?;
                self.left_edge_samples(rect.x() - 1, (rect.y() - 1)..end_y, len, context)
            }
        }
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

    fn dc_edge_samples(
        &self,
        nominal: PlaneRect,
        rect: PlaneRect,
        edge: IntraDcEdge,
    ) -> Result<Option<([T; CURRENT_FRAME_INTRA_EDGE_CAPACITY], usize)>> {
        if !dc_edge_available(nominal, edge) {
            return Ok(None);
        }

        let nominal_len = dc_edge_len(nominal, edge);
        if nominal_len > CURRENT_FRAME_INTRA_EDGE_CAPACITY {
            return Err(ReconError::ArithmeticOverflow {
                context: "workspace intra edge inline capacity",
            });
        }

        let mut samples = [T::default(); CURRENT_FRAME_INTRA_EDGE_CAPACITY];
        let actual_len = match edge {
            IntraDcEdge::Left => {
                for (index, row) in (rect.y()..rect.y() + rect.height()).enumerate() {
                    samples[index] = self.samples[self.sample_index(rect.x() - 1, row)?];
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
        Ok(Some((samples, nominal_len)))
    }

    fn fold_dc_edge_samples<A: Copy>(
        &self,
        rect: PlaneRect,
        edge: IntraDcEdge,
        step: usize,
        mut acc: A,
        mut fold: impl FnMut(A, usize, T) -> Result<A>,
    ) -> Result<Option<A>> {
        if step == 0 {
            return Err(ReconError::ArithmeticOverflow {
                context: "workspace intra DC edge sampling step",
            });
        }
        self.ensure_rect(rect)?;
        if !dc_edge_available(rect, edge) {
            return Ok(None);
        }

        let len = dc_edge_len(rect, edge);
        match edge {
            IntraDcEdge::Left => {
                for edge_index in (0..len).step_by(step) {
                    let row = rect.y() + edge_index;
                    let sample = self.samples[self.sample_index(rect.x() - 1, row)?];
                    acc = fold(acc, edge_index, sample)?;
                }
            }
            IntraDcEdge::Above => {
                let range = self.row_range(rect.y() - 1, rect.x(), len)?;
                for edge_index in (0..len).step_by(step) {
                    acc = fold(acc, edge_index, self.samples[range.start + edge_index])?;
                }
            }
        }
        Ok(Some(acc))
    }

    fn edge_scratch(&self, len: usize, context: &'static str) -> Result<Vec<T>> {
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: self.plane,
                context,
            })?;
        Ok(samples)
    }

    fn above_edge_samples(
        &self,
        row: usize,
        x: usize,
        len: usize,
        reserve_len: usize,
        context: &'static str,
    ) -> Result<Vec<T>> {
        let range = self.row_range(row, x, len)?;
        let mut samples = self.edge_scratch(reserve_len, context)?;
        // splot-copy-ok: materialize bounded above-edge scratch for intra prediction
        samples.extend_from_slice(&self.samples[range]);
        Ok(samples)
    }

    fn left_edge_samples(
        &self,
        x: usize,
        rows: Range<usize>,
        reserve_len: usize,
        context: &'static str,
    ) -> Result<Vec<T>> {
        let mut samples = self.edge_scratch(reserve_len, context)?;
        for row in rows {
            samples.push(self.samples[self.sample_index(x, row)?]);
        }
        Ok(samples)
    }
}

pub(super) const fn directional_angle_edges<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
) -> IntraDirectionalAngleEdges<'_, T> {
    match edge {
        IntraDirectionalAngleEdge::Above => IntraDirectionalAngleEdges::above(samples),
        IntraDirectionalAngleEdge::Left => IntraDirectionalAngleEdges::left(samples),
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

const fn dc_sum_context(edge: IntraDcEdge) -> &'static str {
    match edge {
        IntraDcEdge::Left => "workspace intra DC left sample sum",
        IntraDcEdge::Above => "workspace intra DC above sample sum",
    }
}

const fn dc_sampled_sum_context(edge: IntraDcEdge) -> &'static str {
    match edge {
        IntraDcEdge::Left => "workspace subsampled intra DC left sample sum",
        IntraDcEdge::Above => "workspace subsampled intra DC above sample sum",
    }
}

const fn dc_sampled_count_context(edge: IntraDcEdge) -> &'static str {
    match edge {
        IntraDcEdge::Left => "workspace subsampled intra DC left sample count",
        IntraDcEdge::Above => "workspace subsampled intra DC above sample count",
    }
}
