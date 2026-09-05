// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Immutable decoded output frame model.

use std::sync::Arc;

use crate::intra_dc_math::validate_sample_type;
use crate::{
    BitDepth, FrameRef, OutputIndex, PixelFormat, Plane, PlaneId, PlaneRect, PlaneSize, ReconError,
    ReconSample, Result,
};

/// Decoded output frame metadata shared by all planes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DecodedFrameInfo {
    output_index: OutputIndex,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    coded_luma_size: PlaneSize,
    visible_luma_rect: PlaneRect,
}

impl DecodedFrameInfo {
    /// Creates decoded output frame metadata after validating crop geometry.
    ///
    /// # Errors
    /// Returns [`ReconError::VisibleRectOutOfBounds`] when the visible luma
    /// rectangle exceeds the coded luma dimensions, or
    /// [`ReconError::CropOriginNotAligned`] when a non-monochrome crop origin
    /// is not aligned to the AV2 § 6.4.1 subsampling factors.
    pub fn new(
        output_index: OutputIndex,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        coded_luma_size: PlaneSize,
        visible_luma_rect: PlaneRect,
    ) -> Result<Self> {
        visible_luma_rect.ensure_within(coded_luma_size)?;
        validate_crop_alignment(pixel_format, visible_luma_rect)?;

        Ok(Self {
            output_index,
            bit_depth,
            pixel_format,
            coded_luma_size,
            visible_luma_rect,
        })
    }

    /// Returns the repository-owned zero-based output emission index.
    pub const fn output_index(self) -> OutputIndex {
        self.output_index
    }

    /// Returns the decoded sample bit depth.
    pub const fn bit_depth(self) -> BitDepth {
        self.bit_depth
    }

    /// Returns the decoded output pixel format.
    pub const fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns the coded luma frame size from AV2 § 6.17.4.1.
    pub const fn coded_luma_size(self) -> PlaneSize {
        self.coded_luma_size
    }

    /// Returns the visible luma crop rectangle.
    pub const fn visible_luma_rect(self) -> PlaneRect {
        self.visible_luma_rect
    }
}

/// Candidate planes for a decoded output frame.
///
/// Does not implement `Clone`: it owns the plane sample buffers (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Eq, PartialEq)]
pub struct FramePlanes<T: ReconSample> {
    y: Plane<T>,
    u: Option<Plane<T>>,
    v: Option<Plane<T>>,
}

impl<T: ReconSample> FramePlanes<T> {
    /// Creates a plane set for later decoded-frame validation.
    pub const fn new(y: Plane<T>, u: Option<Plane<T>>, v: Option<Plane<T>>) -> Self {
        Self { y, u, v }
    }

    /// Returns the Y plane.
    pub const fn y(&self) -> &Plane<T> {
        &self.y
    }

    /// Returns the U plane when present.
    pub const fn u(&self) -> Option<&Plane<T>> {
        self.u.as_ref()
    }

    /// Returns the V plane when present.
    pub const fn v(&self) -> Option<&Plane<T>> {
        self.v.as_ref()
    }

    /// Returns a plane by identifier.
    pub const fn plane(&self, plane: PlaneId) -> Option<&Plane<T>> {
        match plane {
            PlaneId::Y => Some(&self.y),
            PlaneId::U => self.u.as_ref(),
            PlaneId::V => self.v.as_ref(),
        }
    }
}

/// The sample buffers of one frame's planes, kept for the next frame's.
///
/// A frame that no reference slot holds any more still owns three sizeable
/// allocations. Handing them over here lets the frame that takes its place in
/// the slot reuse them instead of asking the allocator for the same bytes
/// again, which is what dav2d's picture pool does for the same lifetime.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct FramePlaneSamples<T: ReconSample> {
    planes: [Vec<T>; 3],
}

impl<T: ReconSample> FramePlaneSamples<T> {
    /// Collects one frame's plane buffers, absent chroma included.
    #[must_use]
    pub fn new(y: Vec<T>, u: Option<Vec<T>>, v: Option<Vec<T>>) -> Self {
        Self {
            planes: [y, u.unwrap_or_default(), v.unwrap_or_default()],
        }
    }

