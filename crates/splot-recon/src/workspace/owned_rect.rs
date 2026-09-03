// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Owned rectangular current-frame reconstruction storage.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use core::slice;

use super::*;

/// One plane's slice of an [`OwnedFrameRect`]'s single allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnedFramePlaneRect {
    pub(super) plane: PlaneId,
    pub(super) storage_size: PlaneSize,
    pub(super) rect: PlaneRect,
    start: usize,
    len: usize,
}

/// One plane's samples plus the geometry describing them.
pub(super) struct OwnedPlaneRef<'a, T: ReconSample> {
    region: OwnedFramePlaneRect,
    samples: &'a [T],
}

/// The mutable counterpart of [`OwnedPlaneRef`].
pub(super) struct OwnedPlaneMut<'a, T: ReconSample> {
    region: OwnedFramePlaneRect,
    samples: &'a mut [T],
}

impl OwnedFramePlaneRect {
    /// Describes one plane's slice, starting at `start` in the shared buffer.
    fn new(plane: PlaneId, storage_size: PlaneSize, rect: PlaneRect, start: usize) -> Result<Self> {
        ensure_rect_in_storage(plane, storage_size, rect)?;
        let len =
            rect.width()
                .checked_mul(rect.height())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "owned rectangle sample count",
                })?;
        Ok(Self {
            plane,
            storage_size,
            rect,
            start,
            len,
        })
    }

    fn end(&self) -> usize {
        self.start.saturating_add(self.len)
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_surface_rect(self.plane, self.rect, rect)
    }
}

impl<'a, T: ReconSample> OwnedPlaneRef<'a, T> {
    pub(super) const fn storage_size(&self) -> PlaneSize {
        self.region.storage_size
    }

    pub(super) fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'a, T>> {
        ensure_rect_in_storage(self.region.plane, self.region.storage_size, rect)?;
        self.region.ensure_rect(rect)?;
        let row_start = rect.y() - self.region.rect.y();
        let row_end = row_start + rect.height();
        let stride = self.region.rect.width();
        Ok(WorkspaceRectRows::Owned(OwnedFrameRectRows {
            rows: self.samples[row_start * stride..row_end * stride].chunks_exact(stride),
            x: rect.x() - self.region.rect.x(),
            width: rect.width(),
        }))
    }
}

impl<'a, T: ReconSample> OwnedPlaneMut<'a, T> {
    pub(super) const fn storage_size(&self) -> PlaneSize {
        self.region.storage_size
    }

    pub(super) const fn rect(&self) -> PlaneRect {
        self.region.rect
    }

    pub(super) const fn plane(&self) -> PlaneId {
        self.region.plane
    }

    pub(super) fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        self.region.ensure_rect(rect)
    }

    pub(super) const fn samples_mut(&mut self) -> &mut [T] {
        self.samples
    }

    /// Releases the plane's samples for the caller's own lifetime.
    pub(super) const fn into_samples_mut(self) -> &'a mut [T] {
        self.samples
    }

    pub(super) fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
        max_sample: u16,
    ) -> Result<()> {
        let rect = clamp_rect_to_storage(self.region.plane, self.region.storage_size, rect)?;
        self.region.ensure_rect(rect)?;
        validate_write_source(
            self.region.plane,
            rect,
            samples,
            row_stride_samples,
            self.region.storage_size.width(),
            max_sample,
        )?;
        let stride = self.region.rect.width();
        let local_x = rect.x() - self.region.rect.x();
        let local_y = rect.y() - self.region.rect.y();
        for row in 0..rect.height() {
            let source = row * row_stride_samples;
            let target = (local_y + row) * stride + local_x;
            copy_row_samples(
                &mut self.samples[target..target + rect.width()],
                &samples[source..source + rect.width()],
            );
        }
        Ok(())
    }
}

fn publish_plane_into<T: ReconSample>(
    region: OwnedFramePlaneRect,
    samples: &[T],
    target: &mut CurrentFramePlane<T>,
) -> Result<()> {
    if target.storage_size != region.storage_size {
        return Err(ReconError::WorkspaceRectOutOfBounds {
            plane: region.plane,
            storage: target.storage_size,
            rect: region.rect,
        });
    }
    target.ensure_rect(region.rect)?;
    let stride = region.rect.width();
    for row in 0..region.rect.height() {
        let source = row * stride;
        let target_start = (region.rect.y() + row) * target.stride_samples() + region.rect.x();
        let target_end = target_start + stride;
        copy_row_samples(
            &mut target.samples[target_start..target_end],
            &samples[source..source + stride],
        );
    }
    Ok(())
}

/// Caller-owned Y/U/V storage for one rectangular current-frame region.
///
/// The three planes share **one** allocation, laid out Y then U then V, so a
/// band costs the allocator a single block rather than three. dav2d packs a
/// whole picture the same way.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFrameRect<T: ReconSample> {
    info: DecodedFrameInfo,
    samples: Vec<T>,
    y: OwnedFramePlaneRect,
    u: Option<OwnedFramePlaneRect>,
    v: Option<OwnedFramePlaneRect>,
}

