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
use std::sync::Arc;

use crate::intra_basic::predict_paeth_sample;
use crate::intra_dc_math::validate_sample_type;
use crate::intra_directional::predict_intra_cardinal_directional_rect_into;
use crate::intra_smooth::{SmoothSampleEdges, SmoothSamplePosition, predict_smooth_sample_values};
use crate::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FrameMut, FramePlaneSamples, FramePlanes, FrameRef,
    IntraCardinalDirection, IntraDirectionalAngleEdge, IntraPaethEdge, IntraRectBlockSize,
    IntraSmoothEdge, IntraSmoothMode, PixelFormat, Plane, PlaneId, PlaneMut, PlaneRect, PlaneRef,
    PlaneSize, ReconError, ReconSample, Result,
};

mod owned_rect;
#[path = "workspace_edges.rs"]
mod workspace_edges;
#[path = "workspace_interintra.rs"]
mod workspace_interintra;
#[path = "workspace_intra_dc.rs"]
mod workspace_intra_dc;
#[path = "workspace_intra_directional_angle.rs"]
mod workspace_intra_directional_angle;
#[path = "workspace_rows.rs"]
mod workspace_rows;
pub use owned_rect::{OwnedFrameRect, OwnedFrameRectRows};
pub use workspace_edges::CurrentFrameIntraEdges;
pub use workspace_interintra::{InterIntraMode, wedge_mask_plane_sample};
pub use workspace_rows::{CurrentFrameRectRowsMut, WorkspaceRectRows};

macro_rules! contiguous_rect_writer {
    ($name:ident, $sample:ty, $slice_mut:ident, $offset:literal, $span:literal) => {
        #[doc = concat!(
            "Runs a writer over contiguous `",
            stringify!($sample),
            "` storage for one exact target rectangle.\n\n",
            "The returned slice starts at the rectangle's top-left sample and spans through its ",
            "final row; `stride` is the destination row stride. Returns `Ok(None)` for other ",
            "sample storage or a rectangle clipped at the frame edge.\n\n",
            "# Errors\n",
            "Returns [`ReconError`] when the plane is absent, the target geometry is invalid, ",
            "or a row target would cross its exclusive band."
        )]
        pub fn $name<R>(
            &mut self,
            plane: PlaneId,
            rect: PlaneRect,
            write: impl FnOnce(&mut [$sample], usize) -> Result<R>,
        ) -> Result<Option<R>> {
            let storage = self.plane_storage_size(plane)?;
            let clipped = clamp_rect_to_storage(plane, storage, rect)?;
            if clipped != rect {
                return Ok(None);
            }
            self.rect_rows(plane, rect)?;
            let (samples, stride, local_x, local_y) = match self {
                Self::Frame(workspace) => {
                    let target = workspace.plane_mut(plane)?;
                    let stride_samples = target.stride_samples();
                    (&mut target.samples[..], stride_samples, rect.x(), rect.y())
                }
                Self::Rect(surface) => {
                    let target = surface.plane_mut(plane)?;
                    target.ensure_rect(rect)?;
                    let stride = target.stride();
                    let local_y = rect.y() - target.rect.y();
                    (&mut target.samples[..], stride, rect.x(), local_y)
                }
                Self::OwnedRect(surface) => {
                    let target = surface.plane_mut(plane)?;
                    target.ensure_rect(rect)?;
                    let target_rect = target.rect();
                    (
                        target.into_samples_mut(),
                        target_rect.width(),
                        rect.x() - target_rect.x(),
                        rect.y() - target_rect.y(),
                    )
                }
            };
            let Some(samples) = T::$slice_mut(samples) else {
                return Ok(None);
            };
            let base = local_y
                .checked_mul(stride)
                .and_then(|start| start.checked_add(local_x))
                .ok_or(ReconError::ArithmeticOverflow { context: $offset })?;
            let end = (rect.height() - 1)
                .checked_mul(stride)
                .and_then(|offset| base.checked_add(offset))
                .and_then(|start| start.checked_add(rect.width()))
                .ok_or(ReconError::ArithmeticOverflow { context: $span })?;
            let available = samples.len();
            let target = samples
                .get_mut(base..end)
                .ok_or(ReconError::BufferLengthMismatch {
                    expected: end,
                    actual: available,
                })?;
            write(target, stride).map(Some)
        }
    };
}