    /// Takes the buffer kept for `plane`, leaving nothing behind.
    #[must_use]
    pub fn take(&mut self, plane: PlaneId) -> Vec<T> {
        core::mem::take(&mut self.planes[plane.index()])
    }
}

/// One frame's retired plane buffers, in whichever storage depth it decoded to.
///
/// The pipeline hands buffers between frames through channels that do not carry
/// the sample type, so the depth travels with the buffers.
#[derive(Debug, Default)]
pub enum RetiredFramePlanes {
    /// No frame has handed its buffers over yet.
    #[default]
    None,
    /// Eight-bit sample storage.
    Eight(FramePlaneSamples<u8>),
    /// Ten-bit sample storage.
    Ten(FramePlaneSamples<u16>),
}

/// Immutable decoded output frame made of owned planes.
///
/// Does not implement `Clone`: it owns the frame's sample storage. Borrow it as a
/// [`FrameRef`] with [`DecodedFrame::as_frame_ref`], or share it without copying
/// pixels via [`SharedFrame`] (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Eq, PartialEq)]
pub struct DecodedFrame<T: ReconSample> {
    info: DecodedFrameInfo,
    planes: FramePlanes<T>,
}

impl<T: ReconSample> DecodedFrame<T> {
    /// Creates a decoded output frame after validating AV2-derived invariants.
    ///
    /// Chroma visible sizes are derived from the visible luma rectangle in
    /// `info` and the AV2 § 6.4.1 subsampling facts for its pixel format.
    ///
    /// # Errors
    /// Returns a [`ReconError`] if plane presence, visible plane sizes, sample
    /// type, or sample ranges do not match the requested decoded frame format.
    pub fn try_new(info: DecodedFrameInfo, planes: FramePlanes<T>) -> Result<Self> {
        validate_sample_type::<T>(info.bit_depth)?;

        let luma_visible_size = info.visible_luma_rect.size();
        validate_plane_size(PlaneId::Y, luma_visible_size, planes.y.visible_size())?;
        validate_plane_samples(PlaneId::Y, &planes.y, info.bit_depth.max_sample())?;

        match info.pixel_format.chroma_size(luma_visible_size)? {
            None => {
                if planes.u.is_some() {
                    return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U });
                }
                if planes.v.is_some() {
                    return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::V });
                }
            }
            Some(chroma_visible_size) => {
                let Some(u_plane) = planes.u.as_ref() else {
                    return Err(ReconError::MissingChromaPlane { plane: PlaneId::U });
                };
                let Some(v_plane) = planes.v.as_ref() else {
                    return Err(ReconError::MissingChromaPlane { plane: PlaneId::V });
                };

                validate_plane_size(PlaneId::U, chroma_visible_size, u_plane.visible_size())?;
                validate_plane_size(PlaneId::V, chroma_visible_size, v_plane.visible_size())?;
                validate_plane_samples(PlaneId::U, u_plane, info.bit_depth.max_sample())?;
                validate_plane_samples(PlaneId::V, v_plane, info.bit_depth.max_sample())?;
            }
        }

        Ok(Self { info, planes })
    }

    /// Hands this frame's sample buffers to the frame that replaces it.
    #[must_use]
    pub fn into_plane_samples(self) -> FramePlaneSamples<T> {
        let FramePlanes { y, u, v } = self.planes;
        FramePlaneSamples {
            planes: [
                y.into_samples(),
                u.map(Plane::into_samples).unwrap_or_default(),
                v.map(Plane::into_samples).unwrap_or_default(),
            ],
        }
    }

    /// Returns the decoded output frame metadata.
    pub const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the repository-owned zero-based output emission index.
    pub const fn output_index(&self) -> OutputIndex {
        self.info.output_index
    }

    /// Returns the decoded sample bit depth.
    pub const fn bit_depth(&self) -> BitDepth {
        self.info.bit_depth
    }

    /// Returns the decoded output pixel format.
    pub const fn pixel_format(&self) -> PixelFormat {
        self.info.pixel_format
    }

    /// Returns the coded luma frame size from AV2 § 6.17.4.1.
    pub const fn coded_luma_size(&self) -> PlaneSize {
        self.info.coded_luma_size
    }

    /// Returns the visible luma crop rectangle.
    pub const fn visible_luma_rect(&self) -> PlaneRect {
        self.info.visible_luma_rect
    }

    /// Returns the Y plane.
    pub const fn y(&self) -> &Plane<T> {
        &self.planes.y
    }

    /// Returns the U plane when present.
    pub const fn u(&self) -> Option<&Plane<T>> {
        self.planes.u.as_ref()
    }

    /// Returns the V plane when present.
    pub const fn v(&self) -> Option<&Plane<T>> {
        self.planes.v.as_ref()
    }

    /// Returns a plane by identifier.
    pub const fn plane(&self, plane: PlaneId) -> Option<&Plane<T>> {
        self.planes.plane(plane)
    }

    /// Borrows the whole frame as an immutable [`FrameRef`] without copying.
    pub fn as_frame_ref(&self) -> FrameRef<'_, T> {
        FrameRef::from_parts(
            self.info,
            self.planes.y().as_plane_ref(),
            self.planes.u().map(Plane::as_plane_ref),
            self.planes.v().map(Plane::as_plane_ref),
        )
    }
}

