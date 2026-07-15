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

use crate::intra_basic::predict_paeth_sample;
use crate::intra_dc_math::validate_sample_type;
use crate::intra_directional::predict_intra_cardinal_directional_rect_into;
use crate::intra_smooth::{SmoothSampleEdges, SmoothSamplePosition, predict_smooth_sample_values};
use crate::{
    DecodedFrame, DecodedFrameInfo, FrameMut, FramePlanes, FrameRef, IntraCardinalDirection,
    IntraDirectionalAngleEdge, IntraPaethEdge, IntraRectBlockSize, IntraSmoothEdge,
    IntraSmoothMode, IntraSquareBlockSize, PixelFormat, Plane, PlaneId, PlaneMut, PlaneRect,
    PlaneRef, PlaneSize, ReconError, ReconSample, Result,
};

#[path = "workspace_edges.rs"]
mod workspace_edges;
#[path = "workspace_interintra.rs"]
mod workspace_interintra;
#[path = "workspace_intra_dc.rs"]
mod workspace_intra_dc;
#[path = "workspace_intra_directional_angle.rs"]
mod workspace_intra_directional_angle;

pub use workspace_edges::{CurrentFrameIntraEdges, WorkspaceRectRows};
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
    intra_prediction_scratch: [Vec<T>; 2],
}

/// Selects one of the two reusable current-frame intra-prediction buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntraPredictionScratchBuffer {
    /// Primary prediction storage used by every intra mode.
    Primary,
    /// Secondary prediction storage used while blending two predictors.
    Secondary,
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
            intra_prediction_scratch: [Vec::new(), Vec::new()],
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
        match plane {
            PlaneId::Y => Ok(&self.y),
            PlaneId::U => self
                .u
                .as_ref()
                .ok_or(ReconError::MissingWorkspacePlane { plane }),
            PlaneId::V => self
                .v
                .as_ref()
                .ok_or(ReconError::MissingWorkspacePlane { plane }),
        }
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

    /// Writes a rectangular block of samples directly into workspace plane storage
    /// without per-sample bounds checking against `max_sample`.
    ///
    /// The caller guarantees that `samples` are already valid for the active bit depth
    /// (e.g., produced by an internal loop filter or prediction step).
    pub fn write_rect_trusted(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
    ) -> Result<()> {
        self.plane_mut(plane)?
            .write_rect_trusted(rect, samples, row_stride_samples)
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
        let mut scratch = Vec::new();
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

        self.write_rect(plane, target, &scratch, source.width())
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
        if sample_count > MAX_INTRA_PREDICTION_SAMPLES {
            return Err(ReconError::WorkspaceIntraPredictionScratchTooLarge {
                sample_count,
                max_sample_count: MAX_INTRA_PREDICTION_SAMPLES,
            });
        }
        let mut buffer = mem::take(match slot {
            IntraPredictionScratchBuffer::Primary => &mut self.intra_prediction_scratch[0],
            IntraPredictionScratchBuffer::Secondary => &mut self.intra_prediction_scratch[1],
        });
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

    /// Returns an intra-prediction buffer to its reusable workspace slot.
    pub fn recycle_intra_prediction_buffer(
        &mut self,
        slot: IntraPredictionScratchBuffer,
        mut buffer: Vec<T>,
    ) {
        buffer.clear();
        *match slot {
            IntraPredictionScratchBuffer::Primary => &mut self.intra_prediction_scratch[0],
            IntraPredictionScratchBuffer::Secondary => &mut self.intra_prediction_scratch[1],
        } = buffer;
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
        match plane {
            PlaneId::Y => Ok(&mut self.y),
            PlaneId::U => self
                .u
                .as_mut()
                .ok_or(ReconError::MissingWorkspacePlane { plane }),
            PlaneId::V => self
                .v
                .as_mut()
                .ok_or(ReconError::MissingWorkspacePlane { plane }),
        }
    }
}

