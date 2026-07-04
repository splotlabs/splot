// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Immutable owned decoded output plane storage.

use core::mem;

use crate::{PlaneRect, PlaneRef, PlaneSize, ReconError, ReconSample, Result};

/// Immutable owned decoded plane with explicit storage and visible geometry.
///
/// Does not implement `Clone`: cloning would duplicate the backing sample
/// buffer. Borrow it as a [`PlaneRef`] with [`Plane::as_plane_ref`] instead (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Eq, PartialEq)]
pub struct Plane<T: ReconSample> {
    storage_size: PlaneSize,
    stride_samples: usize,
    visible_rect: PlaneRect,
    required_samples: usize,
    allocation_bytes: usize,
    samples: Vec<T>,
}

impl<T: ReconSample> Plane<T> {
    /// Creates a plane from owned sample storage.
    ///
    /// `storage_size` describes the full rectangular backing storage.
    /// `visible_rect` selects the decoded output region inside that backing
    /// storage. Padding and stride samples remain owned by the plane but are
    /// not part of visible decoded output.
    ///
    /// # Errors
    /// Returns a [`ReconError`] if the stride is too small, arithmetic
    /// overflows, `samples.len()` does not match the derived backing length, or
    /// `visible_rect` falls outside `storage_size`.
    pub fn from_vec(
        storage_size: PlaneSize,
        stride_samples: usize,
        visible_rect: PlaneRect,
        samples: Vec<T>,
    ) -> Result<Self> {
        if stride_samples < storage_size.width() {
            return Err(ReconError::StrideTooSmall {
                stride_samples,
                storage_width: storage_size.width(),
            });
        }

        visible_rect.ensure_within(storage_size)?;

        let required_samples = stride_samples.checked_mul(storage_size.height()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "plane required sample count",
            },
        )?;
        if samples.len() != required_samples {
            return Err(ReconError::BufferLengthMismatch {
                expected: required_samples,
                actual: samples.len(),
            });
        }

        let allocation_bytes = required_samples.checked_mul(mem::size_of::<T>()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "plane allocation byte count",
            },
        )?;

        Ok(Self {
            storage_size,
            stride_samples,
            visible_rect,
            required_samples,
            allocation_bytes,
            samples,
        })
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

    /// Returns the visible decoded-output size.
    pub const fn visible_size(&self) -> PlaneSize {
        self.visible_rect.size()
    }

    /// Returns the required backing sample count.
    pub const fn required_samples(&self) -> usize {
        self.required_samples
    }

    /// Returns the backing allocation size in bytes for the sample type.
    pub const fn allocation_bytes(&self) -> usize {
        self.allocation_bytes
    }

    /// Returns the complete backing sample buffer, including padding.
    pub fn samples(&self) -> &[T] {
        &self.samples
    }

    /// Consumes the plane and returns the complete backing sample buffer.
    pub fn into_samples(self) -> Vec<T> {
        self.samples
    }

    /// Iterates over visible decoded-output rows, excluding padding and stride.
    pub const fn visible_rows(&self) -> VisibleRows<'_, T> {
        VisibleRows {
            plane: self,
            next_row: 0,
        }
    }

    /// Borrows this plane's storage as an immutable [`PlaneRef`] without copying.
    pub fn as_plane_ref(&self) -> PlaneRef<'_, T> {
        PlaneRef::from_parts(&self.samples, self.stride_samples, self.visible_rect)
    }
}

/// Iterator over visible decoded-output rows in a [`Plane`].
#[derive(Clone, Debug)]
pub struct VisibleRows<'a, T: ReconSample> {
    plane: &'a Plane<T>,
    next_row: usize,
}

impl<'a, T: ReconSample> Iterator for VisibleRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.plane.visible_rect.height() {
            return None;
        }

        let source_row = self.plane.visible_rect.y() + self.next_row;
        let start = source_row * self.plane.stride_samples + self.plane.visible_rect.x();
        let end = start + self.plane.visible_rect.width();
        self.next_row += 1;
        Some(&self.plane.samples[start..end])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.plane.visible_rect.height() - self.next_row;
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for VisibleRows<'_, T> {}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    #[test]
    fn plane_rejects_stride_smaller_than_storage_width() {
        let storage = size(4, 2);
        let visible = rect(0, 0, 4, 2);
        assert!(matches!(
            Plane::from_vec(storage, 3, visible, vec![0_u8; 6]),
            Err(ReconError::StrideTooSmall {
                stride_samples: 3,
                storage_width: 4
            })
        ));
    }

    #[test]
    fn plane_rejects_buffer_length_mismatch() {
        let storage = size(4, 2);
        let visible = rect(0, 0, 4, 2);
        assert!(matches!(
            Plane::from_vec(storage, 4, visible, vec![0_u8; 7]),
            Err(ReconError::BufferLengthMismatch {
                expected: 8,
                actual: 7
            })
        ));
    }

    #[test]
    fn plane_rejects_overflowing_required_sample_count() {
        let storage = size(2, 2);
        let visible = rect(0, 0, 1, 1);
        assert!(matches!(
            Plane::<u8>::from_vec(storage, usize::MAX, visible, Vec::new()),
            Err(ReconError::ArithmeticOverflow {
                context: "plane required sample count"
            })
        ));
    }

    #[test]
    fn plane_rejects_visible_rectangle_outside_storage() {
        let storage = size(4, 2);
        let visible = rect(2, 0, 3, 1);
        assert!(matches!(
            Plane::from_vec(storage, 4, visible, vec![0_u8; 8]),
            Err(ReconError::VisibleRectOutOfBounds { .. })
        ));
    }

    #[test]
    fn visible_rows_exclude_stride_and_padding() {
        let storage = size(4, 3);
        let visible = rect(1, 1, 2, 2);
        let samples = vec![0_u8, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23];
        let plane = Plane::from_vec(storage, 4, visible, samples).unwrap();

        assert_eq!(plane.required_samples(), 12);
        assert_eq!(plane.allocation_bytes(), 12);
        let rows: Vec<&[u8]> = plane.visible_rows().collect();
        assert_eq!(rows, vec![&[11, 12][..], &[21, 22][..]]);
    }

    #[test]
    fn allocation_bytes_use_sample_storage_type() {
        let storage = size(2, 2);
        let visible = rect(0, 0, 2, 2);
        let plane = Plane::from_vec(storage, 2, visible, vec![0_u16; 4]).unwrap();
        assert_eq!(plane.required_samples(), 4);
        assert_eq!(plane.allocation_bytes(), 8);
    }
}
