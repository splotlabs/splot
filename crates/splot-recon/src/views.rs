// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Borrowed plane and frame views.
//!
//! These view types are the zero-copy default for reading and writing decoded
//! media (see [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)). They borrow
//! existing sample storage, validate stride/visible-rect/length on construction,
//! and never allocate or copy samples. Owned storage types (`Plane`,
//! `DecodedFrame`, `CurrentFramePlane`, `CurrentFrameWorkspace`) hand out these
//! views without copying their backing buffers.
//!
//! Feature tracking: `INFRA-ZERO-COPY-MEDIA-POLICY`.

use crate::{DecodedFrameInfo, PlaneId, PlaneRect, PlaneSize, ReconError, ReconSample, Result};

/// Validates that a visible rectangle fits a borrowed sample buffer at `stride`.
///
/// Shared by every view constructor: the visible columns must fit within one
/// stride row, and the last visible sample must lie inside the borrowed storage.
fn validate_plane_view(
    samples_len: usize,
    stride_samples: usize,
    visible_rect: PlaneRect,
) -> Result<()> {
    let row_end = visible_rect.x().checked_add(visible_rect.width()).ok_or(
        ReconError::ArithmeticOverflow {
            context: "plane view visible row end",
        },
    )?;
    if stride_samples < row_end {
        return Err(ReconError::StrideTooSmall {
            stride_samples,
            storage_width: row_end,
        });
    }

    let bottom = visible_rect.y().checked_add(visible_rect.height()).ok_or(
        ReconError::ArithmeticOverflow {
            context: "plane view bottom row",
        },
    )?;
    let last_row = bottom - 1;
    let last_row_start =
        last_row
            .checked_mul(stride_samples)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "plane view last row offset",
            })?;
    let required = last_row_start
        .checked_add(row_end)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "plane view required length",
        })?;
    if required > samples_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: required,
            actual: samples_len,
        });
    }
    Ok(())
}

/// Borrowed immutable view over plane sample storage.
///
/// Holds a shared borrow of the backing samples plus the stride and visible
/// rectangle. Construction validates geometry and never allocates or copies.
#[derive(Clone, Copy, Debug)]
pub struct PlaneRef<'a, T: ReconSample> {
    samples: &'a [T],
    stride_samples: usize,
    visible_rect: PlaneRect,
}

impl<'a, T: ReconSample> PlaneRef<'a, T> {
    /// Borrows `samples` as an immutable plane view.
    ///
    /// # Errors
    /// Returns a [`ReconError`] when the stride is smaller than the visible row
    /// width, arithmetic overflows, or the visible rectangle falls outside the
    /// borrowed buffer.
    pub fn new(samples: &'a [T], stride_samples: usize, visible_rect: PlaneRect) -> Result<Self> {
        validate_plane_view(samples.len(), stride_samples, visible_rect)?;
        Ok(Self {
            samples,
            stride_samples,
            visible_rect,
        })
    }

    /// Borrows already-validated parts without rechecking (owned-type accessors).
    pub(crate) const fn from_parts(
        samples: &'a [T],
        stride_samples: usize,
        visible_rect: PlaneRect,
    ) -> Self {
        Self {
            samples,
            stride_samples,
            visible_rect,
        }
    }

    /// Returns the storage stride in samples.
    pub const fn stride_samples(&self) -> usize {
        self.stride_samples
    }

    /// Returns the visible rectangle.
    pub const fn visible_rect(&self) -> PlaneRect {
        self.visible_rect
    }

    /// Returns the visible size.
    pub const fn visible_size(&self) -> PlaneSize {
        self.visible_rect.size()
    }

    /// Returns the full borrowed sample buffer, including stride padding.
    pub const fn samples(&self) -> &'a [T] {
        self.samples
    }

    /// Iterates over visible rows, excluding stride padding.
    pub const fn visible_rows(&self) -> PlaneRefRows<'a, T> {
        PlaneRefRows {
            samples: self.samples,
            stride_samples: self.stride_samples,
            visible_rect: self.visible_rect,
            next_row: 0,
        }
    }
}

