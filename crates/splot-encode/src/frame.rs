// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Borrowed encoder input frame views.
//!
//! This module advances `ENC-Y4M-INPUT` with the first real encoder input
//! surface: validated borrowed 8-bit YUV420 planes. It does not parse Y4M, retain
//! lookahead input, or expose a successful encode path.

use splot_recon::{
    BitDepth as ReconBitDepth, FrameRef as ReconFrameRef, PixelFormat, PlaneId, PlaneRef,
    PlaneSize, SharedFrame,
};

use crate::config::{BitDepth, ChromaSubsampling};
use crate::error::{Error, Result};

/// Repository-owned input frame identifier.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameId(u64);

impl FrameId {
    /// Creates an input frame identifier.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the zero-based frame identifier value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Input timestamp ticks in the caller's stream timebase.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameTimestamp(i64);

impl FrameTimestamp {
    /// Creates timestamp ticks for an input frame.
    pub const fn new(ticks: i64) -> Self {
        Self(ticks)
    }

    /// Returns timestamp ticks in the caller's stream timebase.
    pub const fn ticks(self) -> i64 {
        self.0
    }
}

/// Typed metadata shared by all input planes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameInfo {
    id: FrameId,
    visible_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
    timestamp: Option<FrameTimestamp>,
}

impl FrameInfo {
    /// Creates input frame metadata.
    pub const fn new(
        id: FrameId,
        visible_luma_size: PlaneSize,
        bit_depth: BitDepth,
        chroma_subsampling: ChromaSubsampling,
    ) -> Self {
        Self {
            id,
            visible_luma_size,
            bit_depth,
            chroma_subsampling,
            timestamp: None,
        }
    }

    /// Creates metadata for the current 8-bit YUV420 input subset.
    pub const fn yuv420_8bit(id: FrameId, visible_luma_size: PlaneSize) -> Self {
        Self::new(
            id,
            visible_luma_size,
            BitDepth::Eight,
            ChromaSubsampling::Yuv420,
        )
    }

    /// Returns metadata with timestamp ticks attached.
    #[must_use]
    pub const fn with_timestamp(mut self, timestamp: FrameTimestamp) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Returns the input frame identifier.
    pub const fn id(self) -> FrameId {
        self.id
    }

    /// Returns the visible luma size in samples.
    pub const fn visible_luma_size(self) -> PlaneSize {
        self.visible_luma_size
    }

    /// Returns the input sample bit depth.
    pub const fn bit_depth(self) -> BitDepth {
        self.bit_depth
    }

    /// Returns the input chroma layout.
    pub const fn chroma_subsampling(self) -> ChromaSubsampling {
        self.chroma_subsampling
    }

    /// Returns timestamp ticks when provided by the caller.
    pub const fn timestamp(self) -> Option<FrameTimestamp> {
        self.timestamp
    }
}

/// Borrowed candidate plane input before frame-level validation.
#[derive(Clone, Copy, Debug)]
pub struct FramePlaneInput<'a> {
    samples: &'a [u8],
    stride_samples: usize,
    visible_rect: splot_recon::PlaneRect,
}

impl<'a> FramePlaneInput<'a> {
    /// Creates a borrowed plane input descriptor.
    ///
    /// The descriptor is validated when passed to [`Frame::from_planes`].
    pub const fn new(
        samples: &'a [u8],
        stride_samples: usize,
        visible_rect: splot_recon::PlaneRect,
    ) -> Self {
        Self {
            samples,
            stride_samples,
            visible_rect,
        }
    }

    /// Creates a descriptor from an already-validated reconstruction plane view.
    pub const fn from_plane_ref(plane: PlaneRef<'a, u8>) -> Self {
        Self {
            samples: plane.samples(),
            stride_samples: plane.stride_samples(),
            visible_rect: plane.visible_rect(),
        }
    }

    /// Returns the borrowed backing samples.
    pub const fn samples(&self) -> &'a [u8] {
        self.samples
    }

    /// Returns stride in samples.
    pub const fn stride_samples(&self) -> usize {
        self.stride_samples
    }

    /// Returns the visible plane rectangle.
    pub const fn visible_rect(&self) -> splot_recon::PlaneRect {
        self.visible_rect
    }
}

