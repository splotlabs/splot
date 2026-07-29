// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Owned rectangular current-frame reconstruction storage.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use core::slice;

use super::*;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct OwnedFramePlaneRect<T: ReconSample> {
    pub(super) plane: PlaneId,
    pub(super) storage_size: PlaneSize,
    pub(super) rect: PlaneRect,
    pub(super) samples: Vec<T>,
}

impl<T: ReconSample> OwnedFramePlaneRect<T> {
    fn new(plane: PlaneId, storage_size: PlaneSize, rect: PlaneRect, fill: T) -> Result<Self> {
        ensure_rect_in_storage(plane, storage_size, rect)?;
        let len =
            rect.width()
                .checked_mul(rect.height())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "owned rectangle sample count",
                })?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane,
                context: "owned rectangle samples",
            })?;
        samples.resize(len, fill);
        Ok(Self {
            plane,
            storage_size,
            rect,
            samples,
        })
    }

    pub(super) fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_surface_rect(self.plane, self.rect, rect)
    }

    pub(super) fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        let row_start = rect.y() - self.rect.y();
        let row_end = row_start + rect.height();
        let stride = self.rect.width();
        Ok(WorkspaceRectRows::Owned(OwnedFrameRectRows {
            rows: self.samples[row_start * stride..row_end * stride].chunks_exact(stride),
            x: rect.x() - self.rect.x(),
            width: rect.width(),
        }))
    }

    pub(super) fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
        max_sample: u16,
    ) -> Result<()> {
        let rect = clamp_rect_to_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        validate_write_source(
            self.plane,
            rect,
            samples,
            row_stride_samples,
            self.storage_size.width(),
            max_sample,
        )?;
        let stride = self.rect.width();
        let local_x = rect.x() - self.rect.x();
        let local_y = rect.y() - self.rect.y();
        for row in 0..rect.height() {
            let source = row * row_stride_samples;
            let target = (local_y + row) * stride + local_x;
            // splot-copy-ok: materialize an exclusive scheduler-owned reconstruction row
            self.samples[target..target + rect.width()]
                .copy_from_slice(&samples[source..source + rect.width()]);
        }
        Ok(())
    }

    fn publish_into(&self, target: &mut CurrentFramePlane<T>) -> Result<()> {
        if target.storage_size != self.storage_size {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: target.storage_size,
                rect: self.rect,
            });
        }
        target.ensure_rect(self.rect)?;
        let stride = self.rect.width();
        for row in 0..self.rect.height() {
            let source = row * stride;
            let target_start = (self.rect.y() + row) * target.stride_samples + self.rect.x();
            // splot-copy-ok: commit an exclusive scheduler-owned row into the frame surface
            target.samples[target_start..target_start + stride]
                .copy_from_slice(&self.samples[source..source + stride]);
        }
        Ok(())
    }
}

/// Caller-owned Y/U/V storage for one rectangular current-frame region.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFrameRect<T: ReconSample> {
    info: DecodedFrameInfo,
    y: OwnedFramePlaneRect<T>,
    u: Option<OwnedFramePlaneRect<T>>,
    v: Option<OwnedFramePlaneRect<T>>,
}

impl<T: ReconSample> OwnedFrameRect<T> {
    /// Allocates one tightly packed rectangular reconstruction target.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the rectangle exceeds the frame or its
    /// per-plane storage cannot be allocated.
    pub fn new(info: DecodedFrameInfo, luma_rect: PlaneRect, fill: T) -> Result<Self> {
        let luma = info.coded_luma_size();
        let y_size = PlaneSize::new(luma.width(), luma.height())?;
        let chroma_size = info.pixel_format().chroma_size(luma)?;
        let chroma_rect = match chroma_size {
            Some(size) => subsampled_rects(
                &[luma_rect],
                info.pixel_format().subsampling_x(),
                info.pixel_format().subsampling_y(),
                size,
            )?
            .into_iter()
            .next(),
            None => None,
        };
        Ok(Self {
            info,
            y: OwnedFramePlaneRect::new(PlaneId::Y, y_size, luma_rect, fill)?,
            u: chroma_rect
                .zip(chroma_size)
                .map(|(rect, size)| OwnedFramePlaneRect::new(PlaneId::U, size, rect, fill))
                .transpose()?,
            v: chroma_rect
                .zip(chroma_size)
                .map(|(rect, size)| OwnedFramePlaneRect::new(PlaneId::V, size, rect, fill))
                .transpose()?,
        })
    }

    /// Returns the decoded-frame metadata for this rectangle.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the luma rectangle in global frame coordinates.
    pub const fn luma_rect(&self) -> PlaneRect {
        self.y.rect
    }

    pub(super) fn plane(&self, plane: PlaneId) -> Result<&OwnedFramePlaneRect<T>> {
        select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())
    }

    pub(super) fn plane_mut(&mut self, plane: PlaneId) -> Result<&mut OwnedFramePlaneRect<T>> {
        select_plane_mut(plane, &mut self.y, self.u.as_mut(), self.v.as_mut())
    }

    /// Publishes this completed rectangle into its matching frame workspace.
    ///
    /// # Errors
    /// Returns [`ReconError`] when plane geometry differs.
    pub fn publish_into(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        self.y.publish_into(&mut workspace.y)?;
        if let (Some(source), Some(target)) = (&self.u, &mut workspace.u) {
            source.publish_into(target)?;
        }
        if let (Some(source), Some(target)) = (&self.v, &mut workspace.v) {
            source.publish_into(target)?;
        }
        Ok(())
    }
}

/// Iterator over rows borrowed from one owned rectangular surface.
#[derive(Debug)]
pub struct OwnedFrameRectRows<'a, T: ReconSample> {
    rows: slice::ChunksExact<'a, T>,
    x: usize,
    width: usize,
}

impl<'a, T: ReconSample> Iterator for OwnedFrameRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next()?;
        Some(&row[self.x..self.x + self.width])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl<T: ReconSample> ExactSizeIterator for OwnedFrameRectRows<'_, T> {}