/// Iterator over visible rows of a [`PlaneRef`].
#[derive(Clone, Debug)]
pub struct PlaneRefRows<'a, T: ReconSample> {
    samples: &'a [T],
    stride_samples: usize,
    visible_rect: PlaneRect,
    next_row: usize,
}

impl<'a, T: ReconSample> Iterator for PlaneRefRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.visible_rect.height() {
            return None;
        }
        let row = self.visible_rect.y() + self.next_row;
        let start = row * self.stride_samples + self.visible_rect.x();
        let end = start + self.visible_rect.width();
        self.next_row += 1;
        Some(&self.samples[start..end])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.visible_rect.height() - self.next_row;
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for PlaneRefRows<'_, T> {}

/// Borrowed exclusive view over plane sample storage.
///
/// Holds a unique borrow of the backing samples plus the stride and visible
/// rectangle. Construction validates geometry and never allocates or copies. The
/// exclusive borrow is what makes disjoint parallel writes sound.
#[derive(Debug)]
pub struct PlaneMut<'a, T: ReconSample> {
    samples: &'a mut [T],
    stride_samples: usize,
    visible_rect: PlaneRect,
}

impl<'a, T: ReconSample> PlaneMut<'a, T> {
    /// Borrows `samples` as an exclusive plane view.
    ///
    /// # Errors
    /// Returns a [`ReconError`] when the stride is smaller than the visible row
    /// width, arithmetic overflows, or the visible rectangle falls outside the
    /// borrowed buffer.
    pub fn new(
        samples: &'a mut [T],
        stride_samples: usize,
        visible_rect: PlaneRect,
    ) -> Result<Self> {
        validate_plane_view(samples.len(), stride_samples, visible_rect)?;
        Ok(Self {
            samples,
            stride_samples,
            visible_rect,
        })
    }

    /// Borrows already-validated parts without rechecking (owned-type accessors).
    pub(crate) const fn from_parts(
        samples: &'a mut [T],
        stride_samples: usize,
        visible_rect: PlaneRect,
    ) -> Self {
        Self {
            samples,
            stride_samples,
            visible_rect,
        }
    }

    /// Returns the storage stride in samples.
    pub const fn stride_samples(&self) -> usize {
        self.stride_samples
    }

    /// Returns the visible rectangle.
    pub const fn visible_rect(&self) -> PlaneRect {
        self.visible_rect
    }

    /// Returns the visible size.
    pub const fn visible_size(&self) -> PlaneSize {
        self.visible_rect.size()
    }

    /// Returns the full borrowed sample buffer, including stride padding.
    pub fn samples(&self) -> &[T] {
        self.samples
    }

    /// Consumes the view, returning the full backing sample slice with the
    /// view's own lifetime, so callers can partition it (for example with
    /// `chunks_mut`) into disjoint regions that outlive the view value.
    /// Callers own bit-depth range enforcement, exactly as with
    /// [`Self::visible_rows_mut`].
    pub fn into_samples(self) -> &'a mut [T] {
        self.samples
    }

    /// Returns the full borrowed sample buffer mutably, including stride
    /// padding. Callers own bit-depth range enforcement, exactly as with
    /// [`Self::visible_rows_mut`].
    pub fn samples_mut(&mut self) -> &mut [T] {
        self.samples
    }

    /// Borrows the plane immutably as a [`PlaneRef`].
    pub fn as_plane_ref(&self) -> PlaneRef<'_, T> {
        PlaneRef::from_parts(self.samples, self.stride_samples, self.visible_rect)
    }

    /// Iterates over visible rows as exclusive slices, excluding stride padding.
    pub fn visible_rows_mut(&mut self) -> PlaneMutRows<'_, T> {
        let start = self.visible_rect.y() * self.stride_samples;
        PlaneMutRows {
            rest: Some(&mut self.samples[start..]),
            stride_samples: self.stride_samples,
            x: self.visible_rect.x(),
            width: self.visible_rect.width(),
            rows_left: self.visible_rect.height(),
        }
    }
}

