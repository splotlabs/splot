// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mutable current-frame reconstruction workspace.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`,
//! `RECON-INTRA-DC-RECTANGULAR-PREDICTION`,
//! `RECON-INTRA-BASIC-PAETH-PREDICTION`,
//! `RECON-INTRA-SMOOTH-PREDICTION`.

use core::mem;
use core::ops::Range;

use crate::intra::predict_intra_dc_rect_value_from_sums;
use crate::intra_basic::predict_paeth_sample;
use crate::intra_smooth::{SmoothSampleEdges, SmoothSamplePosition, predict_smooth_sample_values};
use crate::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, IntraDcEdges, IntraPaethEdge, IntraRectBlockSize,
    IntraSmoothEdge, IntraSmoothMode, IntraSquareBlockSize, PixelFormat, Plane, PlaneId, PlaneRect,
    PlaneSize, ReconError, ReconSample, Result,
};

/// Mutable current-frame reconstruction workspace.
///
/// The workspace owns checked plane storage that future decode or encoder paths
/// can fill incrementally before freezing into the immutable [`DecodedFrame`]
/// model. It is intentionally scheduler-free: callers own any parallel
/// partitioning above this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentFrameWorkspace<T: ReconSample> {
    info: DecodedFrameInfo,
    y: CurrentFramePlane<T>,
    u: Option<CurrentFramePlane<T>>,
    v: Option<CurrentFramePlane<T>>,
}

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

        Ok(Self { info, y, u, v })
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

    /// Returns all backing samples for `plane`, including padding if present.
    ///
    /// # Errors
    /// Returns [`ReconError::MissingWorkspacePlane`] for absent chroma planes in
    /// monochrome workspaces.
    pub fn samples(&self, plane: PlaneId) -> Result<&[T]> {
        Ok(self.plane(plane)?.samples())
    }

    /// Iterates over a checked rectangular region in `plane`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent or `rect` falls outside
    /// the plane storage.
    pub fn rect_rows(&self, plane: PlaneId, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        self.plane(plane)?.rect_rows(rect)
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
        if samples.len() != size.sample_count() {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane,
                expected: size.sample_count(),
                actual: samples.len(),
            });
        }

        let rect = block_rect(x, y, size)?;
        self.write_rect(plane, rect, samples, size.width())
    }

    /// Extracts left and above in-storage edges for a square block.
    ///
    /// The helper only reads edges adjacent to the requested plane-local square
    /// when they are inside workspace storage. It does not decide AV2 block,
    /// tile, superblock, or palette/CfL availability semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target square is out
    /// of bounds, or edge scratch allocation fails.
    pub fn intra_dc_edges_for_square(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        self.intra_dc_edges_for_rect(plane, x, y, size.into())
    }

    /// Extracts left and above in-storage edges for a rectangular block.
    ///
    /// The helper only reads edges adjacent to the requested plane-local
    /// rectangle when they are inside workspace storage. It does not decide AV2
    /// block, tile, superblock, subsampled-DC, palette, or CfL availability
    /// semantics.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle is
    /// out of bounds, or edge scratch allocation fails.
    pub fn intra_dc_edges_for_rect(
        &self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<CurrentFrameIntraEdges<T>> {
        let rect = block_rect(x, y, size)?;
        self.plane(plane)?.intra_dc_edges_for_rect(rect)
    }

    /// Predicts square DC intra samples into the workspace.
    ///
    /// This is a convenience wrapper over [`Self::predict_intra_dc_rect`]. Edge
    /// extraction is limited to in-storage left/above neighbors and does not
    /// model AV2 availability.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
    /// or invalid prediction inputs.
    pub fn predict_intra_dc_square(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
    ) -> Result<()> {
        self.predict_intra_dc_rect(plane, x, y, size.into())
    }

    /// Predicts rectangular DC intra samples into the workspace.
    ///
    /// This computes the constant DC sample from in-storage left/above neighbor
    /// sums and fills the target rectangle. Edge extraction is limited to
    /// in-storage neighbors and does not model AV2 availability.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
    /// or invalid prediction inputs.
    pub fn predict_intra_dc_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<()> {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        let (left_sum, above_sum) = self.plane(plane)?.intra_dc_edge_sums_for_rect(rect)?;
        let sample = predict_intra_dc_rect_value_from_sums(bit_depth, size, left_sum, above_sum)?;

        self.plane_mut(plane)?.fill_rect(rect, sample)
    }

    /// Predicts rectangular basic/PAETH intra samples into the workspace.
    ///
    /// This helper uses in-storage top-left, left, and above neighbor samples as
    /// the prepared AV2 §7.13.2.2 inputs. It does not synthesize §7.13.2.1
    /// fallback samples or decide AV2 edge availability, MRL, tile-boundary, or
    /// superblock semantics.
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
    /// This helper uses in-storage left, above, bottom-left, and top-right
    /// neighbor samples as the prepared AV2 §7.13.2.13 inputs. It does not
    /// synthesize §7.13.2.1 fallback samples or decide AV2 edge availability,
    /// MRL, tile-boundary, or superblock semantics.
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

    /// Iterates over a checked rectangular region in this plane.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] when `rect` falls
    /// outside the plane storage.
    pub fn rect_rows(&self, rect: PlaneRect) -> Result<WorkspaceRectRows<'_, T>> {
        self.ensure_rect(rect)?;
        Ok(WorkspaceRectRows {
            plane: self,
            rect,
            next_row: 0,
        })
    }

    fn fill_rect(&mut self, rect: PlaneRect, sample: T) -> Result<()> {
        self.ensure_rect(rect)?;
        for row in rect.y()..rect.y() + rect.height() {
            let range = self.row_range(row, rect.x(), rect.width())?;
            self.samples[range].fill(sample);
        }
        Ok(())
    }

    fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
        max_sample: u16,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
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

        for row_index in 0..rect.height() {
            let source_row_start = row_index.checked_mul(row_stride_samples).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "current-frame workspace source row offset",
                },
            )?;
            let target_row = rect.y() + row_index;
            let target_start = self
                .sample_index(rect.x(), target_row)?
                .checked_add(rect.width())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "current-frame workspace target row validation",
                })?
                - rect.width();

            for column in 0..rect.width() {
                let source_index = source_row_start + column;
                let target_index = target_start + column;
                validate_sample_value(self.plane, target_index, samples[source_index], max_sample)?;
            }
        }

        for row_index in 0..rect.height() {
            let source_row_start = row_index.checked_mul(row_stride_samples).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "current-frame workspace source row offset",
                },
            )?;
            let target_row = rect.y() + row_index;
            let target_range = self.row_range(target_row, rect.x(), rect.width())?;
            let source_range = source_row_start..source_row_start + rect.width();
            self.samples[target_range].copy_from_slice(&samples[source_range]);
        }
        Ok(())
    }

    fn intra_dc_edges_for_rect(&self, rect: PlaneRect) -> Result<CurrentFrameIntraEdges<T>> {
        self.ensure_rect(rect)?;

        let left = if rect.x() == 0 {
            None
        } else {
            let mut left = Vec::new();
            left.try_reserve_exact(rect.height()).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane: self.plane,
                    context: "left intra edge",
                }
            })?;
            for row in rect.y()..rect.y() + rect.height() {
                let index = self.sample_index(rect.x() - 1, row)?;
                left.push(self.samples[index]);
            }
            Some(left)
        };

        let above = if rect.y() == 0 {
            None
        } else {
            let row = rect.y() - 1;
            let range = self.row_range(row, rect.x(), rect.width())?;
            let mut above = Vec::new();
            above.try_reserve_exact(rect.width()).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane: self.plane,
                    context: "above intra edge",
                }
            })?;
            above.extend_from_slice(&self.samples[range]);
            Some(above)
        };

        Ok(CurrentFrameIntraEdges { left, above })
    }

    fn intra_dc_edge_sums_for_rect(&self, rect: PlaneRect) -> Result<(Option<u64>, Option<u64>)> {
        self.ensure_rect(rect)?;

        let left = if rect.x() == 0 {
            None
        } else {
            let mut sum = 0u64;
            for row in rect.y()..rect.y() + rect.height() {
                let index = self.sample_index(rect.x() - 1, row)?;
                sum += u64::from(self.samples[index].to_u16());
            }
            Some(sum)
        };

        let above = if rect.y() == 0 {
            None
        } else {
            let row = rect.y() - 1;
            let range = self.row_range(row, rect.x(), rect.width())?;
            Some(
                self.samples[range]
                    .iter()
                    .map(|sample| u64::from(sample.to_u16()))
                    .sum(),
            )
        };

        Ok((left, above))
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

    fn freeze(self) -> Result<Plane<T>> {
        Plane::from_vec(
            self.storage_size,
            self.stride_samples,
            self.visible_rect,
            self.samples,
        )
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

/// Iterator over checked workspace rectangle rows.
#[derive(Clone, Debug)]
pub struct WorkspaceRectRows<'a, T: ReconSample> {
    plane: &'a CurrentFramePlane<T>,
    rect: PlaneRect,
    next_row: usize,
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

fn validate_sample_type<T: ReconSample>(bit_depth: crate::BitDepth) -> Result<()> {
    if T::supports_bit_depth(bit_depth) {
        Ok(())
    } else {
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth,
        })
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
