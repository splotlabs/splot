// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mutable current-frame reconstruction workspace.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`,
//! `RECON-INTRABC-CURRENT-FRAME-COPY`,
//! `RECON-INTRA-DC-RECTANGULAR-PREDICTION`,
//! `RECON-INTRA-BASIC-PAETH-PREDICTION`,
//! `RECON-INTRA-SMOOTH-PREDICTION`,
//! `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`,
//! `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`.

use core::mem;
use core::ops::Range;
use core::slice;

use splot_core::headers::sequence::SuperblockSize;

use crate::intra_basic::predict_paeth_sample;
use crate::intra_dc_math::validate_sample_type;
use crate::intra_directional::predict_intra_cardinal_directional_rect_into;
use crate::intra_smooth::{SmoothSampleEdges, SmoothSamplePosition, predict_smooth_sample_values};
use crate::{
    DecodedFrame, DecodedFrameInfo, FrameMut, FramePlanes, FrameRef, IntraCardinalDirection,
    IntraDirectionalAngleEdge, IntraPaethEdge, IntraRectBlockSize, IntraSmoothEdge,
    IntraSmoothMode, IntraSquareBlockSize, PixelFormat, Plane, PlaneId, PlaneMut, PlaneRect,
    PlaneRef, PlaneRefRows, PlaneSize, ReconError, ReconSample, Result,
};

#[path = "workspace_edges.rs"]
mod workspace_edges;
#[path = "workspace_interintra.rs"]
mod workspace_interintra;
#[path = "workspace_intra_dc.rs"]
mod workspace_intra_dc;
#[path = "workspace_intra_directional_angle.rs"]
mod workspace_intra_directional_angle;
pub use workspace_edges::CurrentFrameIntraEdges;
pub use workspace_interintra::{InterIntraMode, wedge_mask_plane_sample};

/// Mutable current-frame reconstruction workspace.
///
/// The workspace owns checked plane storage that future decode or encoder paths
/// can fill incrementally before freezing into the immutable [`DecodedFrame`]
/// model. It is intentionally scheduler-free: callers own any parallel
/// partitioning above this type.
///
/// Does not implement `Clone`: it owns the current-frame plane buffers. Borrow it
/// as a [`FrameRef`]/[`FrameMut`] with [`CurrentFrameWorkspace::as_frame_ref`]/
/// [`CurrentFrameWorkspace::as_frame_mut`] instead (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Eq, PartialEq)]
pub struct CurrentFrameWorkspace<T: ReconSample> {
    info: DecodedFrameInfo,
    y: CurrentFramePlane<T>,
    u: Option<CurrentFramePlane<T>>,
    v: Option<CurrentFramePlane<T>>,
    intra_prediction_scratch: IntraPredictionScratch<T>,
}

/// Selects one of the two reusable current-frame intra-prediction buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntraPredictionScratchBuffer {
    /// Primary prediction storage used by every intra mode.
    Primary,
    /// Secondary prediction storage used while blending two predictors.
    Secondary,
}

/// Two reusable prediction buffers owned by one reconstruction task.
///
/// A workspace keeps one instance for the existing ordered reconstruction API.
/// Parallel row reconstruction can instead keep one instance per task without
/// sharing scratch storage between workers.
#[derive(Debug, Eq, PartialEq)]
pub struct IntraPredictionScratch<T: ReconSample> {
    buffers: [Vec<T>; 2],
}

impl<T: ReconSample> IntraPredictionScratch<T> {
    /// Creates empty reusable prediction buffers.
    pub const fn new() -> Self {
        Self {
            buffers: [Vec::new(), Vec::new()],
        }
    }

    /// Creates two reusable prediction buffers with equal initial capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffers: core::array::from_fn(|_| Vec::with_capacity(capacity)),
        }
    }

    /// Takes one initialized prediction buffer from this scratch owner.
    ///
    /// # Errors
    /// Returns [`ReconError`] if `sample_count` exceeds the largest AV2 intra
    /// block or the buffer cannot reserve enough storage.
    pub fn take_buffer(
        &mut self,
        slot: IntraPredictionScratchBuffer,
        plane: PlaneId,
        sample_count: usize,
        fill: T,
    ) -> Result<Vec<T>> {
        if sample_count > MAX_INTRA_PREDICTION_SAMPLES {
            return Err(ReconError::WorkspaceIntraPredictionScratchTooLarge {
                sample_count,
                max_sample_count: MAX_INTRA_PREDICTION_SAMPLES,
            });
        }
        let mut buffer = mem::take(self.buffer_mut(slot));
        buffer.clear();
        if buffer.capacity() < sample_count {
            buffer.try_reserve_exact(sample_count).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane,
                    context: "intra prediction scratch",
                }
            })?;
        }
        buffer.resize(sample_count, fill);
        Ok(buffer)
    }

    /// Returns a prediction buffer to this scratch owner for reuse.
    pub fn recycle_buffer(&mut self, slot: IntraPredictionScratchBuffer, mut buffer: Vec<T>) {
        buffer.clear();
        *self.buffer_mut(slot) = buffer;
    }

    fn buffer_mut(&mut self, slot: IntraPredictionScratchBuffer) -> &mut Vec<T> {
        match slot {
            IntraPredictionScratchBuffer::Primary => &mut self.buffers[0],
            IntraPredictionScratchBuffer::Secondary => &mut self.buffers[1],
        }
    }
}

impl<T: ReconSample> Default for IntraPredictionScratch<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Exclusive full-width plane storage for one superblock row.
///
/// `rect` uses global plane coordinates while `samples` starts at that
/// rectangle's first row. The storage contains whole stride rows and aliases no
/// other band returned by the same [`CurrentFrameRowBands`] iterator.
#[derive(Debug)]
pub struct CurrentFramePlaneRowBand<'a, T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    stride_samples: usize,
    rect: PlaneRect,
    samples: &'a mut [T],
}

impl<T: ReconSample> CurrentFramePlaneRowBand<'_, T> {
    /// Returns the complete frame-plane storage size.
    pub const fn storage_size(&self) -> PlaneSize {
        self.storage_size
    }

    /// Returns this full-width band's rectangle in global plane coordinates.
    pub const fn rect(&self) -> PlaneRect {
        self.rect
    }

    /// Returns the frame-plane stride in samples.
    pub const fn stride_samples(&self) -> usize {
        self.stride_samples
    }

    /// Returns this band's exclusive backing rows.
    ///
    /// Callers own bit-depth range enforcement, as with
    /// [`PlaneMut::samples_mut`].
    pub fn samples_mut(&mut self) -> &mut [T] {
        self.samples
    }

    fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        let local = PlaneRect::new(
            rect.x() - self.rect.x(),
            rect.y() - self.rect.y(),
            rect.width(),
            rect.height(),
        )?;
        Ok(WorkspaceRectRows::Strided(
            PlaneRef::from_parts(self.samples, self.stride_samples, local).visible_rows(),
        ))
    }

    fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
        max_sample: u16,
    ) -> Result<()> {
        let rect = clamp_rect_to_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        let local_y = rect.y() - self.rect.y();
        write_rect_to_samples(
            self.plane,
            self.samples,
            self.stride_samples,
            rect,
            rect.x() - self.rect.x(),
            local_y,
            samples,
            row_stride_samples,
            max_sample,
        )
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_surface_rect(self.plane, self.rect, rect)
    }
}

/// Exclusive Y/U/V plane bands for one full-width superblock row.
#[derive(Debug)]
pub struct CurrentFrameRowBand<'a, T: ReconSample> {
    info: DecodedFrameInfo,
    y: CurrentFramePlaneRowBand<'a, T>,
    u: Option<CurrentFramePlaneRowBand<'a, T>>,
    v: Option<CurrentFramePlaneRowBand<'a, T>>,
}

/// Exclusive rectangular plane storage assembled from disjoint row slices.
#[derive(Debug)]
pub struct CurrentFramePlaneRect<'a, T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    rect: PlaneRect,
    rows: Vec<&'a mut [T]>,
}