/// Borrowed candidate plane set before frame-level validation.
#[derive(Clone, Copy, Debug)]
pub struct FramePlanesInput<'a> {
    y: FramePlaneInput<'a>,
    u: Option<FramePlaneInput<'a>>,
    v: Option<FramePlaneInput<'a>>,
}

impl<'a> FramePlanesInput<'a> {
    /// Creates a candidate YUV plane set.
    pub const fn yuv(
        y: FramePlaneInput<'a>,
        u: FramePlaneInput<'a>,
        v: FramePlaneInput<'a>,
    ) -> Self {
        Self {
            y,
            u: Some(u),
            v: Some(v),
        }
    }

    /// Creates a candidate luma-only plane set.
    pub const fn luma_only(y: FramePlaneInput<'a>) -> Self {
        Self {
            y,
            u: None,
            v: None,
        }
    }

    /// Creates a candidate plane set from optional chroma descriptors.
    pub const fn new(
        y: FramePlaneInput<'a>,
        u: Option<FramePlaneInput<'a>>,
        v: Option<FramePlaneInput<'a>>,
    ) -> Self {
        Self { y, u, v }
    }

    /// Returns the candidate luma plane descriptor.
    pub const fn y(&self) -> &FramePlaneInput<'a> {
        &self.y
    }

    /// Returns the candidate U plane descriptor when present.
    pub const fn u(&self) -> Option<&FramePlaneInput<'a>> {
        self.u.as_ref()
    }

    /// Returns the candidate V plane descriptor when present.
    pub const fn v(&self) -> Option<&FramePlaneInput<'a>> {
        self.v.as_ref()
    }
}

/// Borrowed validated encoder input frame.
#[derive(Debug)]
pub struct Frame<'a> {
    info: FrameInfo,
    y: PlaneRef<'a, u8>,
    u: PlaneRef<'a, u8>,
    v: PlaneRef<'a, u8>,
}

impl<'a> Frame<'a> {
    /// Builds a borrowed input frame after validating format, plane count,
    /// visible geometry, stride, and buffer lengths.
    ///
    /// # Errors
    /// Returns [`crate::Error`] when the input is outside the current 8-bit
    /// YUV420 subset or any borrowed plane cannot represent the requested
    /// visible frame.
    pub fn from_planes(info: FrameInfo, planes: FramePlanesInput<'a>) -> Result<Self> {
        ensure_supported_info(info)?;

        let FramePlanesInput { y, u, v } = planes;
        let y = plane_ref(PlaneId::Y, y)?;
        validate_plane_size(PlaneId::Y, info.visible_luma_size, y.visible_size())?;

        let Some(u) = u else {
            return Err(Error::MissingInputPlane { plane: PlaneId::U });
        };
        let Some(v) = v else {
            return Err(Error::MissingInputPlane { plane: PlaneId::V });
        };
        let u = plane_ref(PlaneId::U, u)?;
        let v = plane_ref(PlaneId::V, v)?;

        let Some(chroma_visible_size) = PixelFormat::Yuv420
            .chroma_size(info.visible_luma_size)
            .map_err(|source| Error::InputChromaGeometry { source })?
        else {
            // Future non-subsampled input formats must not accept chroma planes.
            return Err(Error::UnexpectedInputPlane { plane: PlaneId::U });
        };
        validate_plane_size(PlaneId::U, chroma_visible_size, u.visible_size())?;
        validate_plane_size(PlaneId::V, chroma_visible_size, v.visible_size())?;

        Ok(Self { info, y, u, v })
    }

    /// Builds an input frame from a reconstruction frame view after validating
    /// that the view matches the current 8-bit YUV420 input subset.
    ///
    /// # Errors
    /// Returns [`crate::Error`] if the reconstruction view has unsupported
    /// format metadata or lacks required chroma planes.
    pub fn from_recon_frame_ref(
        id: FrameId,
        timestamp: Option<FrameTimestamp>,
        frame: ReconFrameRef<'a, u8>,
    ) -> Result<Self> {
        let info = info_from_recon(id, timestamp, frame);
        let y = FramePlaneInput::from_plane_ref(frame.y());
        let u = frame.u().map(FramePlaneInput::from_plane_ref);
        let v = frame.v().map(FramePlaneInput::from_plane_ref);
        Self::from_planes(info, FramePlanesInput::new(y, u, v))
    }