/// Mutable current-frame reconstruction workspace.
///
/// The workspace owns checked plane storage that callers fill incrementally
/// before freezing into the immutable [`DecodedFrame`]
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

/// Exclusive storage for one full-width band of a current-frame plane.
///
/// A band spans whole plane rows, so it is one contiguous run of the plane's
/// samples and needs no per-row slice list to keep adjacent bands disjoint.
#[derive(Debug)]
pub struct CurrentFramePlaneRect<'a, T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    rect: PlaneRect,
    samples: &'a mut [T],
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

    const fn stride(&self) -> usize {
        self.storage_size.width()
    }

    fn offset_of(&self, rect: PlaneRect) -> Result<usize> {
        (rect.y() - self.rect.y())
            .checked_mul(self.stride())
            .and_then(|row| row.checked_add(rect.x()))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame row band offset",
            })
    }

    fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)?;
        self.ensure_rect(rect)?;
        let local = PlaneRect::new(
            rect.x(),
            rect.y() - self.rect.y(),
            rect.width(),
            rect.height(),
        )?;
        Ok(WorkspaceRectRows::Strided(
            PlaneRef::from_parts(self.samples, self.stride(), local).visible_rows(),
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
        validate_write_source(
            self.plane,
            rect,
            samples,
            row_stride_samples,
            self.storage_size.width(),
            max_sample,
        )?;
        let stride = self.stride();
        let base = self.offset_of(rect)?;
        for row in 0..rect.height() {
            let target_start = base + row * stride;
            let source_start = row * row_stride_samples;
            copy_row_samples(
                &mut self.samples[target_start..target_start + rect.width()],
                &samples[source_start..source_start + rect.width()],
            );
        }
        Ok(())
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        ensure_surface_rect(self.plane, self.rect, rect)
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
        let start = self.rect.y() * target.stride_samples();
        let output = target
            .samples
            .get_mut(start..start + self.samples.len())
            .ok_or(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: target.storage_size,
                rect: self.rect,
            })?;
        copy_row_samples(output, self.samples);
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
    /// Returns [`ReconError`] when plane geometry differs.
    pub fn publish_into(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        self.y.publish_into(&mut workspace.y)?;
        match (&self.u, &mut workspace.u) {
            (Some(source), Some(target)) => source.publish_into(target)?,
            (None, None) => {}
            (Some(_), None) => {
                return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U });
            }
            (None, Some(_)) => {
                return Err(ReconError::MissingChromaPlane { plane: PlaneId::U });
            }
        }
        match (&self.v, &mut workspace.v) {
            (Some(source), Some(target)) => source.publish_into(target)?,
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

/// Checked reconstruction target backed by a whole frame or one exclusive rectangle.
#[derive(Debug)]
pub enum CurrentFrameSurface<'surface, 'storage, T: ReconSample> {
    /// Existing ordered reconstruction over the complete current frame.
    Frame(&'surface mut CurrentFrameWorkspace<T>),
    /// Reconstruction over one exclusive rectangular frame region.
    Rect(&'surface mut CurrentFrameRect<'storage, T>),
    /// Reconstruction over one caller-owned rectangular frame region.
    OwnedRect(&'surface mut OwnedFrameRect<T>),
}

struct CurrentFrameResidualTarget<'surface, T: ReconSample> {
    samples: &'surface mut [T],
    stride: usize,
    base: usize,
    rect: PlaneRect,
    max_sample: u16,
}

impl<T: ReconSample> CurrentFrameResidualTarget<'_, T> {
    #[inline]
    fn add(self, mut residual_at: impl FnMut(usize, usize) -> i32) -> Result<()> {
        let max = i32::from(self.max_sample);
        for row in 0..self.rect.height() {
            let target_start = self.base + row * self.stride;
            add_residual_row(
                &mut self.samples[target_start..target_start + self.rect.width()],
                row,
                max,
                &mut residual_at,
            )?;
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

impl<T: ReconSample> CurrentFrameSurface<'_, '_, T> {
    /// Returns the decoded-frame metadata for this target.
    pub fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Frame(workspace) => workspace.info(),
            Self::Rect(rect) => rect.info(),
            Self::OwnedRect(rect) => rect.info(),
        }
    }

    /// Returns the complete frame-plane storage size.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] for absent chroma planes.
    pub fn plane_storage_size(&self, plane: PlaneId) -> Result<PlaneSize> {
        match self {
            Self::Frame(workspace) => Ok(workspace.plane(plane)?.storage_size()),
            Self::Rect(rect) => Ok(rect.plane(plane)?.storage_size()),
            Self::OwnedRect(rect) => Ok(rect.plane(plane)?.storage_size()),
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
            Self::Rect(surface) => surface.plane(plane)?.rect_rows(rect),
            Self::OwnedRect(surface) => surface.plane(plane)?.rect_rows(rect),
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
            Self::Rect(surface) => {
                let max_sample = surface.info().bit_depth().max_sample();
                surface
                    .plane_mut(plane)?
                    .write_rect(rect, samples, row_stride_samples, max_sample)
            }
            Self::OwnedRect(surface) => {
                let max_sample = surface.info().bit_depth().max_sample();
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
        if u16_samples_exceed(samples, max_sample) {
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
        }

        match self {
            Self::Frame(workspace) => {
                let target = workspace.plane_mut(plane)?;
                let stride_samples = target.stride_samples();
                write_u16_rect_to_samples(
                    &mut target.samples,
                    stride_samples,
                    rect,
                    rect.x(),
                    rect.y(),
                    samples,
                    row_stride_samples,
                )
            }
            Self::Rect(surface) => {
                let target = surface.plane_mut(plane)?;
                target.ensure_rect(rect)?;
                let stride = target.stride();
                write_u16_rect_to_samples(
                    target.samples,
                    stride,
                    rect,
                    rect.x(),
                    rect.y() - target.rect.y(),
                    samples,
                    row_stride_samples,
                )
            }
            Self::OwnedRect(surface) => {
                let mut target = surface.plane_mut(plane)?;
                target.ensure_rect(rect)?;
                let target_rect = target.rect();
                write_u16_rect_to_samples(
                    target.samples_mut(),
                    target_rect.width(),
                    rect,
                    rect.x() - target_rect.x(),
                    rect.y() - target_rect.y(),
                    samples,
                    row_stride_samples,
                )
            }
        }
    }

    contiguous_rect_writer!(
        with_contiguous_u16_rect_mut,
        u16,
        u16_slice_mut,
        "contiguous u16 target offset",
        "contiguous u16 target span"
    );
    contiguous_rect_writer!(
        with_contiguous_u8_rect_mut,
        u8,
        u8_slice_mut,
        "contiguous u8 target offset",
        "contiguous u8 target span"
    );

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
    fn residual_rect_target(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        source_stride: usize,
    ) -> Result<CurrentFrameResidualTarget<'_, T>> {
        let max_sample = self.info().bit_depth().max_sample();
        let (target, target_stride, target_base, rect) = match self {
            Self::Frame(workspace) => {
                let target = workspace.plane_mut(plane)?;
                let rect = target.clamp_rect_to_storage(rect)?;
                let stride = target.stride_samples();
                let base = rect
                    .y()
                    .checked_mul(stride)
                    .and_then(|start| start.checked_add(rect.x()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "current-frame residual target row offset",
                    })?;
                (&mut target.samples[..], stride, base, rect)
            }
            Self::Rect(surface) => {
                let target = surface.plane_mut(plane)?;
                let rect = clamp_rect_to_storage(target.plane, target.storage_size, rect)?;
                target.ensure_rect(rect)?;
                let stride = target.stride();
                let base = target.offset_of(rect)?;
                (&mut target.samples[..], stride, base, rect)
            }
            Self::OwnedRect(surface) => {
                let target = surface.plane_mut(plane)?;
                let rect = clamp_rect_to_storage(target.plane(), target.storage_size(), rect)?;
                target.ensure_rect(rect)?;
                let target_rect = target.rect();
                let stride = target_rect.width();
                let base = (rect.y() - target_rect.y())
                    .checked_mul(stride)
                    .and_then(|start| start.checked_add(rect.x() - target_rect.x()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "current-frame residual target row offset",
                    })?;
                (target.into_samples_mut(), stride, base, rect)
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
        Ok(CurrentFrameResidualTarget {
            samples: target,
            stride: target_stride,
            base: target_base,
            rect,
            max_sample,
        })
    }
}

/// Hands out the successive full-width bands of one plane's storage.
struct PlaneBandSplit<'a, T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    rest: &'a mut [T],
    settled_rows: usize,
}

impl<'a, T: ReconSample> PlaneBandSplit<'a, T> {
    fn new(plane: &'a mut CurrentFramePlane<T>) -> Self {
        Self {
            plane: plane.plane,
            storage_size: plane.storage_size,
            rest: &mut plane.samples,
            settled_rows: 0,
        }
    }

    /// Splits `rect` off the storage this split has not handed out yet.
    ///
    /// Bands run down the plane, so a rectangle that is not full width or that
    /// reaches back above one already handed out cannot be carved out here.
    fn take(&mut self, rect: PlaneRect) -> Result<CurrentFramePlaneRect<'a, T>> {
        ensure_rect_in_storage(self.plane, self.storage_size, rect)?;
        let stride = self.storage_size.width();
        if rect.x() != 0 || rect.width() != stride || rect.y() < self.settled_rows {
            return Err(ReconError::WorkspaceRectSurfaceNotABand {
                plane: self.plane,
                storage: self.storage_size,
                rect,
                settled_rows: self.settled_rows,
            });
        }
        let skip = (rect.y() - self.settled_rows) * stride;
        let span = rect.height() * stride;
        let rest = core::mem::take(&mut self.rest);
        let (_, below) = rest.split_at_mut(skip);
        let (band, tail) = below.split_at_mut(span);
        self.rest = tail;
        self.settled_rows = rect.y() + rect.height();
        Ok(CurrentFramePlaneRect {
            plane: self.plane,
            storage_size: self.storage_size,
            rect,
            samples: band,
        })
    }
}

/// The chroma rectangle covering one luma rectangle.
///
/// The rectangle form exists because callers that want a single rectangle
/// vastly outnumber the one that partitions a whole frame, and building a
/// `Vec` to take its first element was one allocation per rectangle.
pub(crate) fn subsampled_rect(
    rect: PlaneRect,
    shift_x: u8,
    shift_y: u8,
    storage: PlaneSize,
) -> Result<PlaneRect> {
    let scale_x = 1usize << shift_x;
    let scale_y = 1usize << shift_y;
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
    PlaneRect::new(
        x,
        y,
        right.div_ceil(scale_x).min(storage.width()) - x,
        bottom.div_ceil(scale_y).min(storage.height()) - y,
    )
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
        validate_sample_value(PlaneId::Y, 0, fill, info.bit_depth().max_sample())?;
        Self::with_fill(info, Some(fill))
    }

    pub(crate) fn with_fill(info: DecodedFrameInfo, fill: Option<T>) -> Result<Self> {
        Self::with_planes(info, fill, &mut FramePlaneSamples::default())
    }

    fn with_planes(
        info: DecodedFrameInfo,
        fill: Option<T>,
        recycled: &mut FramePlaneSamples<T>,
    ) -> Result<Self> {
        validate_sample_type::<T>(info.bit_depth())?;
        let luma_size = info.coded_luma_size();
        let luma_rect = info.visible_luma_rect();
        let pool = recycled.pool.clone();
        let pool = pool.as_ref();
        let y = CurrentFramePlane::new(
            PlaneId::Y,
            luma_size,
            luma_rect,
            fill,
            recycled.take(PlaneId::Y),
            pool,
        )?;
        let (u, v) = match chroma_plane_geometry(info.pixel_format(), luma_size, luma_rect)? {
            None => (None, None),
            Some((storage_size, visible_rect)) => (
                Some(CurrentFramePlane::new(
                    PlaneId::U,
                    storage_size,
                    visible_rect,
                    fill,
                    recycled.take(PlaneId::U),
                    pool,
                )?),
                Some(CurrentFramePlane::new(
                    PlaneId::V,
                    storage_size,
                    visible_rect,
                    fill,
                    recycled.take(PlaneId::V),
                    pool,
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

    /// Partitions the frame into exclusive Y/U/V surfaces over full-width bands.
    ///
    /// Each band is a contiguous run of every plane's storage, so bands handed
    /// to different workers cannot alias. Chroma bounds are derived from the
    /// frame subsampling and clipped to coded storage.
    ///
    /// # Errors
    /// Returns [`ReconError`] when a rectangle exceeds luma storage, is not a
    /// full-width band below the previous one, geometry overflows, or the
    /// surface list cannot be allocated.
    pub fn rect_surfaces(
        &mut self,
        luma_rects: &[PlaneRect],
    ) -> Result<Vec<CurrentFrameRect<'_, T>>> {
        let info = self.info;
        let pixel_format = info.pixel_format();
        let mut output = Vec::new();
        output.try_reserve_exact(luma_rects.len()).map_err(|_| {
            ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "rectangle surfaces",
            }
        })?;
        let Self { y, u, v, .. } = self;
        let mut y_split = PlaneBandSplit::new(y);
        let mut u_split = u.as_mut().map(PlaneBandSplit::new);
        let mut v_split = v.as_mut().map(PlaneBandSplit::new);
        for &rect in luma_rects {
            let chroma = match u_split.as_ref() {
                Some(split) => Some(subsampled_rect(
                    rect,
                    pixel_format.subsampling_x(),
                    pixel_format.subsampling_y(),
                    split.storage_size,
                )?),
                None => None,
            };
            output.push(CurrentFrameRect {
                info,
                y: y_split.take(rect)?,
                u: match (u_split.as_mut(), chroma) {
                    (Some(split), Some(chroma)) => Some(split.take(chroma)?),
                    _ => None,
                },
                v: match (v_split.as_mut(), chroma) {
                    (Some(split), Some(chroma)) => Some(split.take(chroma)?),
                    _ => None,
                },
            });
        }
        Ok(output)
    }

    /// Hands this workspace's sample buffers to the next frame that needs them.
    #[must_use]
    pub fn into_plane_samples(mut self) -> FramePlaneSamples<T> {
        let pool = self.y.pool.clone();
        FramePlaneSamples::new(
            mem::take(&mut self.y.samples),
            self.u.as_mut().map(|plane| mem::take(&mut plane.samples)),
            self.v.as_mut().map(|plane| mem::take(&mut plane.samples)),
        )
        .with_pool(pool.as_ref())
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
        let (target, rect, bit_depth) = self.intra_rect_target(plane, x, y, size)?;
        target.predict_intra_smooth_rect(rect, size, mode, bit_depth)
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
        let (target, rect, bit_depth) = self.intra_rect_target(plane, x, y, size)?;
        target.predict_intra_cardinal_directional_rect(rect, size, direction, bit_depth)
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

    fn plane_mut(&mut self, plane: PlaneId) -> Result<&mut CurrentFramePlane<T>> {
        select_plane_mut(plane, &mut self.y, self.u.as_mut(), self.v.as_mut())
    }

    /// Resolves the writable plane, block rectangle, and active bit depth shared
    /// by the rectangular intra prediction entry points.
    fn intra_rect_target(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<(&mut CurrentFramePlane<T>, PlaneRect, BitDepth)> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        Ok((self.plane_mut(plane)?, rect, bit_depth))
    }
}

/// Does not implement `Clone`: it owns the plane sample buffer (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug)]
pub struct CurrentFramePlane<T: ReconSample> {
    plane: PlaneId,
    storage_size: PlaneSize,
    visible_rect: PlaneRect,
    samples: Vec<T>,
    pool: Option<Arc<crate::PlanePool>>,
}

/// The pool a plane retires into is not part of its value.
impl<T: ReconSample + PartialEq> PartialEq for CurrentFramePlane<T> {
    fn eq(&self, other: &Self) -> bool {
        self.plane == other.plane
            && self.storage_size == other.storage_size
            && self.visible_rect == other.visible_rect
            && self.samples == other.samples
    }
}

impl<T: ReconSample + Eq> Eq for CurrentFramePlane<T> {}

/// Returns a retired workspace plane's storage to the pool the next workspace
/// of this depth takes from.
impl<T: ReconSample> Drop for CurrentFramePlane<T> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.take() {
            pool.recycle(mem::take(&mut self.samples));
        }
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn new(
        plane: PlaneId,
        storage_size: PlaneSize,
        visible_rect: PlaneRect,
        fill: Option<T>,
        mut samples: Vec<T>,
        pool: Option<&Arc<crate::PlanePool>>,
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
        required_samples.checked_mul(mem::size_of::<T>()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "current-frame workspace plane allocation byte count",
            },
        )?;

        if let Some(pool) = pool
            && samples.capacity() < required_samples
        {
            // A buffer too small for this frame would be reallocated anyway, so
            // it goes back to the pool and a frame-sized spare comes out.
            pool.recycle(mem::take(&mut samples));
            samples = pool.take(required_samples);
        }
        samples.truncate(required_samples);
        samples
            .try_reserve_exact(required_samples.saturating_sub(samples.len()))
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane,
                context: "sample buffer",
            })?;
        let recycled = samples.len();
        samples.resize(required_samples, fill.unwrap_or_default());
        if let Some(fill) = fill {
            // A recycled buffer still holds the last frame's samples, which
            // `resize` only overwrites past the end of what it kept.
            samples[..recycled.min(required_samples)].fill(fill);
        }

        Ok(Self {
            pool: pool.map(Arc::clone),
            plane,
            storage_size,
            visible_rect,
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
        self.storage_size.width()
    }

    /// Returns the visible decoded-output rectangle.
    pub const fn visible_rect(&self) -> PlaneRect {
        self.visible_rect
    }

    /// Returns the required backing sample count.
    pub const fn required_samples(&self) -> usize {
        self.samples.len()
    }

    /// Returns the backing allocation size in bytes for the sample type.
    pub const fn allocation_bytes(&self) -> usize {
        self.samples.len() * mem::size_of::<T>()
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
        PlaneRef::from_parts(&self.samples, self.stride_samples(), self.visible_rect)
    }

    /// Borrows this plane's storage as an exclusive [`PlaneMut`] without copying.
    pub fn as_plane_mut(&mut self) -> PlaneMut<'_, T> {
        let stride_samples = self.stride_samples();
        PlaneMut::from_parts(&mut self.samples, stride_samples, self.visible_rect)
    }

    /// Iterates over a checked rectangular region in this plane.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `rect` falls
    /// outside the plane storage.
    pub fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        self.ensure_rect(rect)?;
        Ok(WorkspaceRectRows::Strided(
            PlaneRef::from_parts(&self.samples, self.stride_samples(), rect).visible_rows(),
        ))
    }

    fn fill_rect(&mut self, rect: PlaneRect, sample: T) -> Result<()> {
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
        // Drop partial frame-edge overhang; an out-of-frame origin remains an error.
        let rect = self.clamp_rect_to_storage(rect)?;
        let stride_samples = self.stride_samples();
        write_rect_to_samples(
            self.plane,
            &mut self.samples,
            stride_samples,
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
        let stride_samples = self.stride_samples();
        predict_intra_cardinal_directional_rect_into(
            bit_depth,
            size,
            direction,
            edges,
            &mut self.samples[output_start..],
            stride_samples,
        )
    }

    fn freeze(mut self) -> Result<Plane<T>> {
        let stride_samples = self.stride_samples();
        let mut plane = Plane::from_vec(
            self.storage_size,
            stride_samples,
            self.visible_rect,
            mem::take(&mut self.samples),
        )?;
        plane.pool = self.pool.take();
        Ok(plane)
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
            y.checked_mul(self.stride_samples())
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

    copy_rect_rows(
        target,
        target_stride_samples,
        target_base,
        rect,
        samples,
        row_stride_samples,
        |target, source| {
            copy_row_samples(target, source);
            Ok(())
        },
    )
}

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
    copy_rect_rows(
        target,
        target_stride_samples,
        target_base,
        rect,
        samples,
        row_stride_samples,
        copy_u16_samples,
    )
}

#[inline]
fn copy_rect_rows<T, S>(
    target: &mut [T],
    target_stride_samples: usize,
    target_base: usize,
    rect: PlaneRect,
    samples: &[S],
    row_stride_samples: usize,
    mut copy_row: impl FnMut(&mut [T], &[S]) -> Result<()>,
) -> Result<()> {
    for row in 0..rect.height() {
        let source_start = row * row_stride_samples;
        let target_start = target_base + row * target_stride_samples;
        copy_row(
            &mut target[target_start..target_start + rect.width()],
            &samples[source_start..source_start + rect.width()],
        )?;
    }
    Ok(())
}

/// Copies one rectangle row inline instead of through a size-dispatched memmove.
///
/// Rect rows run 8..2048 samples; at those widths a runtime-length
/// [`slice::copy_from_slice`] lowers to a `memmove`/`memcpy` call whose entry
/// cost dominates the move, so hot block writes pay call overhead per row.
/// Rows below the 512-byte crossover are copied as straight-line fixed-size
/// power-of-two pieces that fold into inline vector loads and stores; any copy
/// loop here would be recognized by LLVM and re-lowered to a library call.
/// Rows at or above 512 bytes keep the tuned library memcpy, where it wins.
#[allow(clippy::inline_always, reason = "measured no-LTO row-copy hot path")]
#[inline(always)]
fn copy_row_samples<T: ReconSample>(dst: &mut [T], src: &[T]) {
    debug_assert_eq!(dst.len(), src.len(), "rect row copy length mismatch");
    let len = dst.len();
    if std::mem::size_of_val(dst) >= 512 {
        // splot-copy-ok: full-width rows take the tuned library memcpy past the crossover.
        dst[..len].copy_from_slice(&src[..len]);
        return;
    }
    let mut done = 0;
    if done + 256 <= len {
        copy_fixed_chunk::<256, T>(dst, src, done);
        done += 256;
    }
    if done + 128 <= len {
        copy_fixed_chunk::<128, T>(dst, src, done);
        done += 128;
    }
    if done + 64 <= len {
        copy_fixed_chunk::<64, T>(dst, src, done);
        done += 64;
    }
    if done + 32 <= len {
        copy_fixed_chunk::<32, T>(dst, src, done);
        done += 32;
    }
    if done + 16 <= len {
        copy_fixed_chunk::<16, T>(dst, src, done);
        done += 16;
    }
    if done + 8 <= len {
        copy_fixed_chunk::<8, T>(dst, src, done);
        done += 8;
    }
    if done + 4 <= len {
        copy_fixed_chunk::<4, T>(dst, src, done);
        done += 4;
    }
    if done + 2 <= len {
        copy_fixed_chunk::<2, T>(dst, src, done);
        done += 2;
    }
    if done < len {
        dst[done] = src[done];
    }
}

#[inline]
fn copy_fixed_chunk<const N: usize, T: ReconSample>(dst: &mut [T], src: &[T], at: usize) {
    // splot-copy-ok: a fixed chunk length folds into inline vector loads and stores.
    dst[at..at + N].copy_from_slice(&src[at..at + N]);
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
    if row_stride_samples == rect.width()
        && let Some(span) = rect.width().checked_mul(rect.height())
        && let Some(source) = samples.get(..span)
        && !samples_exceed(source, max_sample)
    {
        return Ok(());
    }
    for row_index in 0..rect.height() {
        let source_start = row_index * row_stride_samples;
        let source_row = &samples[source_start..source_start + rect.width()];
        if samples_exceed(source_row, max_sample) {
            let global_start = global_target_base + row_index * target_stride_samples;
            for (column, &sample) in source_row.iter().enumerate() {
                validate_sample_value(plane, global_start + column, sample, max_sample)?;
            }
        }
    }

    Ok(())
}

pub(crate) use crate::sample_range::{samples_exceed, u16_samples_exceed};

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