impl<T: ReconSample> CurrentFramePlaneRect<'_, T> {
    /// Returns the complete frame-plane storage size.
    pub const fn storage_size(&self) -> PlaneSize {
        self.storage_size
    }

    /// Returns this surface's rectangle in global plane coordinates.
    pub const fn rect(&self) -> PlaneRect {
        self.rect
    }

    fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        let row_start = rect.y() - self.rect.y();
        let row_end = row_start + rect.height();
        Ok(WorkspaceRectRows::Sliced(CurrentFrameRectRows {
            rows: self.rows[row_start..row_end].iter(),
            x: rect.x() - self.rect.x(),
            width: rect.width(),
        }))
    }

    fn write_rect(
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
        let row_start = rect.y() - self.rect.y();
        let x = rect.x() - self.rect.x();
        for (row, target) in self.rows[row_start..row_start + rect.height()]
            .iter_mut()
            .enumerate()
        {
            let source_start = row * row_stride_samples;
            // splot-copy-ok: publish caller-owned samples into this exclusive rectangle.
            target[x..x + rect.width()]
                .copy_from_slice(&samples[source_start..source_start + rect.width()]);
        }
        Ok(())
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_surface_rect(self.plane, self.rect, rect)
    }

    fn publish_into(&self, target: &mut CurrentFramePlane<T>, max_sample: u16) -> Result<()> {
        if target.storage_size != self.storage_size {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: target.storage_size,
                rect: self.rect,
            });
        }
        target.ensure_rect(self.rect)?;
        for (row, samples) in self.rows.iter().enumerate() {
            let rect = PlaneRect::new(self.rect.x(), self.rect.y() + row, self.rect.width(), 1)?;
            target.write_rect(rect, samples, self.rect.width(), max_sample)?;
        }
        Ok(())
    }
}

fn ensure_surface_rect(plane: PlaneId, band: PlaneRect, rect: PlaneRect) -> Result<()> {
    if rect_is_within(rect, band) {
        Ok(())
    } else {
        Err(ReconError::WorkspaceRowBandRectOutOfBounds { plane, band, rect })
    }
}

/// Exclusive Y/U/V storage for one rectangular current-frame region.
#[derive(Debug)]
pub struct CurrentFrameRect<'a, T: ReconSample> {
    info: DecodedFrameInfo,
    y: CurrentFramePlaneRect<'a, T>,
    u: Option<CurrentFramePlaneRect<'a, T>>,
    v: Option<CurrentFramePlaneRect<'a, T>>,
}

impl<'storage, T: ReconSample> CurrentFrameRect<'storage, T> {
    /// Returns the decoded-frame metadata for this rectangle.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the luma rectangle in global plane coordinates.
    pub const fn luma_rect(&self) -> PlaneRect {
        self.y.rect
    }

    fn plane(&self, plane: PlaneId) -> Result<&CurrentFramePlaneRect<'storage, T>> {
        select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())
    }

    fn plane_mut(&mut self, plane: PlaneId) -> Result<&mut CurrentFramePlaneRect<'storage, T>> {
        select_plane_mut(plane, &mut self.y, self.u.as_mut(), self.v.as_mut())
    }

    /// Publishes this completed rectangle into its matching current-frame workspace.
    ///
    /// # Errors
    /// Returns [`ReconError`] when plane geometry differs or a sample exceeds
    /// the workspace bit depth.
    pub fn publish_into(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        let max_sample = workspace.info.bit_depth().max_sample();
        self.y.publish_into(&mut workspace.y, max_sample)?;
        match (&self.u, &mut workspace.u) {
            (Some(source), Some(target)) => source.publish_into(target, max_sample)?,
            (None, None) => {}
            (Some(_), None) => {
                return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U });
            }
            (None, Some(_)) => {
                return Err(ReconError::MissingChromaPlane { plane: PlaneId::U });
            }
        }
        match (&self.v, &mut workspace.v) {
            (Some(source), Some(target)) => source.publish_into(target, max_sample)?,
            (None, None) => {}
            (Some(_), None) => {
                return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::V });
            }
            (None, Some(_)) => {
                return Err(ReconError::MissingChromaPlane { plane: PlaneId::V });
            }
        }
        Ok(())
    }
}

/// Iterator over checked workspace rectangle rows.
#[derive(Debug)]
pub enum WorkspaceRectRows<'a, T: ReconSample> {
    /// Rows backed by conventional stride-based plane storage.
    Strided(PlaneRefRows<'a, T>),
    /// Rows backed by individually partitioned rectangle slices.
    Sliced(CurrentFrameRectRows<'a, T>),
}

impl<'a, T: ReconSample> Iterator for WorkspaceRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Strided(rows) => rows.next(),
            Self::Sliced(rows) => rows.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Strided(rows) => rows.size_hint(),
            Self::Sliced(rows) => rows.size_hint(),
        }
    }
}

impl<T: ReconSample> ExactSizeIterator for WorkspaceRectRows<'_, T> {}

/// Iterator over rows borrowed from one rectangular surface.
#[derive(Debug)]
pub struct CurrentFrameRectRows<'a, T: ReconSample> {
    rows: slice::Iter<'a, &'a mut [T]>,
    x: usize,
    width: usize,
}

impl<'a, T: ReconSample> Iterator for CurrentFrameRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next()?;
        Some(&row[self.x..self.x + self.width])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl<T: ReconSample> ExactSizeIterator for CurrentFrameRectRows<'_, T> {}

impl<'a, T: ReconSample> CurrentFrameRowBand<'a, T> {
    /// Returns the decoded-frame metadata for this row band.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the luma rectangle in global plane coordinates.
    pub const fn luma_rect(&self) -> PlaneRect {
        self.y.rect
    }

    /// Splits this row band into its disjoint per-plane bands.
    pub fn into_planes(
        self,
    ) -> (
        CurrentFramePlaneRowBand<'a, T>,
        Option<CurrentFramePlaneRowBand<'a, T>>,
        Option<CurrentFramePlaneRowBand<'a, T>>,
    ) {
        (self.y, self.u, self.v)
    }

    fn plane(&self, plane: PlaneId) -> Result<&CurrentFramePlaneRowBand<'a, T>> {
        select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())
    }

    fn plane_mut(&mut self, plane: PlaneId) -> Result<&mut CurrentFramePlaneRowBand<'a, T>> {
        select_plane_mut(plane, &mut self.y, self.u.as_mut(), self.v.as_mut())
    }
}

/// Checked reconstruction target backed by a whole frame or one exclusive row band.
#[derive(Debug)]
pub enum CurrentFrameSurface<'surface, 'storage, T: ReconSample> {
    /// Existing ordered reconstruction over the complete current frame.
    Frame(&'surface mut CurrentFrameWorkspace<T>),
    /// Row-local reconstruction over one exclusive full-width superblock band.
    Row(&'surface mut CurrentFrameRowBand<'storage, T>),
    /// Reconstruction over one exclusive rectangular frame region.
    Rect(&'surface mut CurrentFrameRect<'storage, T>),
}

enum CurrentFrameResidualTarget<'surface, 'storage, T: ReconSample> {
    Contiguous {
        samples: &'surface mut [T],
        stride: usize,
        base: usize,
        rect: PlaneRect,
        max_sample: u16,
    },
    Sliced {
        rows: &'surface mut [&'storage mut [T]],
        x: usize,
        rect: PlaneRect,
        max_sample: u16,
    },
}

impl<T: ReconSample> CurrentFrameResidualTarget<'_, '_, T> {
    #[inline]
    fn add(self, mut residual_at: impl FnMut(usize, usize) -> i32) -> Result<()> {
        match self {
            Self::Contiguous {
                samples,
                stride,
                base,
                rect,
                max_sample,
            } => {
                let max = i32::from(max_sample);
                for row in 0..rect.height() {
                    let target_start = base + row * stride;
                    add_residual_row(
                        &mut samples[target_start..target_start + rect.width()],
                        row,
                        max,
                        &mut residual_at,
                    )?;
                }
            }
            Self::Sliced {
                rows,
                x,
                rect,
                max_sample,
            } => {
                let max = i32::from(max_sample);
                for (row, target) in rows.iter_mut().enumerate() {
                    add_residual_row(&mut target[x..x + rect.width()], row, max, &mut residual_at)?;
                }
            }
        }
        Ok(())
    }
}

fn add_residual_row<T: ReconSample>(
    samples: &mut [T],
    row: usize,
    max: i32,
    residual_at: &mut impl FnMut(usize, usize) -> i32,
) -> Result<()> {
    for (column, sample) in samples.iter_mut().enumerate() {
        let value = i32::from(sample.to_u16())
            .saturating_add(residual_at(row, column))
            .clamp(0, max) as u16;
        debug_assert!(value <= T::MAX_VALUE);
        *sample = T::try_from_u16(value)?;
    }
    Ok(())
}