    /// Returns the validated frame metadata.
    pub const fn info(&self) -> FrameInfo {
        self.info
    }

    /// Returns the input frame identifier.
    pub const fn id(&self) -> FrameId {
        self.info.id()
    }

    /// Returns timestamp ticks when provided by the caller.
    pub const fn timestamp(&self) -> Option<FrameTimestamp> {
        self.info.timestamp()
    }

    /// Returns the visible luma size in samples.
    pub const fn visible_luma_size(&self) -> PlaneSize {
        self.info.visible_luma_size()
    }

    /// Returns the input sample bit depth.
    pub const fn bit_depth(&self) -> BitDepth {
        self.info.bit_depth()
    }

    /// Returns the input chroma layout.
    pub const fn chroma_subsampling(&self) -> ChromaSubsampling {
        self.info.chroma_subsampling()
    }

    /// Returns the borrowed luma plane.
    pub const fn y(&self) -> PlaneRef<'a, u8> {
        self.y
    }

    /// Returns the borrowed U plane.
    pub const fn u(&self) -> PlaneRef<'a, u8> {
        self.u
    }

    /// Returns the borrowed V plane.
    pub const fn v(&self) -> PlaneRef<'a, u8> {
        self.v
    }

    /// Returns a borrowed plane by identifier.
    pub const fn plane(&self, plane: PlaneId) -> PlaneRef<'a, u8> {
        match plane {
            PlaneId::Y => self.y,
            PlaneId::U => self.u,
            PlaneId::V => self.v,
        }
    }
}

/// Explicit retained input frame handle for future lookahead.
#[derive(Debug)]
pub struct RetainedFrame {
    info: FrameInfo,
    frame: SharedFrame<u8>,
}

impl RetainedFrame {
    /// Wraps a shared frame as retained encoder input after validating it
    /// against the current 8-bit YUV420 subset.
    ///
    /// # Errors
    /// Returns [`crate::Error`] if `frame` does not match `info` or the current
    /// input subset.
    pub fn from_shared_frame(info: FrameInfo, frame: SharedFrame<u8>) -> Result<Self> {
        let frame_ref = frame.as_frame_ref();
        let shared_info = info_from_recon(info.id(), info.timestamp(), frame_ref);
        ensure_supported_info(shared_info)?;
        validate_plane_size(
            PlaneId::Y,
            info.visible_luma_size,
            shared_info.visible_luma_size,
        )?;

        Frame::from_planes(
            info,
            FramePlanesInput::new(
                FramePlaneInput::from_plane_ref(frame_ref.y()),
                frame_ref.u().map(FramePlaneInput::from_plane_ref),
                frame_ref.v().map(FramePlaneInput::from_plane_ref),
            ),
        )?;
        Ok(Self { info, frame })
    }

    /// Returns a second retained handle to the same frame storage without
    /// copying pixels.
    #[must_use]
    pub fn share(&self) -> Self {
        Self {
            info: self.info,
            frame: self.frame.share(),
        }
    }

    /// Borrows the retained input as a validated encoder frame.
    ///
    /// # Errors
    /// Returns [`crate::Error`] only if the retained-frame invariant has been
    /// broken by future code changes.
    pub fn as_frame(&self) -> Result<Frame<'_>> {
        let frame_ref = self.frame.as_frame_ref();
        Frame::from_planes(
            self.info,
            FramePlanesInput::new(
                FramePlaneInput::from_plane_ref(frame_ref.y()),
                frame_ref.u().map(FramePlaneInput::from_plane_ref),
                frame_ref.v().map(FramePlaneInput::from_plane_ref),
            ),
        )
    }

    /// Returns the retained frame metadata.
    pub const fn info(&self) -> FrameInfo {
        self.info
    }

    /// Returns the number of live shared handles to this frame storage.
    pub fn handle_count(&self) -> usize {
        self.frame.handle_count()
    }
}

