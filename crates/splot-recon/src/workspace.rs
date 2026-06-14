// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mutable current-frame reconstruction workspace.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use core::mem;
use core::ops::Range;

use crate::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, IntraDcEdges, IntraSquareBlockSize, PixelFormat,
    Plane, PlaneId, PlaneRect, PlaneSize, ReconError, ReconSample, Result,
    predict_intra_dc_square_into,
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
        if samples.len() != size.sample_count() {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane,
                expected: size.sample_count(),
                actual: samples.len(),
            });
        }

        let rect = square_rect(x, y, size)?;
        self.write_rect(plane, rect, samples, size.side_len())
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
        let rect = square_rect(x, y, size)?;
        self.plane(plane)?.intra_dc_edges_for_square(rect)
    }

    /// Predicts square DC intra samples into the workspace.
    ///
    /// This is a convenience wrapper over the existing
    /// [`predict_intra_dc_square_into`] primitive. Edge extraction is limited to
    /// in-storage left/above neighbors and does not model AV2 availability.
    ///
    /// # Errors
    /// Returns [`ReconError`] for invalid target geometry, absent planes,
    /// invalid prediction inputs, or scratch allocation failure.
    pub fn predict_intra_dc_square(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraSquareBlockSize,
    ) -> Result<()> {
        let edges = self.intra_dc_edges_for_square(plane, x, y, size)?;
        let rect = square_rect(x, y, size)?;
        let bit_depth = self.info.bit_depth();
        let target = self.plane_mut(plane)?;
        let start = target.sample_index(rect.x(), rect.y())?;

        predict_intra_dc_square_into(
            bit_depth,
            size,
            edges.as_dc_edges(),
            &mut target.samples[start..],
            target.stride_samples,
        )
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

    fn intra_dc_edges_for_square(&self, rect: PlaneRect) -> Result<CurrentFrameIntraEdges<T>> {
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

    /// Borrows the owned edges as square DC prediction inputs.
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

fn square_rect(x: usize, y: usize, size: IntraSquareBlockSize) -> Result<PlaneRect> {
    PlaneRect::new(x, y, size.side_len(), size.side_len())
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