impl<'storage, T: ReconSample> CurrentFrameSurface<'_, 'storage, T> {
    /// Returns the decoded-frame metadata for this target.
    pub fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Frame(workspace) => workspace.info(),
            Self::Row(row) => row.info(),
            Self::Rect(rect) => rect.info(),
        }
    }

    /// Returns the complete frame-plane storage size.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] for absent chroma planes.
    pub fn plane_storage_size(&self, plane: PlaneId) -> Result<PlaneSize> {
        match self {
            Self::Frame(workspace) => Ok(workspace.plane(plane)?.storage_size()),
            Self::Row(row) => Ok(row.plane(plane)?.storage_size()),
            Self::Rect(rect) => Ok(rect.plane(plane)?.storage_size()),
        }
    }

    /// Iterates over a checked target-local rectangle using global coordinates.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the rectangle exceeds
    /// frame storage, or a row target would read outside its exclusive band.
    pub fn rect_rows(&self, plane: PlaneId, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        match self {
            Self::Frame(workspace) => workspace.rect_rows(plane, rect),
            Self::Row(row) => row.plane(plane)?.rect_rows(rect),
            Self::Rect(surface) => surface.plane(plane)?.rect_rows(rect),
        }
    }

    /// Writes row-strided samples into a checked target rectangle.
    ///
    /// Frame-edge overhang is clipped exactly as for
    /// [`CurrentFrameWorkspace::write_rect`]. A row target rejects any clipped
    /// rectangle crossing its exclusive band before mutating samples.
    ///
    /// # Errors
    /// Returns [`ReconError`] for absent planes, invalid target or source
    /// geometry, cross-band access, or samples outside the active bit depth.
    pub fn write_rect(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
    ) -> Result<()> {
        match self {
            Self::Frame(workspace) => {
                workspace.write_rect(plane, rect, samples, row_stride_samples)
            }
            Self::Row(row) => {
                let max_sample = row.info.bit_depth().max_sample();
                row.plane_mut(plane)?
                    .write_rect(rect, samples, row_stride_samples, max_sample)
            }
            Self::Rect(surface) => {
                let max_sample = surface.info.bit_depth().max_sample();
                surface
                    .plane_mut(plane)?
                    .write_rect(rect, samples, row_stride_samples, max_sample)
            }
        }
    }

    /// Writes row-strided `u16` prediction samples directly into this surface.
    ///
    /// Source geometry and every sample are validated before the first target
    /// write, preserving the fail-atomic behavior of [`Self::write_rect`].
    /// Frame-edge overhang is clipped, while row and rectangle targets reject
    /// writes outside their exclusive region.
    ///
    /// # Errors
    /// Returns [`ReconError`] for absent planes, invalid source or target
    /// geometry, samples unsupported by `T`, or values outside the active bit
    /// depth.
    pub fn write_u16_rect(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[u16],
        row_stride_samples: usize,
    ) -> Result<()> {
        let storage = self.plane_storage_size(plane)?;
        let rect = clamp_rect_to_storage(plane, storage, rect)?;
        self.rect_rows(plane, rect)?;
        if row_stride_samples < rect.width() {
            return Err(ReconError::WorkspaceWriteStrideTooSmall {
                plane,
                stride_samples: row_stride_samples,
                width: rect.width(),
            });
        }
        let expected = required_row_strided_samples(rect, row_stride_samples)?;
        if samples.len() < expected {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane,
                expected,
                actual: samples.len(),
            });
        }
        let max_sample = self.info().bit_depth().max_sample();
        for (sample_index, &sample) in samples.iter().enumerate() {
            T::try_from_u16(sample)?;
            if sample > max_sample {
                return Err(ReconError::SampleOutOfRange {
                    plane,
                    sample_index,
                    value: sample,
                    max: max_sample,
                });
            }
        }

        match self {
            Self::Frame(workspace) => {
                let target = workspace.plane_mut(plane)?;
                write_u16_rect_to_samples(
                    &mut target.samples,
                    target.stride_samples,
                    rect,
                    rect.x(),
                    rect.y(),
                    samples,
                    row_stride_samples,
                )
            }
            Self::Row(row) => {
                let target = row.plane_mut(plane)?;
                target.ensure_rect(rect)?;
                write_u16_rect_to_samples(
                    target.samples,
                    target.stride_samples,
                    rect,
                    rect.x() - target.rect.x(),
                    rect.y() - target.rect.y(),
                    samples,
                    row_stride_samples,
                )
            }
            Self::Rect(surface) => {
                let target = surface.plane_mut(plane)?;
                target.ensure_rect(rect)?;
                let row_start = rect.y() - target.rect.y();
                let x = rect.x() - target.rect.x();
                for (row, target) in target.rows[row_start..row_start + rect.height()]
                    .iter_mut()
                    .enumerate()
                {
                    let source_start = row * row_stride_samples;
                    copy_u16_samples(
                        &mut target[x..x + rect.width()],
                        &samples[source_start..source_start + rect.width()],
                    )?;
                }
                Ok(())
            }
        }
    }

    /// Writes one contiguous rectangular prediction block.
    ///
    /// # Errors
    /// Returns [`ReconError`] under the same conditions as [`Self::write_rect`]
    /// or when `samples` does not exactly match the requested block size.
    pub fn write_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        samples: &[T],
    ) -> Result<()> {
        let rect = checked_sample_block_rect(plane, x, y, size, samples.len())?;
        self.write_rect(plane, rect, samples, size.width())
    }

    /// Adds a contiguous signed residual block directly to reconstructed samples.
    ///
    /// The complete source and target geometry and every prediction sample are
    /// validated before the first sample is changed. Frame-edge overhang is
    /// clipped, while row targets reject blocks crossing their exclusive band.
    ///
    /// # Errors
    /// Returns [`ReconError`] for an absent plane, invalid source or target
    /// geometry, cross-band access, or an existing prediction sample outside
    /// the active bit depth.
    #[inline]
    pub fn add_residual_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        residual: &[i32],
    ) -> Result<()> {
        let rect = checked_sample_block_rect(plane, x, y, size, residual.len())?;
        let source_stride = size.width();
        self.residual_rect_target(plane, rect, source_stride)?
            .add(|row, column| residual[row * source_stride + column])
    }

    /// Adds an adjusted-size signed residual block directly to reconstructed
    /// samples, applying the AV2 § 7.15.4 sample duplication for a 64-sample
    /// transform side while writing.
    ///
    /// The complete adjusted source and original target geometry and every
    /// prediction sample are validated before the first sample is changed.
    /// Frame-edge overhang is clipped using the original block stride, while
    /// row targets reject blocks crossing their exclusive band.
    ///
    /// # Errors
    /// Returns [`ReconError`] for an absent plane, an adjusted source length
    /// other than `min(width, 32) * min(height, 32)`, invalid target geometry,
    /// cross-band access, or an existing prediction sample outside the active
    /// bit depth.
    #[inline]
    pub fn add_adjusted_residual_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        residual: &[i32],
    ) -> Result<()> {
        if size.log2_width() < 6 && size.log2_height() < 6 {
            return self.add_residual_rect_block(plane, x, y, size, residual);
        }
        let width_shift = usize::from(size.log2_width() == 6);
        let height_shift = usize::from(size.log2_height() == 6);
        let adjusted_width = size.width() >> width_shift;
        let adjusted_height = size.height() >> height_shift;
        let expected = adjusted_width * adjusted_height;
        if residual.len() != expected {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane,
                expected,
                actual: residual.len(),
            });
        }
        let rect = block_rect(x, y, size)?;
        let source_stride = size.width();
        self.residual_rect_target(plane, rect, source_stride)?
            .add(|row, column| {
                residual[(row >> height_shift) * adjusted_width + (column >> width_shift)]
            })
    }

    /// Adds one constant signed residual directly to a rectangular block.
    ///
    /// Target geometry and every prediction sample are validated before the
    /// first sample is changed. Frame-edge overhang is clipped, while row
    /// targets reject blocks crossing their exclusive band.
    ///
    /// # Errors
    /// Returns [`ReconError`] for an absent plane, invalid target geometry,
    /// cross-band access, or an existing prediction sample outside the active
    /// bit depth.
    #[inline]
    pub fn add_constant_residual_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        residual: i32,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        self.residual_rect_target(plane, rect, size.width())?
            .add(|_, _| residual)
    }

    #[inline]
    fn residual_rect_target<'borrow>(
        &'borrow mut self,
        plane: PlaneId,
        rect: PlaneRect,
        source_stride: usize,
    ) -> Result<CurrentFrameResidualTarget<'borrow, 'storage, T>> {
        let max_sample = self.info().bit_depth().max_sample();
        let (target, target_stride, target_base, rect) = match self {
            Self::Frame(workspace) => {
                let target = workspace.plane_mut(plane)?;
                let rect = target.clamp_rect_to_storage(rect)?;
                let stride = target.stride_samples;
                let base = rect
                    .y()
                    .checked_mul(stride)
                    .and_then(|start| start.checked_add(rect.x()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "current-frame residual target row offset",
                    })?;
                (&mut target.samples[..], stride, base, rect)
            }
            Self::Row(row) => {
                let target = row.plane_mut(plane)?;
                let rect = clamp_rect_to_storage(target.plane, target.storage_size, rect)?;
                target.ensure_rect(rect)?;
                let stride = target.stride_samples;
                let base = (rect.y() - target.rect.y())
                    .checked_mul(stride)
                    .and_then(|start| start.checked_add(rect.x() - target.rect.x()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "current-frame residual target row offset",
                    })?;
                (&mut *target.samples, stride, base, rect)
            }
            Self::Rect(surface) => {
                let target = surface.plane_mut(plane)?;
                let rect = clamp_rect_to_storage(target.plane, target.storage_size, rect)?;
                target.ensure_rect(rect)?;
                let row_start = rect.y() - target.rect.y();
                let x = rect.x() - target.rect.x();
                let rows = &mut target.rows[row_start..row_start + rect.height()];
                for (row, samples) in rows.iter().enumerate() {
                    for (column, sample) in samples[x..x + rect.width()].iter().enumerate() {
                        let value = sample.to_u16();
                        if value > max_sample {
                            return Err(ReconError::ReconstructPredictionOutOfRange {
                                sample_index: row * source_stride + column,
                                value,
                                max: max_sample,
                            });
                        }
                    }
                }
                return Ok(CurrentFrameResidualTarget::Sliced {
                    rows,
                    x,
                    rect,
                    max_sample,
                });
            }
        };
        let last_target_end = (rect.height() - 1)
            .checked_mul(target_stride)
            .and_then(|offset| target_base.checked_add(offset))
            .and_then(|start| start.checked_add(rect.width()))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame residual target sample span",
            })?;
        if last_target_end > target.len() {
            return Err(ReconError::BufferLengthMismatch {
                expected: last_target_end,
                actual: target.len(),
            });
        }
        for row_index in 0..rect.height() {
            let target_start = target_base + row_index * target_stride;
            for (column, sample) in target[target_start..target_start + rect.width()]
                .iter()
                .enumerate()
            {
                let value = sample.to_u16();
                if value > max_sample {
                    return Err(ReconError::ReconstructPredictionOutOfRange {
                        sample_index: row_index * source_stride + column,
                        value,
                        max: max_sample,
                    });
                }
            }
        }
        Ok(CurrentFrameResidualTarget::Contiguous {
            samples: target,
            stride: target_stride,
            base: target_base,
            rect,
            max_sample,
        })
    }
}