fn ensure_supported_info(info: FrameInfo) -> Result<()> {
    if info.bit_depth != BitDepth::Eight {
        return Err(Error::UnsupportedInputBitDepth {
            bit_depth: info.bit_depth,
        });
    }
    if info.chroma_subsampling != ChromaSubsampling::Yuv420 {
        return Err(Error::UnsupportedInputChromaSubsampling {
            chroma_subsampling: info.chroma_subsampling,
        });
    }
    Ok(())
}

fn plane_ref(plane: PlaneId, input: FramePlaneInput<'_>) -> Result<PlaneRef<'_, u8>> {
    PlaneRef::new(input.samples, input.stride_samples, input.visible_rect)
        .map_err(|source| Error::InputPlane { plane, source })
}

fn validate_plane_size(plane: PlaneId, expected: PlaneSize, actual: PlaneSize) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(Error::InputPlaneSizeMismatch {
            plane,
            expected,
            actual,
        })
    }
}

fn info_from_recon(
    id: FrameId,
    timestamp: Option<FrameTimestamp>,
    frame: ReconFrameRef<'_, u8>,
) -> FrameInfo {
    let info = frame.info();
    let bit_depth = match info.bit_depth() {
        ReconBitDepth::Eight => BitDepth::Eight,
        ReconBitDepth::Ten => BitDepth::Ten,
    };
    let chroma_subsampling = match info.pixel_format() {
        PixelFormat::Monochrome => ChromaSubsampling::Monochrome,
        PixelFormat::Yuv420 => ChromaSubsampling::Yuv420,
        PixelFormat::Yuv422 => ChromaSubsampling::Yuv422,
        PixelFormat::Yuv444 => ChromaSubsampling::Yuv444,
    };
    let mut frame_info = FrameInfo::new(
        id,
        info.visible_luma_rect().size(),
        bit_depth,
        chroma_subsampling,
    );
    if let Some(timestamp) = timestamp {
        frame_info = frame_info.with_timestamp(timestamp);
    }
    frame_info
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use splot_recon::{
        BitDepth as ReconBitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, Plane,
        PlaneRect,
    };

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn plane_input(
        samples: &[u8],
        stride_samples: usize,
        width: usize,
        height: usize,
    ) -> FramePlaneInput<'_> {
        FramePlaneInput::new(samples, stride_samples, rect(0, 0, width, height))
    }

    fn valid_odd_frame_data() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            (0..20).collect(),
            vec![21, 22, 23, 24, 25, 26, 27, 28, 29],
            vec![31, 32, 33, 34, 35, 36, 37, 38, 39],
        )
    }

    fn valid_odd_frame<'a>(y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> Frame<'a> {
        let info = FrameInfo::yuv420_8bit(FrameId::new(7), size(3, 5))
            .with_timestamp(FrameTimestamp::new(42));
        Frame::from_planes(
            info,
            FramePlanesInput::yuv(
                plane_input(y, 4, 3, 5),
                plane_input(u, 3, 2, 3),
                plane_input(v, 3, 2, 3),
            ),
        )
        .unwrap()
    }

    #[test]
    fn borrowed_yuv420_frame_accepts_odd_visible_size_without_copying() {
        let (y, u, v) = valid_odd_frame_data();
        let frame = valid_odd_frame(&y, &u, &v);

        assert_eq!(frame.id(), FrameId::new(7));
        assert_eq!(frame.timestamp(), Some(FrameTimestamp::new(42)));
        assert_eq!(frame.visible_luma_size(), size(3, 5));
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.chroma_subsampling(), ChromaSubsampling::Yuv420);
        assert_eq!(frame.y().samples().as_ptr(), y.as_ptr());
        assert_eq!(frame.u().samples().as_ptr(), u.as_ptr());
        assert_eq!(frame.v().samples().as_ptr(), v.as_ptr());
        assert_eq!(frame.u().visible_size(), size(2, 3));
        assert_eq!(frame.v().visible_size(), size(2, 3));

        let y_rows: Vec<&[u8]> = frame.y().visible_rows().collect();
        assert_eq!(
            y_rows,
            vec![
                &[0, 1, 2][..],
                &[4, 5, 6][..],
                &[8, 9, 10][..],
                &[12, 13, 14][..],
                &[16, 17, 18][..],
            ]
        );
    }

    #[test]
    fn borrowed_frame_rejects_truncated_plane_with_plane_identity() {
        let (y, u, mut v) = valid_odd_frame_data();
        v.truncate(7);

        let err = Frame::from_planes(
            FrameInfo::yuv420_8bit(FrameId::new(0), size(3, 5)),
            FramePlanesInput::yuv(
                plane_input(&y, 4, 3, 5),
                plane_input(&u, 3, 2, 3),
                plane_input(&v, 3, 2, 3),
            ),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::InputPlane {
                plane: PlaneId::V,
                ..
            }
        ));
    }

    #[test]
    fn borrowed_frame_rejects_too_small_stride() {
        let (y, u, v) = valid_odd_frame_data();

        let err = Frame::from_planes(
            FrameInfo::yuv420_8bit(FrameId::new(0), size(3, 5)),
            FramePlanesInput::yuv(
                plane_input(&y, 2, 3, 5),
                plane_input(&u, 3, 2, 3),
                plane_input(&v, 3, 2, 3),
            ),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::InputPlane {
                plane: PlaneId::Y,
                ..
            }
        ));
    }

    #[test]
    fn borrowed_frame_rejects_missing_chroma_planes() {
        let (y, _u, _v) = valid_odd_frame_data();

        let err = Frame::from_planes(
            FrameInfo::yuv420_8bit(FrameId::new(0), size(3, 5)),
            FramePlanesInput::luma_only(plane_input(&y, 4, 3, 5)),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::MissingInputPlane { plane: PlaneId::U }
        ));
    }

    #[test]
    fn borrowed_frame_rejects_unsupported_formats() {
        let (y, u, v) = valid_odd_frame_data();

        let ten_bit = Frame::from_planes(
            FrameInfo::new(
                FrameId::new(0),
                size(3, 5),
                BitDepth::Ten,
                ChromaSubsampling::Yuv420,
            ),
            FramePlanesInput::yuv(
                plane_input(&y, 4, 3, 5),
                plane_input(&u, 3, 2, 3),
                plane_input(&v, 3, 2, 3),
            ),
        )
        .unwrap_err();
        assert!(matches!(
            ten_bit,
            Error::UnsupportedInputBitDepth {
                bit_depth: BitDepth::Ten
            }
        ));

        let yuv444 = Frame::from_planes(
            FrameInfo::new(
                FrameId::new(0),
                size(3, 5),
                BitDepth::Eight,
                ChromaSubsampling::Yuv444,
            ),
            FramePlanesInput::yuv(
                plane_input(&y, 4, 3, 5),
                plane_input(&u, 3, 2, 3),
                plane_input(&v, 3, 2, 3),
            ),
        )
        .unwrap_err();
        assert!(matches!(
            yuv444,
            Error::UnsupportedInputChromaSubsampling {
                chroma_subsampling: ChromaSubsampling::Yuv444
            }
        ));
    }

    #[test]
    fn borrowed_frame_rejects_chroma_size_mismatch() {
        let (y, u, v) = valid_odd_frame_data();

        let err = Frame::from_planes(
            FrameInfo::yuv420_8bit(FrameId::new(0), size(3, 5)),
            FramePlanesInput::yuv(
                plane_input(&y, 4, 3, 5),
                plane_input(&u, 3, 1, 3),
                plane_input(&v, 3, 2, 3),
            ),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::InputPlaneSizeMismatch {
                plane: PlaneId::U,
                expected,
                actual,
            } if expected == size(2, 3) && actual == size(1, 3)
        ));
    }

    #[test]
    fn small_dimension_stride_and_truncation_cases_are_checked() {
        for width in 1..=8 {
            for height in 1..=8 {
                let luma = size(width, height);
                let chroma = PixelFormat::Yuv420.chroma_size(luma).unwrap().unwrap();
                let y_stride = width + 1;
                let uv_stride = chroma.width() + 1;
                let y_len = y_stride * height;
                let uv_len = uv_stride * chroma.height();
                let y = vec![0_u8; y_len];
                let u = vec![0_u8; uv_len];
                let v = vec![0_u8; uv_len];

                assert!(
                    Frame::from_planes(
                        FrameInfo::yuv420_8bit(FrameId::new(0), luma),
                        FramePlanesInput::yuv(
                            plane_input(&y, y_stride, width, height),
                            plane_input(&u, uv_stride, chroma.width(), chroma.height()),
                            plane_input(&v, uv_stride, chroma.width(), chroma.height()),
                        ),
                    )
                    .is_ok()
                );

                let required_uv_len = (chroma.height() - 1) * uv_stride + chroma.width();
                if required_uv_len > 0 {
                    let truncated_u = vec![0_u8; required_uv_len - 1];
                    assert!(
                        Frame::from_planes(
                            FrameInfo::yuv420_8bit(FrameId::new(0), luma),
                            FramePlanesInput::yuv(
                                plane_input(&y, y_stride, width, height),
                                plane_input(
                                    &truncated_u,
                                    uv_stride,
                                    chroma.width(),
                                    chroma.height(),
                                ),
                                plane_input(&v, uv_stride, chroma.width(), chroma.height()),
                            ),
                        )
                        .is_err()
                    );
                }
            }
        }
    }

    #[test]
    fn retained_frame_shares_storage_without_clone() {
        let decoded = decoded_yuv420_frame();
        let shared = SharedFrame::new(decoded);
        let retained = RetainedFrame::from_shared_frame(
            FrameInfo::yuv420_8bit(FrameId::new(3), size(3, 5)),
            shared,
        )
        .unwrap();

        assert_eq!(retained.handle_count(), 1);
        let retained_again = retained.share();
        assert_eq!(retained.handle_count(), 2);
        assert_eq!(retained_again.handle_count(), 2);

        let frame = retained.as_frame().unwrap();
        let frame_again = retained_again.as_frame().unwrap();
        assert_eq!(frame.id(), FrameId::new(3));
        assert_eq!(frame_again.id(), FrameId::new(3));
        assert_eq!(
            frame.y().samples().as_ptr(),
            frame_again.y().samples().as_ptr()
        );
    }

    #[test]
    fn retained_frame_rejects_unsupported_shared_format() {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            ReconBitDepth::Eight,
            PixelFormat::Yuv444,
            size(3, 5),
            rect(0, 0, 3, 5),
        )
        .unwrap();
        let y = Plane::from_vec(size(3, 5), 3, rect(0, 0, 3, 5), vec![0_u8; 15]).unwrap();
        let u = Plane::from_vec(size(3, 5), 3, rect(0, 0, 3, 5), vec![0_u8; 15]).unwrap();
        let v = Plane::from_vec(size(3, 5), 3, rect(0, 0, 3, 5), vec![0_u8; 15]).unwrap();
        let frame = DecodedFrame::try_new(info, FramePlanes::new(y, Some(u), Some(v))).unwrap();
        let shared = SharedFrame::new(frame);

        let err = RetainedFrame::from_shared_frame(
            FrameInfo::yuv420_8bit(FrameId::new(0), size(3, 5)),
            shared,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::UnsupportedInputChromaSubsampling {
                chroma_subsampling: ChromaSubsampling::Yuv444
            }
        ));
    }

    fn decoded_yuv420_frame() -> DecodedFrame<u8> {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            ReconBitDepth::Eight,
            PixelFormat::Yuv420,
            size(3, 5),
            rect(0, 0, 3, 5),
        )
        .unwrap();
        let y = Plane::from_vec(size(3, 5), 3, rect(0, 0, 3, 5), vec![0_u8; 15]).unwrap();
        let u = Plane::from_vec(size(2, 3), 2, rect(0, 0, 2, 3), vec![1_u8; 6]).unwrap();
        let v = Plane::from_vec(size(2, 3), 2, rect(0, 0, 2, 3), vec![2_u8; 6]).unwrap();
        DecodedFrame::try_new(info, FramePlanes::new(y, Some(u), Some(v))).unwrap()
    }
}