/// An immutable decoded frame shared without copying its pixels.
///
/// `SharedFrame` is the only way to give a second owner access to a decoded
/// frame's storage. It is `Arc`-backed and intentionally does **not** implement
/// `Clone`: sharing is always the explicit, review-visible [`SharedFrame::share`]
/// (an `Arc::clone`), never a hidden full-frame copy. It exposes no mutable
/// access to its storage and never uses copy-on-write (see
/// [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md)).
#[derive(Debug)]
pub struct SharedFrame<T: ReconSample> {
    inner: Arc<DecodedFrame<T>>,
}

impl<T: ReconSample> SharedFrame<T> {
    /// Wraps an owned decoded frame in a shareable handle.
    pub fn new(frame: DecodedFrame<T>) -> Self {
        Self {
            inner: Arc::new(frame),
        }
    }

    /// Returns a second handle to the same frame storage without copying pixels.
    ///
    /// This is the explicit, review-visible sharing operation (an `Arc::clone`);
    /// `SharedFrame` deliberately does not implement `Clone`.
    #[must_use]
    pub fn share(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Borrows the shared decoded frame.
    pub fn get(&self) -> &DecodedFrame<T> {
        &self.inner
    }

    /// Borrows the shared frame as an immutable [`FrameRef`] without copying.
    pub fn as_frame_ref(&self) -> FrameRef<'_, T> {
        self.inner.as_frame_ref()
    }

    /// Takes the frame back when this is its last handle.
    ///
    /// Returns `None` while any other handle is still sharing the storage, so a
    /// caller reclaiming a frame's buffers cannot take them from a live reader.
    #[must_use]
    pub fn into_frame(self) -> Option<DecodedFrame<T>> {
        Arc::into_inner(self.inner)
    }

    /// Returns the number of live handles sharing this frame storage.
    pub fn handle_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }
}

impl<T: ReconSample> core::ops::Deref for SharedFrame<T> {
    type Target = DecodedFrame<T>;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

fn validate_crop_alignment(pixel_format: PixelFormat, rect: PlaneRect) -> Result<()> {
    if pixel_format.is_monochrome() {
        return Ok(());
    }

    let sub_x = pixel_format.subsampling_x();
    let sub_y = pixel_format.subsampling_y();
    let x_aligned = is_aligned(rect.x(), sub_x);
    let y_aligned = is_aligned(rect.y(), sub_y);
    if x_aligned && y_aligned {
        Ok(())
    } else {
        Err(ReconError::CropOriginNotAligned {
            x: rect.x(),
            y: rect.y(),
            subsampling_x: sub_x,
            subsampling_y: sub_y,
        })
    }
}

fn is_aligned(value: usize, subsampling: u8) -> bool {
    if subsampling == 0 {
        true
    } else {
        value.is_multiple_of(1_usize << subsampling)
    }
}

fn validate_plane_size(plane: PlaneId, expected: PlaneSize, actual: PlaneSize) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(ReconError::PlaneSizeMismatch {
            plane,
            expected,
            actual,
        })
    }
}

