// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Edge and row-view helpers for current-frame workspaces.

use super::CurrentFramePlane;
use crate::{IntraDcEdges, PlaneRect, ReconSample};

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
    left: Option<Vec<T>>,
    above: Option<Vec<T>>,
}

impl<T: ReconSample> CurrentFrameIntraEdges<T> {
    pub(super) const fn new(left: Option<Vec<T>>, above: Option<Vec<T>>) -> Self {
        Self { left, above }
    }

    /// Returns left edge samples when they were inside workspace storage.
    pub fn left_samples(&self) -> Option<&[T]> {
        self.left.as_deref()
    }

    /// Returns above edge samples when they were inside workspace storage.
    pub fn above_samples(&self) -> Option<&[T]> {
        self.above.as_deref()
    }

    /// Borrows the owned edges as DC prediction inputs.
    pub fn as_dc_edges(&self) -> IntraDcEdges<'_, T> {
        IntraDcEdges::new(self.left_samples(), self.above_samples())
    }
}