/// Iterator over visible rows of a [`PlaneMut`] as disjoint exclusive slices.
///
/// Each yielded `&mut [T]` is a distinct, non-overlapping visible row, produced
/// safely by repeatedly splitting the remaining borrow with `split_at_mut`.
#[derive(Debug)]
pub struct PlaneMutRows<'a, T: ReconSample> {
    rest: Option<&'a mut [T]>,
    stride_samples: usize,
    x: usize,
    width: usize,
    rows_left: usize,
}

impl<'a, T: ReconSample> Iterator for PlaneMutRows<'a, T> {
    type Item = &'a mut [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.rows_left == 0 {
            return None;
        }
        let rest = self.rest.take()?;
        self.rows_left -= 1;
        let take = self.stride_samples.min(rest.len());
        let (row, tail) = rest.split_at_mut(take);
        self.rest = Some(tail);
        Some(&mut row[self.x..self.x + self.width])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rows_left, Some(self.rows_left))
    }
}

impl<T: ReconSample> ExactSizeIterator for PlaneMutRows<'_, T> {}

/// Expected chroma visible size derived from frame metadata, or `None` for
/// monochrome. Mirrors the derivation in `DecodedFrame::try_new`.
fn expected_chroma_visible_size(info: DecodedFrameInfo) -> Result<Option<PlaneSize>> {
    info.pixel_format()
        .chroma_size(info.visible_luma_rect().size())
}

/// Validates view plane presence and visible geometry against frame metadata.
///
/// Shared by [`FrameRef::new`] and [`FrameMut::new`]. The closures read each
/// candidate plane's visible size without taking ownership of the view.
fn validate_frame_planes(
    info: DecodedFrameInfo,
    y_visible: PlaneSize,
    u_visible: Option<PlaneSize>,
    v_visible: Option<PlaneSize>,
) -> Result<()> {
    let luma_visible = info.visible_luma_rect().size();
    if y_visible != luma_visible {
        return Err(ReconError::PlaneSizeMismatch {
            plane: PlaneId::Y,
            expected: luma_visible,
            actual: y_visible,
        });
    }

    match expected_chroma_visible_size(info)? {
        None => {
            if u_visible.is_some() {
                return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U });
            }
            if v_visible.is_some() {
                return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::V });
            }
        }
        Some(chroma_visible) => {
            check_chroma_plane(PlaneId::U, u_visible, chroma_visible)?;
            check_chroma_plane(PlaneId::V, v_visible, chroma_visible)?;
        }
    }
    Ok(())
}

fn check_chroma_plane(
    plane: PlaneId,
    visible: Option<PlaneSize>,
    expected: PlaneSize,
) -> Result<()> {
    let Some(visible) = visible else {
        return Err(ReconError::MissingChromaPlane { plane });
    };
    if visible != expected {
        return Err(ReconError::PlaneSizeMismatch {
            plane,
            expected,
            actual: visible,
        });
    }
    Ok(())
}

/// Borrowed immutable view over a full decoded frame.
///
/// Mirrors `DecodedFrame`'s plane layout with [`PlaneRef`] planes. Construction
/// validates plane presence and visible geometry against the frame metadata.
#[derive(Clone, Copy, Debug)]
pub struct FrameRef<'a, T: ReconSample> {
    info: DecodedFrameInfo,
    y: PlaneRef<'a, T>,
    u: Option<PlaneRef<'a, T>>,
    v: Option<PlaneRef<'a, T>>,
}

impl<'a, T: ReconSample> FrameRef<'a, T> {
    /// Builds a frame view after validating plane presence and visible geometry.
    ///
    /// # Errors
    /// Returns a [`ReconError`] when chroma plane presence or any visible plane
    /// size disagrees with the frame metadata.
    pub fn new(
        info: DecodedFrameInfo,
        y: PlaneRef<'a, T>,
        u: Option<PlaneRef<'a, T>>,
        v: Option<PlaneRef<'a, T>>,
    ) -> Result<Self> {
        validate_frame_planes(
            info,
            y.visible_size(),
            u.as_ref().map(PlaneRef::visible_size),
            v.as_ref().map(PlaneRef::visible_size),
        )?;
        Ok(Self { info, y, u, v })
    }