/// Allocation-free iterator over exclusive full-width superblock-row bands.
#[derive(Debug)]
pub struct CurrentFrameRowBands<'a, T: ReconSample> {
    info: DecodedFrameInfo,
    y: CurrentFramePlaneRowBands<'a, T>,
    u: Option<CurrentFramePlaneRowBands<'a, T>>,
    v: Option<CurrentFramePlaneRowBands<'a, T>>,
}

impl<'a, T: ReconSample> Iterator for CurrentFrameRowBands<'a, T> {
    type Item = CurrentFrameRowBand<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let y = self.y.next()?;
        let u = self.u.as_mut().and_then(Iterator::next);
        let v = self.v.as_mut().and_then(Iterator::next);
        Some(CurrentFrameRowBand {
            info: self.info,
            y,
            u,
            v,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.y.size_hint()
    }
}

impl<T: ReconSample> ExactSizeIterator for CurrentFrameRowBands<'_, T> {}

#[derive(Debug)]
struct CurrentFramePlaneRowBands<'a, T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    stride_samples: usize,
    rows_per_band: usize,
    next_y: usize,
    rest: Option<&'a mut [T]>,
}

impl<'a, T: ReconSample> CurrentFramePlaneRowBands<'a, T> {
    fn new(plane: &'a mut CurrentFramePlane<T>, rows_per_band: usize) -> Self {
        Self {
            plane: plane.plane,
            storage_size: plane.storage_size,
            stride_samples: plane.stride_samples,
            rows_per_band,
            next_y: 0,
            rest: Some(&mut plane.samples),
        }
    }
}

impl<'a, T: ReconSample> Iterator for CurrentFramePlaneRowBands<'a, T> {
    type Item = CurrentFramePlaneRowBand<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.rest.take()?;
        if rest.is_empty() {
            return None;
        }
        let remaining_rows = rest.len() / self.stride_samples;
        let rows = remaining_rows.min(self.rows_per_band);
        let samples = rows * self.stride_samples;
        let (band, tail) = rest.split_at_mut(samples);
        self.rest = Some(tail);
        let rect = PlaneRect::new(0, self.next_y, self.storage_size.width(), rows).ok()?;
        self.next_y += rows;
        Some(CurrentFramePlaneRowBand {
            plane: self.plane,
            storage_size: self.storage_size,
            stride_samples: self.stride_samples,
            rect,
            samples: band,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rows = self
            .rest
            .as_deref()
            .map_or(0, |rest| rest.len() / self.stride_samples);
        let remaining = rows.div_ceil(self.rows_per_band);
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for CurrentFramePlaneRowBands<'_, T> {}

fn partition_plane_rects<'a, T: ReconSample>(
    plane: &'a mut CurrentFramePlane<T>,
    rects: &[PlaneRect],
) -> Result<Vec<CurrentFramePlaneRect<'a, T>>> {
    let mut surfaces = Vec::new();
    surfaces
        .try_reserve_exact(rects.len())
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: plane.plane,
            context: "rectangle surface descriptors",
        })?;
    for &rect in rects {
        ensure_rect_in_storage(plane.plane, plane.storage_size, rect)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(rect.height()).map_err(|_| {
            ReconError::WorkspaceAllocationFailed {
                plane: plane.plane,
                context: "rectangle surface rows",
            }
        })?;
        surfaces.push(CurrentFramePlaneRect {
            plane: plane.plane,
            storage_size: plane.storage_size,
            rect,
            rows,
        });
    }
    let mut order = Vec::new();
    order
        .try_reserve_exact(rects.len())
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: plane.plane,
            context: "rectangle surface ordering",
        })?;
    order.extend(0..rects.len());
    order.sort_unstable_by_key(|&index| (rects[index].x(), rects[index].y()));

    for (y, row) in plane
        .samples
        .chunks_exact_mut(plane.stride_samples)
        .enumerate()
    {
        let mut rest = row;
        let mut consumed = 0usize;
        for &index in &order {
            let rect = rects[index];
            if y < rect.y() || y >= rect.y() + rect.height() {
                continue;
            }
            let skip = rect.x() - consumed;
            let (_, after_skip) = rest.split_at_mut(skip);
            let (surface_row, after_surface) = after_skip.split_at_mut(rect.width());
            surfaces[index].rows.push(surface_row);
            rest = after_surface;
            consumed = rect.x() + rect.width();
        }
    }
    Ok(surfaces)
}

fn subsampled_rects(
    rects: &[PlaneRect],
    shift_x: u8,
    shift_y: u8,
    storage: PlaneSize,
) -> Result<Vec<PlaneRect>> {
    let scale_x = 1usize << shift_x;
    let scale_y = 1usize << shift_y;
    let mut output = Vec::new();
    output
        .try_reserve_exact(rects.len())
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::U,
            context: "subsampled rectangle descriptors",
        })?;
    for rect in rects {
        let right = rect
            .x()
            .checked_add(rect.width())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "rectangle surface chroma right edge",
            })?;
        let bottom = rect
            .y()
            .checked_add(rect.height())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "rectangle surface chroma bottom edge",
            })?;
        let x = rect.x() / scale_x;
        let y = rect.y() / scale_y;
        let right = right.div_ceil(scale_x).min(storage.width());
        let bottom = bottom.div_ceil(scale_y).min(storage.height());
        output.push(PlaneRect::new(x, y, right - x, bottom - y)?);
    }
    validate_disjoint_rects(&output)?;
    Ok(output)
}

fn validate_disjoint_rects(rects: &[PlaneRect]) -> Result<()> {
    for (index, &first) in rects.iter().enumerate() {
        for &second in &rects[index + 1..] {
            if rects_overlap(first, second) {
                return Err(ReconError::WorkspaceRectSurfacesOverlap { first, second });
            }
        }
    }
    Ok(())
}

const fn rects_overlap(first: PlaneRect, second: PlaneRect) -> bool {
    first.x() < second.x().saturating_add(second.width())
        && second.x() < first.x().saturating_add(first.width())
        && first.y() < second.y().saturating_add(second.height())
        && second.y() < first.y().saturating_add(first.height())
}

