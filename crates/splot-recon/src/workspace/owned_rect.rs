// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Owned rectangular current-frame reconstruction storage.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use core::ops::Range;
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
            let target_start = (self.rect.y() + row) * target.stride_samples() + self.rect.x();
            // splot-copy-ok: commit an exclusive scheduler-owned row into the frame surface
            target.samples[target_start..target_start + stride]
                .copy_from_slice(&self.samples[source..source + stride]);
        }
        Ok(())
    }
}

/// Movable full-width storage for one contiguous band of a frame plane.
///
/// The band owns its samples and can therefore cross scheduler task
/// boundaries without borrowing the frame workspace.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFramePlaneBand<T: ReconSample>(OwnedFramePlaneRect<T>);

impl<T: ReconSample> OwnedFramePlaneBand<T> {
    /// Returns the plane this band belongs to.
    pub const fn plane(&self) -> PlaneId {
        self.0.plane
    }

    /// Returns the complete frame-plane storage size.
    pub const fn storage_size(&self) -> PlaneSize {
        self.0.storage_size
    }

    /// Returns this band's rectangle in global plane coordinates.
    pub const fn rect(&self) -> PlaneRect {
        self.0.rect
    }

    /// Returns the tightly packed band samples.
    pub fn samples(&self) -> &[T] {
        &self.0.samples
    }

    /// Returns the tightly packed band samples mutably.
    pub fn samples_mut(&mut self) -> &mut [T] {
        &mut self.0.samples
    }

    /// Iterates over this band's complete plane rows.
    pub fn rows(&self) -> slice::ChunksExact<'_, T> {
        self.0.samples.chunks_exact(self.0.rect.width())
    }

    /// Iterates mutably over this band's complete plane rows.
    pub fn rows_mut(&mut self) -> slice::ChunksExactMut<'_, T> {
        self.0.samples.chunks_exact_mut(self.0.rect.width())
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

    /// Resets every sample while retaining the rectangle's allocations.
    pub fn fill(&mut self, sample: T) {
        self.y.samples.fill(sample);
        if let Some(u) = self.u.as_mut() {
            u.samples.fill(sample);
        }
        if let Some(v) = self.v.as_mut() {
            v.samples.fill(sample);
        }
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

/// Movable canonical storage for one full-width luma superblock row.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFrameRowBand<T: ReconSample> {
    rect: OwnedFrameRect<T>,
}

impl<T: ReconSample> OwnedFrameRowBand<T> {
    /// Allocates one full-width row band, clipping the final range to the frame.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the row range is empty or outside the coded
    /// frame, or when its per-plane storage cannot be allocated.
    pub fn new(info: DecodedFrameInfo, luma_rows: Range<usize>, fill: T) -> Result<Self> {
        let luma = info.coded_luma_size();
        if luma_rows.start >= luma_rows.end || luma_rows.end > luma.height() {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: PlaneId::Y,
                storage: PlaneSize::new(luma.width(), luma.height())?,
                rect: PlaneRect::new(
                    0,
                    luma_rows.start,
                    luma.width(),
                    luma_rows.end.saturating_sub(luma_rows.start),
                )?,
            });
        }
        Ok(Self {
            rect: OwnedFrameRect::new(
                info,
                PlaneRect::new(
                    0,
                    luma_rows.start,
                    luma.width(),
                    luma_rows.end - luma_rows.start,
                )?,
                fill,
            )?,
        })
    }

    /// Returns the decoded-frame metadata for this band.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.rect.info()
    }

    /// Returns the luma rectangle in global frame coordinates.
    pub const fn luma_rect(&self) -> PlaneRect {
        self.rect.luma_rect()
    }

    /// Borrows this owned band as a checked reconstruction target.
    pub fn surface_mut(&mut self) -> CurrentFrameSurface<'_, '_, T> {
        CurrentFrameSurface::OwnedRect(&mut self.rect)
    }

    /// Splits the row owner into its disjoint plane bands.
    pub fn into_planes(
        self,
    ) -> (
        OwnedFramePlaneBand<T>,
        Option<OwnedFramePlaneBand<T>>,
        Option<OwnedFramePlaneBand<T>>,
    ) {
        (
            OwnedFramePlaneBand(self.rect.y),
            self.rect.u.map(OwnedFramePlaneBand),
            self.rect.v.map(OwnedFramePlaneBand),
        )
    }
}

/// A frame's canonical raw pixels stored as movable full-width row bands.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFrameBands<T: ReconSample> {
    info: DecodedFrameInfo,
    next_luma_y: usize,
    y: Vec<OwnedFramePlaneBand<T>>,
    u: Vec<OwnedFramePlaneBand<T>>,
    v: Vec<OwnedFramePlaneBand<T>>,
}

/// Disjoint mutable rows for the luma plane and any present chroma planes.
pub type OwnedFrameRowsMut<'a, T> = (
    Vec<&'a mut [T]>,
    Option<Vec<&'a mut [T]>>,
    Option<Vec<&'a mut [T]>>,
);

impl<T: ReconSample> OwnedFrameBands<T> {
    /// Creates an empty segmented frame owner.
    pub const fn new(info: DecodedFrameInfo) -> Self {
        Self {
            info,
            next_luma_y: 0,
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
        }
    }

    /// Returns the decoded-frame metadata.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Appends the next canonical row band.
    ///
    /// # Errors
    /// Returns [`ReconError`] when metadata or row geometry is not the exact
    /// next full-width band.
    pub fn push(&mut self, band: OwnedFrameRowBand<T>) -> Result<()> {
        let rect = band.luma_rect();
        let size = self.info.coded_luma_size();
        if band.info() != self.info
            || rect.x() != 0
            || rect.width() != size.width()
            || rect.y() != self.next_luma_y
        {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: PlaneId::Y,
                storage: PlaneSize::new(size.width(), size.height())?,
                rect,
            });
        }
        let end = rect
            .y()
            .checked_add(rect.height())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "owned frame band end",
            })?;
        let (y, u, v) = band.into_planes();
        self.y.push(y);
        if let Some(u) = u {
            self.u.push(u);
        }
        if let Some(v) = v {
            self.v.push(v);
        }
        self.next_luma_y = end;
        Ok(())
    }

    /// Returns the owned bands for one plane.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] when chroma is absent.
    pub fn plane_bands(&self, plane: PlaneId) -> Result<&[OwnedFramePlaneBand<T>]> {
        match plane {
            PlaneId::Y => Ok(&self.y),
            PlaneId::U if !self.u.is_empty() => Ok(&self.u),
            PlaneId::V if !self.v.is_empty() => Ok(&self.v),
            PlaneId::U | PlaneId::V => Err(ReconError::MissingWorkspacePlane { plane }),
        }
    }

    /// Splits all present plane bands into disjoint mutable row slices.
    pub fn plane_rows_mut(&mut self) -> OwnedFrameRowsMut<'_, T> {
        fn rows<T: ReconSample>(bands: &mut [OwnedFramePlaneBand<T>]) -> Vec<&mut [T]> {
            bands
                .iter_mut()
                .flat_map(OwnedFramePlaneBand::rows_mut)
                .collect()
        }
        let y = rows(&mut self.y);
        let u = (!self.u.is_empty()).then(|| rows(&mut self.u));
        let v = (!self.v.is_empty()).then(|| rows(&mut self.v));
        (y, u, v)
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