    /// Builds a frame view from already-validated parts (owned-type accessors).
    pub(crate) const fn from_parts(
        info: DecodedFrameInfo,
        y: PlaneRef<'a, T>,
        u: Option<PlaneRef<'a, T>>,
        v: Option<PlaneRef<'a, T>>,
    ) -> Self {
        Self { info, y, u, v }
    }

    /// Returns the decoded frame metadata.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the luma plane view.
    pub const fn y(&self) -> PlaneRef<'a, T> {
        self.y
    }

    /// Returns the U plane view when present.
    pub const fn u(&self) -> Option<PlaneRef<'a, T>> {
        self.u
    }

    /// Returns the V plane view when present.
    pub const fn v(&self) -> Option<PlaneRef<'a, T>> {
        self.v
    }

    /// Returns a plane view by identifier.
    pub const fn plane(&self, plane: PlaneId) -> Option<PlaneRef<'a, T>> {
        match plane {
            PlaneId::Y => Some(self.y),
            PlaneId::U => self.u,
            PlaneId::V => self.v,
        }
    }
}

/// Borrowed exclusive view over a full decoded frame.
///
/// Mirrors [`FrameRef`] with [`PlaneMut`] planes. Construction validates plane
/// presence and visible geometry against the frame metadata.
#[derive(Debug)]
pub struct FrameMut<'a, T: ReconSample> {
    info: DecodedFrameInfo,
    y: PlaneMut<'a, T>,
    u: Option<PlaneMut<'a, T>>,
    v: Option<PlaneMut<'a, T>>,
}

impl<'a, T: ReconSample> FrameMut<'a, T> {
    /// Builds an exclusive frame view after validating presence and geometry.
    ///
    /// # Errors
    /// Returns a [`ReconError`] when chroma plane presence or any visible plane
    /// size disagrees with the frame metadata.
    pub fn new(
        info: DecodedFrameInfo,
        y: PlaneMut<'a, T>,
        u: Option<PlaneMut<'a, T>>,
        v: Option<PlaneMut<'a, T>>,
    ) -> Result<Self> {
        validate_frame_planes(
            info,
            y.visible_size(),
            u.as_ref().map(PlaneMut::visible_size),
            v.as_ref().map(PlaneMut::visible_size),
        )?;
        Ok(Self { info, y, u, v })
    }

    /// Builds an exclusive frame view from already-validated parts.
    pub(crate) const fn from_parts(
        info: DecodedFrameInfo,
        y: PlaneMut<'a, T>,
        u: Option<PlaneMut<'a, T>>,
        v: Option<PlaneMut<'a, T>>,
    ) -> Self {
        Self { info, y, u, v }
    }