const MAX_INTRA_PREDICTION_SAMPLES: usize = 64 * 64;

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Creates a workspace from decoded-frame metadata and an initial fill.
    ///
    /// Plane geometry is derived from [`DecodedFrameInfo`]. Y storage uses the
    /// coded luma size, chroma storage is derived from AV2 §6.4.1 subsampling,
    /// and monochrome workspaces allocate only Y.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample type cannot represent the frame bit
    /// depth, the fill sample exceeds the active bit depth, geometry arithmetic
    /// overflows, or plane allocation fails.
    pub fn new(info: DecodedFrameInfo, fill: T) -> Result<Self> {
        validate_sample_type::<T>(info.bit_depth())?;
        validate_sample_value(PlaneId::Y, 0, fill, info.bit_depth().max_sample())?;

        let y = CurrentFramePlane::new(
            PlaneId::Y,
            info.coded_luma_size(),
            info.visible_luma_rect(),
            fill,
        )?;

        let (u, v) = match chroma_plane_geometry(
            info.pixel_format(),
            info.coded_luma_size(),
            info.visible_luma_rect(),
        )? {
            None => (None, None),
            Some((storage_size, visible_rect)) => (
                Some(CurrentFramePlane::new(
                    PlaneId::U,
                    storage_size,
                    visible_rect,
                    fill,
                )?),
                Some(CurrentFramePlane::new(
                    PlaneId::V,
                    storage_size,
                    visible_rect,
                    fill,
                )?),
            ),
        };

        Ok(Self {
            info,
            y,
            u,
            v,
            intra_prediction_scratch: IntraPredictionScratch::new(),
        })
    }

    /// Returns the decoded-frame metadata used to construct the workspace.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns an immutable workspace plane by identifier.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] for absent chroma planes in
    /// monochrome workspaces.
    #[inline]
    pub fn plane(&self, plane: PlaneId) -> Result<&CurrentFramePlane<T>> {
        select_plane(plane, &self.y, self.u.as_ref(), self.v.as_ref())
    }

    /// Borrows the workspace as an immutable [`FrameRef`] without copying.
    pub fn as_frame_ref(&self) -> FrameRef<'_, T> {
        FrameRef::from_parts(
            self.info,
            self.y.as_plane_ref(),
            self.u.as_ref().map(CurrentFramePlane::as_plane_ref),
            self.v.as_ref().map(CurrentFramePlane::as_plane_ref),
        )
    }

    /// Borrows the workspace as an exclusive [`FrameMut`] without copying.
    ///
    /// The Y/U/V planes are distinct fields, so the three exclusive plane views
    /// borrow disjoint storage and may be written independently.
    pub fn as_frame_mut(&mut self) -> FrameMut<'_, T> {
        FrameMut::from_parts(
            self.info,
            self.y.as_plane_mut(),
            self.u.as_mut().map(CurrentFramePlane::as_plane_mut),
            self.v.as_mut().map(CurrentFramePlane::as_plane_mut),
        )
    }

    /// Partitions all frame planes into exclusive full-width superblock rows.
    ///
    /// Luma bands are 64, 128, or 256 rows according to `sb_size`. Chroma band
    /// heights follow the frame's vertical subsampling, and the final band on
    /// each plane is clipped to its storage height. The returned plane slices
    /// borrow the existing workspace storage without copying pixels.
    pub fn sb_row_bands(&mut self, sb_size: SuperblockSize) -> CurrentFrameRowBands<'_, T> {
        let luma_rows = superblock_side(sb_size);
        let chroma_rows = luma_rows >> self.info.pixel_format().subsampling_y();
        CurrentFrameRowBands {
            info: self.info,
            y: CurrentFramePlaneRowBands::new(&mut self.y, luma_rows),
            u: self
                .u
                .as_mut()
                .map(|plane| CurrentFramePlaneRowBands::new(plane, chroma_rows)),
            v: self
                .v
                .as_mut()
                .map(|plane| CurrentFramePlaneRowBands::new(plane, chroma_rows)),
        }
    }

    /// Partitions requested disjoint luma rectangles into exclusive Y/U/V surfaces.
    ///
    /// Each returned plane owns one mutable slice per row, so adjacent column
    /// tiles can be sent to different workers without aliasing. Chroma bounds
    /// are derived from the frame subsampling and clipped to coded storage.
    ///
    /// # Errors
    /// Returns [`ReconError`] when a rectangle exceeds luma storage, any luma
    /// or derived chroma rectangles overlap, geometry overflows, or descriptor
    /// allocation fails.
    pub fn rect_surfaces(
        &mut self,
        luma_rects: &[PlaneRect],
    ) -> Result<Vec<CurrentFrameRect<'_, T>>> {
        let info = self.info;
        validate_disjoint_rects(luma_rects)?;
        for &rect in luma_rects {
            ensure_rect_in_storage(PlaneId::Y, self.y.storage_size, rect)?;
        }
        let pixel_format = self.info.pixel_format();
        let chroma_rects = self.u.as_ref().map(|plane| {
            subsampled_rects(
                luma_rects,
                pixel_format.subsampling_x(),
                pixel_format.subsampling_y(),
                plane.storage_size,
            )
        });
        let chroma_rects = chroma_rects.transpose()?;
        let y = partition_plane_rects(&mut self.y, luma_rects)?;
        let u = match (&mut self.u, chroma_rects.as_deref()) {
            (Some(plane), Some(rects)) => Some(partition_plane_rects(plane, rects)?),
            _ => None,
        };
        let v = match (&mut self.v, chroma_rects.as_deref()) {
            (Some(plane), Some(rects)) => Some(partition_plane_rects(plane, rects)?),
            _ => None,
        };
        let mut u = u.map(Vec::into_iter);
        let mut v = v.map(Vec::into_iter);
        let mut output = Vec::new();
        output
            .try_reserve_exact(y.len())
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "rectangle surfaces",
            })?;
        for y in y {
            output.push(CurrentFrameRect {
                info,
                y,
                u: u.as_mut().and_then(Iterator::next),
                v: v.as_mut().and_then(Iterator::next),
            });
        }
        Ok(output)
    }

    /// Returns all backing samples for `plane`, including padding if present.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] for absent chroma planes in
    /// monochrome workspaces.
    pub fn samples(&self, plane: PlaneId) -> Result<&[T]> {
        Ok(self.plane(plane)?.samples())
    }

    /// Returns the single already-reconstructed sample at `(x, y)` in `plane`.
    ///
    /// This is the checked reader a caller needs when the AV2 §7.13.2.1 edge
    /// preparation must read a specific reconstructed neighbour sample (for
    /// example the top-right sentinel `CurrFrame[plane][y-1][Min(aboveLimit,
    /// x+w)]`) that does not lie on the block's immediate above/left edge. It
    /// does not decide AV2 availability — the caller is responsible for only
    /// reading samples that the §5.20.2.3 `BlockDecoded` state marks decoded.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent or `(x, y)` falls outside
    /// the plane storage.
    #[inline]
    pub fn reconstructed_sample(&self, plane: PlaneId, x: usize, y: usize) -> Result<T> {
        self.plane(plane)?.reconstructed_sample(x, y)
    }

    /// Iterates over a checked rectangular region in `plane`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent or `rect` falls outside
    /// the plane storage.
    pub fn rect_rows(&self, plane: PlaneId, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        self.plane(plane)?.rect_rows(rect)
    }

    /// Writes a single already-reconstructed sample at `(x, y)` in `plane`.
    ///
    /// This is the checked single-sample writer the AV2 § 7.17 deblocking edge
    /// loop needs: it gathers a perpendicular sample line, filters it, and writes
    /// the modified samples back into the workspace across block boundaries. It
    /// does not decide AV2 edge availability — the caller is responsible for only
    /// writing samples the deblocking edge selection permits.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, `(x, y)` falls outside the
    /// plane storage, or `value` exceeds the active bit depth.
    pub fn set_reconstructed_sample(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        value: T,
    ) -> Result<()> {
        self.fill_rect(plane, PlaneRect::new(x, y, 1, 1)?, value)
    }

    /// Fills a checked rectangular region in `plane` with `sample`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the rectangle falls
    /// outside storage, or `sample` exceeds the active bit depth.
    pub fn fill_rect(&mut self, plane: PlaneId, rect: PlaneRect, sample: T) -> Result<()> {
        let max_sample = self.info.bit_depth().max_sample();
        let target = self.plane_mut(plane)?;
        validate_sample_value(plane, 0, sample, max_sample)?;
        target.fill_rect(rect, sample)
    }

    /// Writes a checked rectangular region in `plane` from row-strided samples.
    ///
    /// `samples` points at the first source row and `row_stride_samples` is the
    /// distance between adjacent source rows. Source padding after each row is
    /// allowed and ignored.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle is
    /// out of bounds, the source stride/buffer is too small, or a source sample
    /// exceeds the active bit depth.
    pub fn write_rect(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
    ) -> Result<()> {
        let max = self.info.bit_depth().max_sample();
        self.plane_mut(plane)?
            .write_rect(rect, samples, row_stride_samples, max)
    }

    /// Copies an already reconstructed source rectangle within one workspace plane.
    ///
    /// The source rectangle is snapped into scratch storage before target writes,
    /// so overlapping copies read the original samples. Fractional IntrABC
    /// prediction needs a separate convolution path.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, either rectangle is out of
    /// bounds, source and target shapes differ, geometry arithmetic overflows, or
    /// scratch allocation fails.
    pub fn copy_rect_within_plane(
        &mut self,
        plane: PlaneId,
        source: PlaneRect,
        target: PlaneRect,
    ) -> Result<()> {
        self.copy_rect_within_plane_into(plane, source, target, &mut Vec::new())
    }

    /// Copies a workspace rectangle using caller-owned reusable scratch storage.
    ///
    /// # Errors
    /// Returns the same errors as [`Self::copy_rect_within_plane`].
    pub fn copy_rect_within_plane_into(
        &mut self,
        plane: PlaneId,
        source: PlaneRect,
        target: PlaneRect,
        scratch: &mut Vec<T>,
    ) -> Result<()> {
        let source_plane = self.plane(plane)?;
        source_plane.ensure_rect(source)?;
        source_plane.ensure_rect(target)?;
        if source.size() != target.size() {
            return Err(ReconError::WorkspaceCopyShapeMismatch {
                plane,
                source_rect: source,
                target_rect: target,
            });
        }

        let sample_count =
            source
                .width()
                .checked_mul(source.height())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "current-frame workspace copy sample count",
                })?;
        scratch.clear();
        scratch.try_reserve_exact(sample_count).map_err(|_| {
            ReconError::WorkspaceAllocationFailed {
                plane,
                context: "copy scratch",
            }
        })?;
        for row in source_plane.rect_rows(source)? {
            // splot-copy-ok: IntrABC copy snapshots the bounded source rectangle before overlapping target writes.
            scratch.extend_from_slice(row);
        }

        self.write_rect(plane, target, scratch, source.width())
    }

    /// Writes a contiguous square prediction block into `plane`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target square is out
    /// of bounds, `samples.len()` is not the square sample count, or a source
    /// sample exceeds the active bit depth.
    pub fn write_square_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
        samples: &[T],
    ) -> Result<()> {
        self.write_rect_block(plane, x, y, size.into(), samples)
    }

    /// Writes a contiguous rectangular prediction block into `plane`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle is
    /// out of bounds, `samples.len()` is not the rectangular sample count, or a
    /// source sample exceeds the active bit depth.
    pub fn write_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        samples: &[T],
    ) -> Result<()> {
        let rect = checked_sample_block_rect(plane, x, y, size, samples.len())?;
        self.write_rect(plane, rect, samples, size.width())
    }

    /// Predicts rectangular basic/PAETH intra samples into the workspace.
    ///
    /// Uses adjacent in-storage samples as the prepared AV2 §7.13.2.2 edges.
    /// Edge synthesis and availability remain caller-owned.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes, or
    /// missing in-storage top/left neighbors.
    pub fn predict_intra_paeth_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        self.plane_mut(plane)?.predict_intra_paeth_rect(rect)
    }

    /// Predicts rectangular smooth intra samples into the workspace.
    ///
    /// Uses adjacent in-storage samples as the prepared AV2 §7.13.2.13 edges.
    /// Edge synthesis and availability remain caller-owned.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes, or
    /// missing in-storage prepared edges.
    pub fn predict_intra_smooth_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        mode: IntraSmoothMode,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_smooth_rect(rect, size, mode, bit_depth)
    }

    /// Predicts rectangular cardinal directional intra samples into the workspace.
    ///
    /// Uses the in-storage above edge for vertical prediction and the in-storage
    /// left edge for horizontal prediction. Edge synthesis remains caller-owned.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes, or
    /// missing in-storage prepared edges.
    pub fn predict_intra_cardinal_directional_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        direction: IntraCardinalDirection,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        self.plane_mut(plane)?
            .predict_intra_cardinal_directional_rect(rect, size, direction, bit_depth)
    }

    /// Takes a reusable, initialized intra-prediction buffer from this workspace.
    ///
    /// The two named slots let dual-prediction modes hold both inputs at once.
    /// Callers return storage with [`Self::recycle_intra_prediction_buffer`].
    ///
    /// # Errors
    /// Returns [`ReconError`] if `sample_count` exceeds the largest AV2 intra
    /// block or the buffer cannot reserve enough storage.
    pub fn take_intra_prediction_buffer(
        &mut self,
        slot: IntraPredictionScratchBuffer,
        plane: PlaneId,
        sample_count: usize,
        fill: T,
    ) -> Result<Vec<T>> {
        self.intra_prediction_scratch
            .take_buffer(slot, plane, sample_count, fill)
    }

    /// Returns an intra-prediction buffer to its reusable workspace slot.
    pub fn recycle_intra_prediction_buffer(
        &mut self,
        slot: IntraPredictionScratchBuffer,
        buffer: Vec<T>,
    ) {
        self.intra_prediction_scratch.recycle_buffer(slot, buffer);
    }

    /// Exchanges reusable intra-prediction storage with a reconstruction task.
    pub fn swap_intra_prediction_scratch(&mut self, scratch: &mut IntraPredictionScratch<T>) {
        mem::swap(&mut self.intra_prediction_scratch, scratch);
    }

    /// Freezes the workspace into an immutable decoded frame.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the existing immutable plane/frame validators
    /// reject the workspace storage.
    pub fn freeze(self) -> Result<DecodedFrame<T>> {
        let y = self.y.freeze()?;
        let u = self.u.map(CurrentFramePlane::freeze).transpose()?;
        let v = self.v.map(CurrentFramePlane::freeze).transpose()?;
        DecodedFrame::try_new(self.info, FramePlanes::new(y, u, v))
    }

    /// Consumes a transient (non-frozen) workspace, returning its plane sample
    /// buffers to the process-global retained pool so the next frame's
    /// reconstruction workspace can reuse them instead of reallocating.
    ///
    /// Call this only on the reconstruction workspace once filtering has read
    /// all of it; the filtered output workspace is [`freeze`](Self::freeze)d
    /// into a decoded frame and must not be recycled.
    pub fn recycle_planes(self) {
        recycle_recon_plane_buffer(self.y.into_samples());
        if let Some(u) = self.u {
            recycle_recon_plane_buffer(u.into_samples());
        }
        if let Some(v) = self.v {
            recycle_recon_plane_buffer(v.into_samples());
        }
    }

    fn plane_mut(&mut self, plane: PlaneId) -> Result<&mut CurrentFramePlane<T>> {
        select_plane_mut(plane, &mut self.y, self.u.as_mut(), self.v.as_mut())
    }
}