impl<T: ReconSample> OwnedFrameRect<T> {
    /// Allocates one tightly packed rectangular reconstruction target.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the rectangle exceeds the frame or its
    /// storage cannot be allocated.
    fn regions(
        info: DecodedFrameInfo,
        luma_rect: PlaneRect,
    ) -> Result<(
        OwnedFramePlaneRect,
        Option<OwnedFramePlaneRect>,
        Option<OwnedFramePlaneRect>,
    )> {
        let luma = info.coded_luma_size();
        let y_size = PlaneSize::new(luma.width(), luma.height())?;
        let chroma_size = info.pixel_format().chroma_size(luma)?;
        let chroma_rect = match chroma_size {
            Some(size) => Some(super::subsampled_rect(
                luma_rect,
                info.pixel_format().subsampling_x(),
                info.pixel_format().subsampling_y(),
                size,
            )?),
            None => None,
        };
        let y = OwnedFramePlaneRect::new(PlaneId::Y, y_size, luma_rect, 0)?;
        let u = chroma_rect
            .zip(chroma_size)
            .map(|(rect, size)| OwnedFramePlaneRect::new(PlaneId::U, size, rect, y.end()))
            .transpose()?;
        let v = u
            .zip(chroma_rect)
            .zip(chroma_size)
            .map(|((u, rect), size)| OwnedFramePlaneRect::new(PlaneId::V, size, rect, u.end()))
            .transpose()?;
        Ok((y, u, v))
    }

    /// Allocates one tightly packed rectangular reconstruction target.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the rectangle exceeds the frame or its
    /// storage cannot be allocated.
    pub fn new(info: DecodedFrameInfo, luma_rect: PlaneRect, fill: T) -> Result<Self> {
        let (y, u, v) = Self::regions(info, luma_rect)?;
        let total = v.or(u).map_or_else(|| y.end(), |last| last.end());
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(total)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "owned rectangle samples",
            })?;
        samples.resize(total, fill);
        Ok(Self {
            info,
            samples,
            y,
            u,
            v,
        })
    }

    /// Returns the decoded-frame metadata for this rectangle.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Points this rectangle's storage at another region of the same frame.
    ///
    /// Reconstruction targets are one superblock each, so every interior
    /// superblock needs exactly the same buffer; only the coordinates differ.
    /// Retargeting lets one allocation serve any of them in turn instead of
    /// keeping one per superblock alive for the whole frame.
    ///
    /// # Errors
    ///
    /// Returns [`ReconError`] when the new rectangle needs different storage,
    /// leaving this rectangle untouched so the caller can allocate instead.
    pub fn retarget(&mut self, luma_rect: PlaneRect) -> Result<()> {
        let (y, u, v) = Self::regions(self.info, luma_rect)?;
        let total = v.or(u).map_or_else(|| y.end(), |last| last.end());
        if total != self.samples.len() {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: PlaneId::Y,
                storage: y.storage_size,
                rect: luma_rect,
            });
        }
        self.y = y;
        self.u = u;
        self.v = v;
        Ok(())
    }

    /// Returns the luma rectangle in global frame coordinates.
    pub const fn luma_rect(&self) -> PlaneRect {
        self.y.rect
    }

    /// Resets every sample while retaining the rectangle's allocation.
    pub fn fill(&mut self, sample: T) {
        self.samples.fill(sample);
    }

    pub(super) fn plane(&self, plane: PlaneId) -> Result<OwnedPlaneRef<'_, T>> {
        let region = *select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())?;
        Ok(OwnedPlaneRef {
            region,
            samples: &self.samples[region.start..region.end()],
        })
    }

    pub(super) fn plane_mut(&mut self, plane: PlaneId) -> Result<OwnedPlaneMut<'_, T>> {
        let region = *select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())?;
        Ok(OwnedPlaneMut {
            region,
            samples: &mut self.samples[region.start..region.end()],
        })
    }

    /// Publishes this completed rectangle into its matching frame workspace.
    ///
    /// # Errors
    /// Returns [`ReconError`] when plane geometry differs.
    pub fn publish_into(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        publish_plane_into(
            self.y,
            &self.samples[self.y.start..self.y.end()],
            &mut workspace.y,
        )?;
        if let (Some(region), Some(target)) = (self.u, workspace.u.as_mut()) {
            publish_plane_into(region, &self.samples[region.start..region.end()], target)?;
        }
        if let (Some(region), Some(target)) = (self.v, workspace.v.as_mut()) {
            publish_plane_into(region, &self.samples[region.start..region.end()], target)?;
        }
        Ok(())
    }
}

/// Rows of one owned rectangle, clipped to a requested sub-rectangle.
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