    /// Returns the decoded frame metadata.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the luma plane view.
    pub const fn y(&self) -> &PlaneMut<'a, T> {
        &self.y
    }

    /// Returns the U plane view when present.
    pub fn u(&self) -> Option<&PlaneMut<'a, T>> {
        self.u.as_ref()
    }

    /// Returns the V plane view when present.
    pub fn v(&self) -> Option<&PlaneMut<'a, T>> {
        self.v.as_ref()
    }

    /// Returns a plane view by identifier.
    pub fn plane(&self, plane: PlaneId) -> Option<&PlaneMut<'a, T>> {
        match plane {
            PlaneId::Y => Some(&self.y),
            PlaneId::U => self.u.as_ref(),
            PlaneId::V => self.v.as_ref(),
        }
    }

    /// Returns the luma plane view for exclusive access.
    pub fn y_mut(&mut self) -> &mut PlaneMut<'a, T> {
        &mut self.y
    }

    /// Returns the U plane view for exclusive access when present.
    pub fn u_mut(&mut self) -> Option<&mut PlaneMut<'a, T>> {
        self.u.as_mut()
    }

    /// Returns the V plane view for exclusive access when present.
    pub fn v_mut(&mut self) -> Option<&mut PlaneMut<'a, T>> {
        self.v.as_mut()
    }

    /// Returns a plane view by identifier for exclusive access.
    pub fn plane_mut(&mut self, plane: PlaneId) -> Option<&mut PlaneMut<'a, T>> {
        match plane {
            PlaneId::Y => Some(&mut self.y),
            PlaneId::U => self.u.as_mut(),
            PlaneId::V => self.v.as_mut(),
        }
    }

    /// Splits the exclusive frame view into its disjoint per-plane views.
    ///
    /// The three planes borrow distinct storage, so callers can partition
    /// each plane (for example into row bands) for parallel work without
    /// aliasing.
    pub fn into_planes(
        self,
    ) -> (
        PlaneMut<'a, T>,
        Option<PlaneMut<'a, T>>,
        Option<PlaneMut<'a, T>>,
    ) {
        (self.y, self.u, self.v)
    }

    /// Borrows the frame immutably as a [`FrameRef`].
    pub fn as_frame_ref(&self) -> FrameRef<'_, T> {
        FrameRef::from_parts(
            self.info,
            self.y.as_plane_ref(),
            self.u.as_ref().map(PlaneMut::as_plane_ref),
            self.v.as_ref().map(PlaneMut::as_plane_ref),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{BitDepth, DecodedFrame, FramePlanes, OutputIndex, PixelFormat, Plane};

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn info(format: PixelFormat, coded: PlaneSize, visible: PlaneRect) -> DecodedFrameInfo {
        DecodedFrameInfo::new(OutputIndex::new(0), BitDepth::Eight, format, coded, visible).unwrap()
    }

    #[test]
    fn plane_ref_borrows_without_copying_and_excludes_padding() {
        let samples: [u8; 12] = [0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23];
        let view = PlaneRef::new(&samples, 4, rect(1, 1, 2, 2)).unwrap();
        assert_eq!(view.visible_size(), size(2, 2));
        assert_eq!(view.samples().as_ptr(), samples.as_ptr());
        let rows: Vec<&[u8]> = view.visible_rows().collect();
        assert_eq!(rows, vec![&[11, 12][..], &[21, 22][..]]);
    }

    #[test]
    fn plane_ref_rejects_bad_stride_and_visible_rectangle_past_buffer() {
        let samples = [0_u8; 6];
        assert!(matches!(
            PlaneRef::new(&samples, 2, rect(0, 0, 3, 2)),
            Err(ReconError::StrideTooSmall {
                stride_samples: 2,
                storage_width: 3
            })
        ));
        assert!(matches!(
            PlaneRef::new(&samples, 3, rect(0, 0, 3, 3)),
            Err(ReconError::BufferLengthMismatch {
                expected: 9,
                actual: 6
            })
        ));
    }

    #[test]
    fn plane_mut_writes_disjoint_rows_and_reads_back() {
        let mut samples: [u8; 12] = [0; 12];
        let mut view = PlaneMut::new(&mut samples, 4, rect(1, 1, 2, 2)).unwrap();
        for (row_index, row) in view.visible_rows_mut().enumerate() {
            for (col_index, sample) in row.iter_mut().enumerate() {
                *sample = (row_index * 10 + col_index) as u8;
            }
        }
        let read_back: Vec<&[u8]> = view.as_plane_ref().visible_rows().collect();
        assert_eq!(read_back, vec![&[0, 1][..], &[10, 11][..]]);
        assert_eq!(samples, [0, 0, 0, 0, 0, 0, 1, 0, 0, 10, 11, 0]);
    }

    #[test]
    fn plane_mut_construction_validates_geometry() {
        let mut samples = [0_u8; 6];
        assert!(matches!(
            PlaneMut::new(&mut samples, 2, rect(0, 0, 3, 2)),
            Err(ReconError::StrideTooSmall { .. })
        ));
    }

    fn owned_plane(width: usize, height: usize, value: u8) -> Plane<u8> {
        Plane::from_vec(
            size(width, height),
            width,
            rect(0, 0, width, height),
            vec![value; width * height],
        )
        .unwrap()
    }

    fn owned_yuv420_frame() -> DecodedFrame<u8> {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let planes = FramePlanes::new(
            owned_plane(4, 4, 7),
            Some(owned_plane(2, 2, 8)),
            Some(owned_plane(2, 2, 9)),
        );
        DecodedFrame::try_new(frame_info, planes).unwrap()
    }

    #[test]
    fn owned_frame_exposes_borrowed_view_without_copy() {
        let frame = owned_yuv420_frame();
        let view = frame.as_frame_ref();
        assert_eq!(view.info().output_index(), frame.output_index());
        assert_eq!(view.y().samples().as_ptr(), frame.y().samples().as_ptr());
        assert_eq!(view.u().map(|p| p.visible_size()), Some(size(2, 2)));
        assert_eq!(view.v().map(|p| p.visible_size()), Some(size(2, 2)));
    }

    #[test]
    fn frame_ref_rejects_missing_chroma_plane() {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let y_samples = [0_u8; 16];
        let y = PlaneRef::new(&y_samples, 4, rect(0, 0, 4, 4)).unwrap();
        assert!(matches!(
            FrameRef::new(frame_info, y, None, None),
            Err(ReconError::MissingChromaPlane { plane: PlaneId::U })
        ));
    }

    #[test]
    fn frame_ref_rejects_wrong_chroma_visible_size() {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let y_samples = [0_u8; 16];
        let chroma_samples = [0_u8; 4];
        let wrong_chroma = [0_u8; 1];
        let y = PlaneRef::new(&y_samples, 4, rect(0, 0, 4, 4)).unwrap();
        let u = PlaneRef::new(&chroma_samples, 2, rect(0, 0, 2, 2)).unwrap();
        let v = PlaneRef::new(&wrong_chroma, 1, rect(0, 0, 1, 1)).unwrap();
        assert!(matches!(
            FrameRef::new(frame_info, y, Some(u), Some(v)),
            Err(ReconError::PlaneSizeMismatch {
                plane: PlaneId::V,
                ..
            })
        ));
    }

    #[test]
    fn frame_ref_rejects_chroma_plane_for_monochrome() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 4), rect(0, 0, 4, 4));
        let y_samples = [0_u8; 16];
        let chroma = [0_u8; 4];
        let y = PlaneRef::new(&y_samples, 4, rect(0, 0, 4, 4)).unwrap();
        let u = PlaneRef::new(&chroma, 2, rect(0, 0, 2, 2)).unwrap();
        assert!(matches!(
            FrameRef::new(frame_info, y, Some(u), None),
            Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U })
        ));
    }

    #[test]
    fn frame_mut_exposes_immutable_plane_accessors_mirroring_frame_ref() {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let mut y_samples = [0_u8; 16];
        let mut u_samples = [0_u8; 4];
        let mut v_samples = [0_u8; 4];
        let y = PlaneMut::new(&mut y_samples, 4, rect(0, 0, 4, 4)).unwrap();
        let u = PlaneMut::new(&mut u_samples, 2, rect(0, 0, 2, 2)).unwrap();
        let v = PlaneMut::new(&mut v_samples, 2, rect(0, 0, 2, 2)).unwrap();
        let frame = FrameMut::new(frame_info, y, Some(u), Some(v)).unwrap();
        assert_eq!(frame.u().map(PlaneMut::visible_size), Some(size(2, 2)));
        assert_eq!(frame.v().map(PlaneMut::visible_size), Some(size(2, 2)));
        assert!(frame.plane(PlaneId::Y).is_some());
        assert!(frame.plane(PlaneId::U).is_some());
        assert_eq!(frame.info().output_index().get(), 0);
    }
}