/// Mutable backing storage for one current-frame workspace plane.
///
/// Bounds retained transient reconstruction-plane buffers: at most this many,
/// and only those whose capacity fits a ~4K single-plane frame, so a large
/// frame cannot pin its high-water allocation in the process-global pool until
/// process exit. Retained memory tracks typical frames rather than the largest
/// ever seen.
const MAX_RETAINED_RECON_PLANE_BUFFERS: usize = 6;
const MAX_RETAINED_RECON_PLANE_SAMPLES: usize = 1 << 24;

/// Takes a cleared plane sample buffer from the per-type retained pool, reusing
/// a prior frame's recycled reconstruction-workspace allocation when available.
fn take_recon_plane_buffer<T: ReconSample>() -> Vec<T> {
    let mut buffer = T::recon_plane_pool()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default();
    buffer.clear();
    buffer
}

/// Returns a plane sample buffer to the per-type retained pool, dropping it when
/// the pool is full or the buffer is oversized.
fn recycle_recon_plane_buffer<T: ReconSample>(buffer: Vec<T>) {
    if buffer.capacity() == 0 || buffer.capacity() > MAX_RETAINED_RECON_PLANE_SAMPLES {
        return;
    }
    let mut pool = T::recon_plane_pool()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pool.len() < MAX_RETAINED_RECON_PLANE_BUFFERS {
        pool.push(buffer);
    }
}