/// Mutable backing storage for one current-frame workspace plane.
///
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

        let mut samples = Vec::new();
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
        Ok(WorkspaceRectRows::new(self, rect))
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
        if row_stride_samples < rect.width() {
            return Err(ReconError::WorkspaceWriteStrideTooSmall {
                plane: self.plane,
                stride_samples: row_stride_samples,
                width: rect.width(),
            });
        }

        let expected = required_row_strided_samples(rect, row_stride_samples)?;
        if samples.len() < expected {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane: self.plane,
                expected,
                actual: samples.len(),
            });
        }

        if rect.width() == 0 || rect.height() == 0 {
            return Ok(());
        }
        let target_base = self.row_range(rect.y(), rect.x(), rect.width())?.start;
        let last_row = rect.y() + rect.height() - 1;
        self.row_range(last_row, rect.x(), rect.width())?;

        for row_index in 0..rect.height() {
            let source_row_start = row_index * row_stride_samples;
            let target_start = target_base + row_index * self.stride_samples;
            let target_range = target_start..target_start + rect.width();
            let source_range = source_row_start..source_row_start + rect.width();
            let source_row = &samples[source_range];

            for (column, &sample) in source_row.iter().enumerate() {
                let value = sample.to_u16();
                if value > max_sample {
                    return Err(ReconError::SampleOutOfRange {
                        plane: self.plane,
                        sample_index: target_start + column,
                        value,
                        max: max_sample,
                    });
                }
            }

            // splot-copy-ok: write caller samples into owned current-frame workspace plane storage
            self.samples[target_range].copy_from_slice(source_row);
        }
        Ok(())
    }

    fn write_rect_trusted(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
    ) -> Result<()> {
        let rect = self.clamp_rect_to_storage(rect)?;
        if row_stride_samples < rect.width() {
            return Err(ReconError::WorkspaceWriteStrideTooSmall {
                plane: self.plane,
                stride_samples: row_stride_samples,
                width: rect.width(),
            });
        }

        let expected = required_row_strided_samples(rect, row_stride_samples)?;
        if samples.len() < expected {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane: self.plane,
                expected,
                actual: samples.len(),
            });
        }

        if rect.width() == 0 || rect.height() == 0 {
            return Ok(());
        }
        let target_base = self.row_range(rect.y(), rect.x(), rect.width())?.start;
        let last_row = rect.y() + rect.height() - 1;
        self.row_range(last_row, rect.x(), rect.width())?;

        for row_index in 0..rect.height() {
            let source_row_start = row_index * row_stride_samples;
            let target_start = target_base + row_index * self.stride_samples;
            let target_range = target_start..target_start + rect.width();
            let source_range = source_row_start..source_row_start + rect.width();
            // splot-copy-ok: write trusted caller samples into owned current-frame workspace plane storage
            self.samples[target_range].copy_from_slice(&samples[source_range]);
        }
        Ok(())
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

    /// Clamps a write/fill `rect` to the in-frame storage extent.
    ///
    /// Models AVM's in-frame-only reconstruction: frame-edge overhang is dropped,
    /// while a rectangle whose origin is already out of frame still errors.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `rect` starts outside
    /// storage.
    fn clamp_rect_to_storage(&self, rect: PlaneRect) -> Result<PlaneRect> {
        let storage_width = self.storage_size.width();
        let storage_height = self.storage_size.height();
        if rect.x() >= storage_width || rect.y() >= storage_height {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect,
            });
        }
        let max_width = storage_width - rect.x();
        let max_height = storage_height - rect.y();
        let width = rect.width().min(max_width);
        let height = rect.height().min(max_height);
        if width == rect.width() && height == rect.height() {
            return Ok(rect);
        }
        PlaneRect::new(rect.x(), rect.y(), width, height)
    }

    fn ensure_rect(&self, rect: PlaneRect) -> Result<()> {
        if rect.is_within(self.storage_size) {
            Ok(())
        } else {
            Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect,
            })
        }
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
