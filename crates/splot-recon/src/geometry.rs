// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Geometry primitives for decoded output planes.

use crate::{ReconError, Result};

/// Positive two-dimensional plane size in samples.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlaneSize {
    width: usize,
    height: usize,
}

impl PlaneSize {
    /// Creates a positive plane size.
    ///
    /// # Errors
    /// Returns [`ReconError::ZeroDimension`] if either dimension is zero.
    pub const fn new(width: usize, height: usize) -> Result<Self> {
        if width == 0 {
            return Err(ReconError::ZeroDimension {
                field: "plane width",
            });
        }
        if height == 0 {
            return Err(ReconError::ZeroDimension {
                field: "plane height",
            });
        }
        Ok(Self { width, height })
    }

    /// Creates a positive plane size without rechecking invariants.
    pub(crate) const fn new_unchecked(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Returns width in samples.
    pub const fn width(self) -> usize {
        self.width
    }

    /// Returns height in samples.
    pub const fn height(self) -> usize {
        self.height
    }
}

/// Positive visible rectangle within a plane storage rectangle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlaneRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl PlaneRect {
    /// Creates a positive visible rectangle.
    ///
    /// # Errors
    /// Returns [`ReconError::ZeroDimension`] if width or height is zero.
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Result<Self> {
        if width == 0 {
            return Err(ReconError::ZeroDimension {
                field: "visible width",
            });
        }
        if height == 0 {
            return Err(ReconError::ZeroDimension {
                field: "visible height",
            });
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the x origin in samples.
    pub const fn x(self) -> usize {
        self.x
    }

    /// Returns the y origin in samples.
    pub const fn y(self) -> usize {
        self.y
    }

    /// Returns the visible width in samples.
    pub const fn width(self) -> usize {
        self.width
    }

    /// Returns the visible height in samples.
    pub const fn height(self) -> usize {
        self.height
    }

    /// Returns the visible rectangle size.
    pub const fn size(self) -> PlaneSize {
        PlaneSize::new_unchecked(self.width, self.height)
    }

    /// Returns whether this rectangle is fully inside `storage`.
    pub fn is_within(self, storage: PlaneSize) -> bool {
        let Some(right) = self.x.checked_add(self.width) else {
            return false;
        };
        let Some(bottom) = self.y.checked_add(self.height) else {
            return false;
        };

        right <= storage.width() && bottom <= storage.height()
    }

    /// Validates that this rectangle is fully inside `storage`.
    ///
    /// # Errors
    /// Returns [`ReconError::VisibleRectOutOfBounds`] when the rectangle
    /// exceeds the storage bounds.
    pub fn ensure_within(self, storage: PlaneSize) -> Result<()> {
        if self.is_within(storage) {
            Ok(())
        } else {
            Err(ReconError::VisibleRectOutOfBounds {
                storage,
                rect: self,
            })
        }
    }
}

/// Repository-owned zero-based decoded output emission index.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OutputIndex(u64);

impl OutputIndex {
    /// Creates an output emission index.
    pub const fn new(index: u64) -> Self {
        Self(index)
    }

    /// Returns the zero-based output emission index.
    pub const fn get(self) -> u64 {
        self.0
    }
}

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
    fn sizes_and_rectangles_reject_zero_visible_dimensions() {
        assert!(matches!(
            PlaneSize::new(0, 1),
            Err(ReconError::ZeroDimension {
                field: "plane width"
            })
        ));
        assert!(matches!(
            PlaneSize::new(1, 0),
            Err(ReconError::ZeroDimension {
                field: "plane height"
            })
        ));
        assert!(matches!(
            PlaneRect::new(0, 0, 0, 1),
            Err(ReconError::ZeroDimension {
                field: "visible width"
            })
        ));
        assert!(matches!(
            PlaneRect::new(0, 0, 1, 0),
            Err(ReconError::ZeroDimension {
                field: "visible height"
            })
        ));
    }

    #[test]
    fn visible_rect_must_fit_storage() {
        let storage = size(4, 3);
        assert!(rect(1, 1, 3, 2).is_within(storage));
        assert!(matches!(
            rect(2, 1, 3, 2).ensure_within(storage),
            Err(ReconError::VisibleRectOutOfBounds { .. })
        ));
        assert!(matches!(
            rect(usize::MAX, 0, 1, 1).ensure_within(storage),
            Err(ReconError::VisibleRectOutOfBounds { .. })
        ));
    }

    #[test]
    fn output_index_is_zero_based_counter() {
        assert_eq!(OutputIndex::new(0).get(), 0);
        assert_eq!(OutputIndex::new(17).get(), 17);
    }
}