/// Does not implement `Clone`: it owns the plane sample buffer (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Eq, PartialEq)]
pub struct CurrentFramePlane<T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    stride_samples: usize,
    visible_rect: PlaneRect,
    required_samples: usize,
    allocation_bytes: usize,
    samples: Vec<T>,
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn new(
        plane: PlaneId,
        storage_size: PlaneSize,
        visible_rect: PlaneRect,
        fill: T,
    ) -> Result<Self> {
        visible_rect.ensure_within(storage_size).map_err(|_| {
            ReconError::WorkspaceRectOutOfBounds {
                plane,
                storage: storage_size,
                rect: visible_rect,
            }
        })?;

        let stride_samples = storage_size.width();
        let required_samples = stride_samples.checked_mul(storage_size.height()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "current-frame workspace plane required sample count",
            },
        )?;
        let allocation_bytes = required_samples.checked_mul(mem::size_of::<T>()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "current-frame workspace plane allocation byte count",
            },
        )?;

        let mut samples = take_recon_plane_buffer::<T>();
        samples.try_reserve_exact(required_samples).map_err(|_| {
            ReconError::WorkspaceAllocationFailed {
                plane,
                context: "sample buffer",
            }
        })?;
        samples.resize(required_samples, fill);

        Ok(Self {
            plane,
            storage_size,
            stride_samples,
            visible_rect,
            required_samples,
            allocation_bytes,
            samples,
        })
    }

    /// Returns the plane identifier.
    pub const fn plane_id(&self) -> PlaneId {
        self.plane
    }

    /// Returns the full storage dimensions in samples.
    pub const fn storage_size(&self) -> PlaneSize {
        self.storage_size
    }

    /// Returns the storage stride in samples.
    pub const fn stride_samples(&self) -> usize {
        self.stride_samples
    }

    /// Returns the visible decoded-output rectangle.
    pub const fn visible_rect(&self) -> PlaneRect {
        self.visible_rect
    }

    /// Returns the required backing sample count.
    pub const fn required_samples(&self) -> usize {
        self.required_samples
    }

    /// Returns the backing allocation size in bytes for the sample type.
    pub const fn allocation_bytes(&self) -> usize {
        self.allocation_bytes
    }

    /// Returns all backing samples for this plane.
    pub fn samples(&self) -> &[T] {
        &self.samples
    }

    /// Returns the already-reconstructed sample at `(x, y)` in this plane.
    /// The column is validated against the storage width (not just the flat
    /// index) so a column at or past the row stride is rejected instead of
    /// aliasing into the next row.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `(x, y)` falls
    /// outside the plane storage.
    #[inline]
    pub fn reconstructed_sample(&self, x: usize, y: usize) -> Result<T> {
        if x >= self.storage_size.width() || y >= self.storage_size.height() {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect: PlaneRect::new(x, y, 1, 1)?,
            });
        }
        let index = self.sample_index(x, y)?;
        Ok(self.samples[index])
    }

    /// Borrows this plane's storage as an immutable [`PlaneRef`] without copying.
    pub fn as_plane_ref(&self) -> PlaneRef<'_, T> {
        PlaneRef::from_parts(&self.samples, self.stride_samples, self.visible_rect)
    }

    /// Borrows this plane's storage as an exclusive [`PlaneMut`] without copying.
    pub fn as_plane_mut(&mut self) -> PlaneMut<'_, T> {
        PlaneMut::from_parts(&mut self.samples, self.stride_samples, self.visible_rect)
    }

    /// Iterates over a checked rectangular region in this plane.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `rect` falls
    /// outside the plane storage.
    pub fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        self.ensure_rect(rect)?;
        Ok(WorkspaceRectRows::Strided(
            PlaneRef::from_parts(&self.samples, self.stride_samples, rect).visible_rows(),
        ))
    }

    fn fill_rect(&mut self, rect: PlaneRect, sample: T) -> Result<()> {
        // Frame-edge fills use the same in-frame clamp as `write_rect`.
        let rect = self.clamp_rect_to_storage(rect)?;
        for row in rect.y()..rect.y() + rect.height() {
            let range = self.row_range(row, rect.x(), rect.width())?;
            self.samples[range].fill(sample);
        }
        Ok(())
    }

    /// One bounds proof covers every row: the clamped rect's first and last
    /// target rows both index in-storage and rows advance by the plane
    /// stride, so per-row range math cannot fail after the up-front checks.
    fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
        max_sample: u16,
    ) -> Result<()> {
        // Reconstruction writes only in-frame samples. Partial frame-edge
        // overhang is dropped; an out-of-frame origin remains an error.
        let rect = self.clamp_rect_to_storage(rect)?;
        write_rect_to_samples(
            self.plane,
            &mut self.samples,
            self.stride_samples,
            rect,
            rect.x(),
            rect.y(),
            samples,
            row_stride_samples,
            max_sample,
        )
    }

    fn predict_intra_paeth_rect(&mut self, rect: PlaneRect) -> Result<()> {
        self.ensure_rect(rect)?;
        if rect.x() == 0 {
            return Err(ReconError::WorkspaceIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraPaethEdge::Left,
                rect,
            });
        }
        if rect.y() == 0 {
            return Err(ReconError::WorkspaceIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraPaethEdge::Above,
                rect,
            });
        }

        let top_left = self.samples[self.sample_index(rect.x() - 1, rect.y() - 1)?];
        let above_range = self.row_range(rect.y() - 1, rect.x(), rect.width())?;
        for row_index in 0..rect.height() {
            let row = rect.y() + row_index;
            let left = self.samples[self.sample_index(rect.x() - 1, row)?];
            let target_range = self.row_range(row, rect.x(), rect.width())?;
            for column in 0..rect.width() {
                let above = self.samples[above_range.start + column];
                self.samples[target_range.start + column] =
                    predict_paeth_sample(left, above, top_left);
            }
        }

        Ok(())
    }

    fn predict_intra_smooth_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        mode: IntraSmoothMode,
        bit_depth: crate::BitDepth,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        if rect.x() == 0 {
            return Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraSmoothEdge::Left,
                rect,
            });
        }
        if rect.y() == 0 {
            return Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraSmoothEdge::Above,
                rect,
            });
        }
        let bottom_left_y =
            rect.y()
                .checked_add(rect.height())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction bottom-left row",
                })?;
        if bottom_left_y >= self.storage_size.height() {
            return Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraSmoothEdge::BottomLeft,
                rect,
            });
        }
        let top_right_x =
            rect.x()
                .checked_add(rect.width())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction top-right column",
                })?;
        if top_right_x >= self.storage_size.width() {
            return Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
                plane: self.plane,
                edge: IntraSmoothEdge::TopRight,
                rect,
            });
        }

        let above_range = self.row_range(rect.y() - 1, rect.x(), rect.width() + 1)?;
        let bottom_left = self.samples[self.sample_index(rect.x() - 1, bottom_left_y)?];
        let top_right = self.samples[above_range.start + rect.width()];

        for row_index in 0..rect.height() {
            let row = rect.y() + row_index;
            let left = self.samples[self.sample_index(rect.x() - 1, row)?];
            let target_range = self.row_range(row, rect.x(), rect.width())?;
            for column in 0..rect.width() {
                let top = self.samples[above_range.start + column];
                let sample = predict_smooth_sample_values(
                    bit_depth,
                    size,
                    mode,
                    SmoothSampleEdges {
                        left,
                        top,
                        bottom_left,
                        top_right,
                    },
                    SmoothSamplePosition {
                        row: row_index,
                        column,
                    },
                )?;
                self.samples[target_range.start + column] = sample;
            }
        }

        Ok(())
    }

    fn predict_intra_cardinal_directional_rect(
        &mut self,
        rect: PlaneRect,
        size: IntraRectBlockSize,
        direction: IntraCardinalDirection,
        bit_depth: crate::BitDepth,
    ) -> Result<()> {
        self.ensure_rect(rect)?;

        let output_start = self.sample_index(rect.x(), rect.y())?;
        let edge_kind = direction.required_edge();
        let edge_len = match edge_kind {
            IntraDirectionalAngleEdge::Above => rect.width(),
            IntraDirectionalAngleEdge::Left => rect.height(),
        };
        let edge = self.directional_angle_edge_samples(
            rect,
            edge_kind,
            edge_len,
            direction.p_angle(),
            cardinal_edge_context(edge_kind),
        )?;
        let edges = workspace_edges::directional_angle_edges(edge_kind, &edge);
        predict_intra_cardinal_directional_rect_into(
            bit_depth,
            size,
            direction,
            edges,
            &mut self.samples[output_start..],
            self.stride_samples,
        )
    }

    fn freeze(self) -> Result<Plane<T>> {
        Plane::from_vec(
            self.storage_size,
            self.stride_samples,
            self.visible_rect,
            self.samples,
        )
    }

    /// Consumes the plane, returning its backing sample buffer for recycling.
    fn into_samples(self) -> Vec<T> {
        self.samples
    }

    /// Clamps a write/fill `rect` to the in-frame storage extent.
    ///
    /// Models AVM's in-frame-only reconstruction: frame-edge overhang is dropped,
    /// while a rectangle whose origin is already out of frame still errors.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `rect` starts outside
    /// storage.
    fn clamp_rect_to_storage(&self, rect: PlaneRect) -> Result<PlaneRect> {
        clamp_rect_to_storage(self.plane, self.storage_size, rect)
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)
    }

    #[inline]
    fn sample_index(&self, x: usize, y: usize) -> Result<usize> {
        let row_start =
            y.checked_mul(self.stride_samples)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "current-frame workspace row offset",
                })?;
        let index = row_start
            .checked_add(x)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame workspace sample index",
            })?;
        if index < self.samples.len() {
            Ok(index)
        } else {
            Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect: PlaneRect::new(x, y, 1, 1)?,
            })
        }
    }

    fn row_range(&self, row: usize, x: usize, width: usize) -> Result<Range<usize>> {
        let start = self.sample_index(x, row)?;
        let end = start
            .checked_add(width)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame workspace row range",
            })?;
        if end <= self.samples.len() {
            Ok(start..end)
        } else {
            Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect: PlaneRect::new(x, row, width, 1)?,
            })
        }
    }
}

const fn cardinal_edge_context(edge: IntraDirectionalAngleEdge) -> &'static str {
    match edge {
        IntraDirectionalAngleEdge::Above => "cardinal directional above edge",
        IntraDirectionalAngleEdge::Left => "cardinal directional left edge",
    }
}