fn validate_plane_samples<T: ReconSample>(
    plane: PlaneId,
    samples: &Plane<T>,
    max: u16,
) -> Result<()> {
    let values = samples.samples();
    if !crate::workspace::samples_exceed(values, max) {
        return Ok(());
    }
    let (sample_index, value) = values
        .iter()
        .enumerate()
        .find_map(|(sample_index, sample)| {
            let value = sample.to_u16();
            (value > max).then_some((sample_index, value))
        })
        .unwrap_or((0, max));
    Err(ReconError::SampleOutOfRange {
        plane,
        sample_index,
        value,
        max,
    })
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

    fn plane_u8(width: usize, height: usize, value: u8) -> Plane<u8> {
        let size = size(width, height);
        let rect = rect(0, 0, width, height);
        Plane::from_vec(size, width, rect, vec![value; width * height]).unwrap()
    }

    fn plane_u16(width: usize, height: usize, value: u16) -> Plane<u16> {
        let size = size(width, height);
        let rect = rect(0, 0, width, height);
        Plane::from_vec(size, width, rect, vec![value; width * height]).unwrap()
    }

    fn info(
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        coded_luma_size: PlaneSize,
        visible_luma_rect: PlaneRect,
    ) -> DecodedFrameInfo {
        DecodedFrameInfo::new(
            OutputIndex::new(0),
            bit_depth,
            pixel_format,
            coded_luma_size,
            visible_luma_rect,
        )
        .unwrap()
    }

    #[test]
    fn accepts_monochrome_frame_without_chroma_planes() {
        let y = plane_u8(4, 2, 7);
        let frame = DecodedFrame::try_new(
            info(
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 2),
                rect(0, 0, 4, 2),
            ),
            FramePlanes::new(y, None, None),
        )
        .unwrap();

        assert_eq!(frame.output_index().get(), 0);
        assert_eq!(frame.pixel_format(), PixelFormat::Monochrome);
        assert!(frame.u().is_none());
        assert!(frame.v().is_none());
    }

    #[test]
    fn rejects_chroma_planes_for_monochrome_frame() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Eight,
                    PixelFormat::Monochrome,
                    size(4, 2),
                    rect(0, 0, 4, 2),
                ),
                FramePlanes::new(plane_u8(4, 2, 0), Some(plane_u8(2, 1, 0)), None),
            ),
            Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U })
        ));
    }

    #[test]
    fn accepts_yuv420_frame_with_derived_chroma_size() {
        let frame = DecodedFrame::try_new(
            DecodedFrameInfo::new(
                OutputIndex::new(3),
                BitDepth::Eight,
                PixelFormat::Yuv420,
                size(5, 3),
                rect(0, 0, 5, 3),
            )
            .unwrap(),
            FramePlanes::new(
                plane_u8(5, 3, 10),
                Some(plane_u8(3, 2, 20)),
                Some(plane_u8(3, 2, 30)),
            ),
        )
        .unwrap();

        assert_eq!(frame.output_index().get(), 3);
        assert_eq!(frame.u().map(Plane::visible_size), Some(size(3, 2)));
        assert_eq!(frame.v().map(Plane::visible_size), Some(size(3, 2)));
    }

    #[test]
    fn rejects_missing_non_monochrome_chroma_plane() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Eight,
                    PixelFormat::Yuv444,
                    size(2, 2),
                    rect(0, 0, 2, 2),
                ),
                FramePlanes::new(plane_u8(2, 2, 0), None, Some(plane_u8(2, 2, 0))),
            ),
            Err(ReconError::MissingChromaPlane { plane: PlaneId::U })
        ));
    }

    #[test]
    fn rejects_wrong_luma_plane_visible_size() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Eight,
                    PixelFormat::Monochrome,
                    size(4, 4),
                    rect(0, 0, 4, 4),
                ),
                FramePlanes::new(plane_u8(4, 3, 0), None, None),
            ),
            Err(ReconError::PlaneSizeMismatch {
                plane: PlaneId::Y,
                expected,
                actual
            }) if expected == size(4, 4) && actual == size(4, 3)
        ));
    }

    #[test]
    fn rejects_wrong_chroma_visible_size() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Eight,
                    PixelFormat::Yuv422,
                    size(5, 3),
                    rect(0, 0, 5, 3),
                ),
                FramePlanes::new(
                    plane_u8(5, 3, 0),
                    Some(plane_u8(2, 3, 0)),
                    Some(plane_u8(3, 3, 0)),
                ),
            ),
            Err(ReconError::PlaneSizeMismatch {
                plane: PlaneId::U,
                expected,
                actual
            }) if expected == size(3, 3) && actual == size(2, 3)
        ));
    }

    #[test]
    fn rejects_luma_crop_outside_coded_size() {
        assert!(matches!(
            DecodedFrameInfo::new(
                OutputIndex::new(0),
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 4),
                rect(1, 1, 4, 4),
            ),
            Err(ReconError::VisibleRectOutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_unaligned_non_monochrome_crop_origin() {
        assert!(matches!(
            DecodedFrameInfo::new(
                OutputIndex::new(0),
                BitDepth::Eight,
                PixelFormat::Yuv420,
                size(6, 4),
                rect(1, 0, 4, 4),
            ),
            Err(ReconError::CropOriginNotAligned {
                x: 1,
                y: 0,
                subsampling_x: 1,
                subsampling_y: 1
            })
        ));
    }

    #[test]
    fn rejects_u8_storage_for_ten_bit_output() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Ten,
                    PixelFormat::Monochrome,
                    size(2, 2),
                    rect(0, 0, 2, 2),
                ),
                FramePlanes::new(plane_u8(2, 2, 0), None, None),
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn rejects_u16_sample_above_active_bit_depth() {
        assert!(matches!(
            DecodedFrame::try_new(
                info(
                    BitDepth::Ten,
                    PixelFormat::Monochrome,
                    size(2, 2),
                    rect(0, 0, 2, 2),
                ),
                FramePlanes::new(plane_u16(2, 2, 1024), None, None),
            ),
            Err(ReconError::SampleOutOfRange {
                plane: PlaneId::Y,
                sample_index: 0,
                value: 1024,
                max: 1023
            })
        ));
    }

    #[test]
    fn shared_frame_share_yields_two_handles_to_one_storage() {
        let frame = DecodedFrame::try_new(
            info(
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 2),
                rect(0, 0, 4, 2),
            ),
            FramePlanes::new(plane_u8(4, 2, 7), None, None),
        )
        .unwrap();
        let shared = SharedFrame::new(frame);
        assert_eq!(shared.handle_count(), 1);

        let second = shared.share();
        assert_eq!(shared.handle_count(), 2);
        assert_eq!(
            shared.get().y().samples().as_ptr(),
            second.get().y().samples().as_ptr()
        );

        drop(second);
        assert_eq!(shared.handle_count(), 1);
    }

    /// Dropping one handle must leave the storage untouched, so a surviving
    /// handle still reads its original samples.
    #[test]
    fn dropping_one_handle_leaves_still_shared_storage_intact() {
        let frame = DecodedFrame::try_new(
            info(
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 2),
                rect(0, 0, 4, 2),
            ),
            FramePlanes::new(plane_u8(4, 2, 7), None, None),
        )
        .unwrap();
        let shared = SharedFrame::new(frame);
        let survivor = shared.share();
        assert_eq!(survivor.handle_count(), 2);

        drop(shared);
        assert_eq!(survivor.handle_count(), 1);
        assert!(survivor.get().y().samples().iter().all(|&s| s == 7));
    }

    /// The sole-owner path runs the recycling drop without panic.
    #[test]
    fn dropping_the_sole_handle_recycles_the_planes() {
        let frame = DecodedFrame::try_new(
            info(
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 2),
                rect(0, 0, 4, 2),
            ),
            FramePlanes::new(plane_u8(4, 2, 3), None, None),
        )
        .unwrap();
        let shared = SharedFrame::new(frame);
        assert_eq!(shared.handle_count(), 1);

        drop(shared);
    }
}