fn validate_sample_value<T: ReconSample>(
    plane: PlaneId,
    sample_index: usize,
    sample: T,
    max: u16,
) -> Result<()> {
    let value = sample.to_u16();
    if value > max {
        Err(ReconError::SampleOutOfRange {
            plane,
            sample_index,
            value,
            max,
        })
    } else {
        Ok(())
    }
}

fn chroma_plane_geometry(
    pixel_format: PixelFormat,
    coded_luma_size: PlaneSize,
    visible_luma_rect: PlaneRect,
) -> Result<Option<(PlaneSize, PlaneRect)>> {
    let Some(storage_size) = pixel_format.chroma_size(coded_luma_size)? else {
        return Ok(None);
    };
    let Some(visible_size) = pixel_format.chroma_size(visible_luma_rect.size())? else {
        return Ok(None);
    };

    let x = visible_luma_rect.x() >> pixel_format.subsampling_x();
    let y = visible_luma_rect.y() >> pixel_format.subsampling_y();
    Ok(Some((
        storage_size,
        PlaneRect::new(x, y, visible_size.width(), visible_size.height())?,
    )))
}

const fn superblock_side(sb_size: SuperblockSize) -> usize {
    match sb_size {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 => 128,
        SuperblockSize::Block256x256 => 256,
    }
}

/// Validates a caller-supplied per-sample buffer against a block's rectangular
/// sample count, then resolves the target rectangle.
fn checked_sample_block_rect(
    plane: PlaneId,
    x: usize,
    y: usize,
    size: IntraRectBlockSize,
    actual: usize,
) -> Result<PlaneRect> {
    if actual != size.sample_count() {
        return Err(ReconError::WorkspaceWriteLengthMismatch {
            plane,
            expected: size.sample_count(),
            actual,
        });
    }
    block_rect(x, y, size)
}

fn block_rect(x: usize, y: usize, size: IntraRectBlockSize) -> Result<PlaneRect> {
    PlaneRect::new(x, y, size.width(), size.height())
}

fn select_plane<'a, P>(
    plane: PlaneId,
    y: &'a P,
    u: Option<&'a P>,
    v: Option<&'a P>,
) -> Result<&'a P> {
    match plane {
        PlaneId::Y => Ok(y),
        PlaneId::U => u.ok_or(ReconError::MissingWorkspacePlane { plane }),
        PlaneId::V => v.ok_or(ReconError::MissingWorkspacePlane { plane }),
    }
}

fn select_plane_mut<'a, P>(
    plane: PlaneId,
    y: &'a mut P,
    u: Option<&'a mut P>,
    v: Option<&'a mut P>,
) -> Result<&'a mut P> {
    match plane {
        PlaneId::Y => Ok(y),
        PlaneId::U => u.ok_or(ReconError::MissingWorkspacePlane { plane }),
        PlaneId::V => v.ok_or(ReconError::MissingWorkspacePlane { plane }),
    }
}

fn clamp_rect_to_storage(plane: PlaneId, storage: PlaneSize, rect: PlaneRect) -> Result<PlaneRect> {
    if rect.x() >= storage.width() || rect.y() >= storage.height() {
        return Err(ReconError::WorkspaceRectOutOfBounds {
            plane,
            storage,
            rect,
        });
    }
    let width = rect.width().min(storage.width() - rect.x());
    let height = rect.height().min(storage.height() - rect.y());
    if width == rect.width() && height == rect.height() {
        Ok(rect)
    } else {
        PlaneRect::new(rect.x(), rect.y(), width, height)
    }
}

fn ensure_rect_in_storage(plane: PlaneId, storage: PlaneSize, rect: PlaneRect) -> Result<()> {
    if rect.is_within(storage) {
        Ok(())
    } else {
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane,
            storage,
            rect,
        })
    }
}

fn rect_is_within(rect: PlaneRect, bounds: PlaneRect) -> bool {
    let Some(x) = rect.x().checked_sub(bounds.x()) else {
        return false;
    };
    let Some(y) = rect.y().checked_sub(bounds.y()) else {
        return false;
    };
    x.checked_add(rect.width())
        .is_some_and(|right| right <= bounds.width())
        && y.checked_add(rect.height())
            .is_some_and(|bottom| bottom <= bounds.height())
}

#[allow(clippy::too_many_arguments)]
fn write_rect_to_samples<T: ReconSample>(
    plane: PlaneId,
    target: &mut [T],
    target_stride_samples: usize,
    rect: PlaneRect,
    local_x: usize,
    local_y: usize,
    samples: &[T],
    row_stride_samples: usize,
    max_sample: u16,
) -> Result<()> {
    validate_write_source(
        plane,
        rect,
        samples,
        row_stride_samples,
        target_stride_samples,
        max_sample,
    )?;

    let target_base = local_y
        .checked_mul(target_stride_samples)
        .and_then(|start| start.checked_add(local_x))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace target row offset",
        })?;
    let last_row_offset = (rect.height() - 1)
        .checked_mul(target_stride_samples)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace target row span",
        })?;
    let last_target_end = target_base
        .checked_add(last_row_offset)
        .and_then(|start| start.checked_add(rect.width()))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace target sample span",
        })?;
    if last_target_end > target.len() {
        return Err(ReconError::BufferLengthMismatch {
            expected: last_target_end,
            actual: target.len(),
        });
    }

    for row_index in 0..rect.height() {
        let source_start = row_index * row_stride_samples;
        let target_start = target_base + row_index * target_stride_samples;
        // splot-copy-ok: write caller samples into exclusive current-frame target storage
        target[target_start..target_start + rect.width()]
            .copy_from_slice(&samples[source_start..source_start + rect.width()]);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_u16_rect_to_samples<T: ReconSample>(
    target: &mut [T],
    target_stride_samples: usize,
    rect: PlaneRect,
    local_x: usize,
    local_y: usize,
    samples: &[u16],
    row_stride_samples: usize,
) -> Result<()> {
    let target_base = local_y
        .checked_mul(target_stride_samples)
        .and_then(|start| start.checked_add(local_x))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame u16 target row offset",
        })?;
    let last_target_end = (rect.height() - 1)
        .checked_mul(target_stride_samples)
        .and_then(|offset| target_base.checked_add(offset))
        .and_then(|start| start.checked_add(rect.width()))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame u16 target sample span",
        })?;
    if last_target_end > target.len() {
        return Err(ReconError::BufferLengthMismatch {
            expected: last_target_end,
            actual: target.len(),
        });
    }
    for row in 0..rect.height() {
        let source_start = row * row_stride_samples;
        let target_start = target_base + row * target_stride_samples;
        copy_u16_samples(
            &mut target[target_start..target_start + rect.width()],
            &samples[source_start..source_start + rect.width()],
        )?;
    }
    Ok(())
}

fn copy_u16_samples<T: ReconSample>(target: &mut [T], samples: &[u16]) -> Result<()> {
    if let Some(target) = T::u16_slice_mut(target) {
        for (target, &sample) in target.iter_mut().zip(samples) {
            *target = sample;
        }
        return Ok(());
    }
    for (target, &sample) in target.iter_mut().zip(samples) {
        *target = T::try_from_u16(sample)?;
    }
    Ok(())
}

fn validate_write_source<T: ReconSample>(
    plane: PlaneId,
    rect: PlaneRect,
    samples: &[T],
    row_stride_samples: usize,
    target_stride_samples: usize,
    max_sample: u16,
) -> Result<()> {
    if row_stride_samples < rect.width() {
        return Err(ReconError::WorkspaceWriteStrideTooSmall {
            plane,
            stride_samples: row_stride_samples,
            width: rect.width(),
        });
    }
    let expected = required_row_strided_samples(rect, row_stride_samples)?;
    if samples.len() < expected {
        return Err(ReconError::WorkspaceWriteLengthMismatch {
            plane,
            expected,
            actual: samples.len(),
        });
    }

    let global_target_base = rect
        .y()
        .checked_mul(target_stride_samples)
        .and_then(|start| start.checked_add(rect.x()))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace global target row offset",
        })?;
    for row_index in 0..rect.height() {
        let source_start = row_index * row_stride_samples;
        let source_row = &samples[source_start..source_start + rect.width()];
        if source_row.iter().any(|sample| sample.to_u16() > max_sample) {
            let global_start = global_target_base + row_index * target_stride_samples;
            for (column, &sample) in source_row.iter().enumerate() {
                validate_sample_value(plane, global_start + column, sample, max_sample)?;
            }
        }
    }

    Ok(())
}

fn required_row_strided_samples(rect: PlaneRect, row_stride_samples: usize) -> Result<usize> {
    let row_offset = rect
        .height()
        .checked_sub(1)
        .ok_or(ReconError::ZeroDimension {
            field: "workspace rectangle height",
        })?
        .checked_mul(row_stride_samples)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace source row span",
        })?;
    row_offset
        .checked_add(rect.width())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "current-frame workspace source sample span",
        })
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod workspace_tests;
