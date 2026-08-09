// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source-backed Y4M output writing for decoded frames.
//!
//! This module serializes already materialized [`DecodedFrame`] values into a
//! repository-owned Y4M byte policy. It uses AV2-derived decoded output facts
//! from § 6.4.1 and § 7.21.2 for visible dimensions, bit depth, and chroma
//! geometry, but the Y4M container tokens are not AV2 syntax.

use std::{
    io::{self, Write},
    num::NonZeroU32,
};

use crate::{BitDepth, DecodedFrame, PixelFormat, Plane, PlaneSize, ReconSample};

/// Result alias used by Y4M writer APIs.
pub type Y4mResult<T> = core::result::Result<T, Y4mError>;

/// Errors reported by the source-backed Y4M writer.
#[derive(Debug)]
#[non_exhaustive]
pub enum Y4mError {
    /// The caller supplied an invalid Y4M frame rate.
    InvalidFrameRate {
        /// Frame-rate numerator.
        numerator: u32,
        /// Frame-rate denominator.
        denominator: u32,
    },
    /// The decoded frame format has no repository-owned Y4M tag mapping.
    UnsupportedFrameFormat {
        /// Decoded sample bit depth.
        bit_depth: BitDepth,
        /// Decoded output pixel format.
        pixel_format: PixelFormat,
    },
    /// A frame did not match the stream header format.
    StreamParameterMismatch {
        /// Format committed by the stream header.
        expected: Y4mFrameFormat,
        /// Format derived from the frame being appended.
        actual: Y4mFrameFormat,
    },
    /// Checked arithmetic overflowed while deriving output byte sizes.
    ArithmeticOverflow {
        /// Short description of the overflowed derivation.
        context: &'static str,
    },
    /// The caller-provided writer returned an I/O error.
    Io {
        /// Original I/O error.
        source: io::Error,
    },
}

impl core::fmt::Display for Y4mError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFrameRate {
                numerator,
                denominator,
            } => write!(
                f,
                "invalid Y4M frame rate {numerator}:{denominator}; numerator and denominator must be nonzero"
            ),
            Self::UnsupportedFrameFormat {
                bit_depth,
                pixel_format,
            } => write!(
                f,
                "unsupported Y4M frame format: {}-bit {pixel_format:?}",
                bit_depth.bits()
            ),
            Self::StreamParameterMismatch { expected, actual } => write!(
                f,
                "Y4M stream/frame mismatch: expected {}x{} {}-bit {:?}, got {}x{} {}-bit {:?}",
                expected.visible_luma_size().width(),
                expected.visible_luma_size().height(),
                expected.bit_depth().bits(),
                expected.pixel_format(),
                actual.visible_luma_size().width(),
                actual.visible_luma_size().height(),
                actual.bit_depth().bits(),
                actual.pixel_format()
            ),
            Self::ArithmeticOverflow { context } => {
                write!(f, "arithmetic overflow while deriving {context}")
            }
            Self::Io { source } => write!(f, "Y4M writer I/O error: {source}"),
        }
    }
}

impl std::error::Error for Y4mError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source } => Some(source),
            Self::InvalidFrameRate { .. }
            | Self::UnsupportedFrameFormat { .. }
            | Self::StreamParameterMismatch { .. }
            | Self::ArithmeticOverflow { .. } => None,
        }
    }
}

impl From<io::Error> for Y4mError {
    fn from(source: io::Error) -> Self {
        Self::Io { source }
    }
}

/// Valid nonzero Y4M frame rate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Y4mFrameRate {
    numerator: NonZeroU32,
    denominator: NonZeroU32,
}

impl Y4mFrameRate {
    /// Creates a Y4M frame rate.
    ///
    /// # Errors
    /// Returns [`Y4mError::InvalidFrameRate`] if either component is zero.
    pub fn new(numerator: u32, denominator: u32) -> Y4mResult<Self> {
        match (NonZeroU32::new(numerator), NonZeroU32::new(denominator)) {
            (Some(numerator), Some(denominator)) => Ok(Self {
                numerator,
                denominator,
            }),
            _ => Err(Y4mError::InvalidFrameRate {
                numerator,
                denominator,
            }),
        }
    }

    /// Returns the frame-rate numerator.
    pub const fn numerator(self) -> u32 {
        self.numerator.get()
    }

    /// Returns the frame-rate denominator.
    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }
}

/// Repository-owned Y4M chroma tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Y4mChromaTag {
    /// `Cmono`.
    Mono,
    /// `Cmono10`.
    Mono10,
    /// `C420`.
    Yuv420,
    /// `C420p10`.
    Yuv420P10,
    /// `C422`.
    Yuv422,
    /// `C422p10`.
    Yuv422P10,
    /// `C444`.
    Yuv444,
    /// `C444p10`.
    Yuv444P10,
}

impl Y4mChromaTag {
    /// Derives the pinned Y4M chroma tag for an AV2-derived output format.
    ///
    /// # Errors
    /// Returns [`Y4mError::UnsupportedFrameFormat`] for modeled formats that do
    /// not have a repository-owned Y4M tag mapping.
    pub fn from_format(bit_depth: BitDepth, pixel_format: PixelFormat) -> Y4mResult<Self> {
        match (bit_depth, pixel_format) {
            (BitDepth::Eight, PixelFormat::Monochrome) => Ok(Self::Mono),
            (BitDepth::Ten, PixelFormat::Monochrome) => Ok(Self::Mono10),
            (BitDepth::Eight, PixelFormat::Yuv420) => Ok(Self::Yuv420),
            (BitDepth::Ten, PixelFormat::Yuv420) => Ok(Self::Yuv420P10),
            (BitDepth::Eight, PixelFormat::Yuv422) => Ok(Self::Yuv422),
            (BitDepth::Ten, PixelFormat::Yuv422) => Ok(Self::Yuv422P10),
            (BitDepth::Eight, PixelFormat::Yuv444) => Ok(Self::Yuv444),
            (BitDepth::Ten, PixelFormat::Yuv444) => Ok(Self::Yuv444P10),
        }
    }

    /// Returns the full serialized Y4M `C...` token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mono => "Cmono",
            Self::Mono10 => "Cmono10",
            Self::Yuv420 => "C420",
            Self::Yuv420P10 => "C420p10",
            Self::Yuv422 => "C422",
            Self::Yuv422P10 => "C422p10",
            Self::Yuv444 => "C444",
            Self::Yuv444P10 => "C444p10",
        }
    }
}

/// Decoded-frame format committed by a Y4M stream header.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Y4mFrameFormat {
    visible_luma_size: PlaneSize,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    chroma_tag: Y4mChromaTag,
}

impl Y4mFrameFormat {
    /// Creates a Y4M frame format from visible luma size and decoded format.
    ///
    /// # Errors
    /// Returns [`Y4mError::UnsupportedFrameFormat`] if no pinned chroma tag
    /// exists for the supplied format.
    pub fn new(
        visible_luma_size: PlaneSize,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
    ) -> Y4mResult<Self> {
        let chroma_tag = Y4mChromaTag::from_format(bit_depth, pixel_format)?;

        Ok(Self {
            visible_luma_size,
            bit_depth,
            pixel_format,
            chroma_tag,
        })
    }

    /// Derives a Y4M frame format from a decoded frame's visible output format.
    ///
    /// # Errors
    /// Returns [`Y4mError::UnsupportedFrameFormat`] if no pinned chroma tag
    /// exists for the frame's bit depth and pixel format.
    pub fn from_frame<T: ReconSample>(frame: &DecodedFrame<T>) -> Y4mResult<Self> {
        Self::new(
            frame.visible_luma_rect().size(),
            frame.bit_depth(),
            frame.pixel_format(),
        )
    }

    /// Returns the visible luma size written to the Y4M stream header.
    pub const fn visible_luma_size(self) -> PlaneSize {
        self.visible_luma_size
    }

    /// Returns the decoded sample bit depth.
    pub const fn bit_depth(self) -> BitDepth {
        self.bit_depth
    }

    /// Returns the decoded output pixel format.
    pub const fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns the pinned Y4M chroma tag.
    pub const fn chroma_tag(self) -> Y4mChromaTag {
        self.chroma_tag
    }
}

/// Y4M stream header metadata.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Y4mStreamHeader {
    frame_format: Y4mFrameFormat,
    frame_rate: Y4mFrameRate,
}

impl Y4mStreamHeader {
    /// Creates stream header metadata from a validated frame format and rate.
    pub const fn new(frame_format: Y4mFrameFormat, frame_rate: Y4mFrameRate) -> Self {
        Self {
            frame_format,
            frame_rate,
        }
    }

    /// Derives stream header metadata from a decoded frame and frame rate.
    ///
    /// # Errors
    /// Returns [`Y4mError::UnsupportedFrameFormat`] if no pinned chroma tag
    /// exists for the frame's bit depth and pixel format.
    pub fn from_frame<T: ReconSample>(
        frame: &DecodedFrame<T>,
        frame_rate: Y4mFrameRate,
    ) -> Y4mResult<Self> {
        Ok(Self::new(Y4mFrameFormat::from_frame(frame)?, frame_rate))
    }

    /// Returns the frame format committed by this stream header.
    pub const fn frame_format(self) -> Y4mFrameFormat {
        self.frame_format
    }

    /// Returns the stream frame rate.
    pub const fn frame_rate(self) -> Y4mFrameRate {
        self.frame_rate
    }

    /// Writes the serialized Y4M stream header.
    ///
    /// The header uses visible luma dimensions, progressive `Ip` output, the
    /// caller-supplied frame rate, default `A0:0` sample aspect ratio, and the
    /// repository-owned chroma tag.
    ///
    /// # Errors
    /// Returns [`Y4mError::Io`] if the caller-provided writer fails.
    pub fn write_to<W: Write + ?Sized>(&self, writer: &mut W) -> Y4mResult<()> {
        let size = self.frame_format.visible_luma_size();
        writeln!(
            writer,
            "YUV4MPEG2 W{} H{} F{}:{} Ip A0:0 {}",
            size.width(),
            size.height(),
            self.frame_rate.numerator(),
            self.frame_rate.denominator(),
            self.frame_format.chroma_tag().as_str()
        )?;
        Ok(())
    }
}

/// Y4M per-frame header metadata.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Y4mFrameHeader;

impl Y4mFrameHeader {
    /// Creates the repository-owned empty Y4M frame header.
    pub const fn new() -> Self {
        Self
    }

    /// Returns the serialized Y4M frame header bytes.
    pub const fn as_bytes(self) -> &'static [u8] {
        b"FRAME\n"
    }

    /// Writes the serialized Y4M frame header.
    ///
    /// # Errors
    /// Returns [`Y4mError::Io`] if the caller-provided writer fails.
    pub fn write_to<W: Write + ?Sized>(&self, writer: &mut W) -> Y4mResult<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
    }
}

/// Source-backed Y4M writer for caller-supplied decoded frames.
#[derive(Debug)]
pub struct Y4mWriter<W: Write> {
    writer: W,
    stream_header: Y4mStreamHeader,
    frames_written: u64,
}

impl<W: Write> Y4mWriter<W> {
    /// Writes the stream header and creates a Y4M writer.
    ///
    /// # Errors
    /// Returns [`Y4mError::Io`] if the caller-provided writer fails while
    /// receiving the stream header.
    pub fn new(mut writer: W, stream_header: Y4mStreamHeader) -> Y4mResult<Self> {
        stream_header.write_to(&mut writer)?;
        Ok(Self {
            writer,
            stream_header,
            frames_written: 0,
        })
    }

    /// Derives the stream format from `frame`, writes the stream header, and
    /// creates a Y4M writer.
    ///
    /// The frame is not written by this constructor; call [`Self::write_frame`]
    /// to append it.
    ///
    /// # Errors
    /// Returns [`Y4mError::UnsupportedFrameFormat`] if no pinned chroma tag
    /// exists for the frame's format, or [`Y4mError::Io`] if the
    /// caller-provided writer fails while receiving the stream header.
    pub fn from_frame<T: ReconSample>(
        writer: W,
        frame: &DecodedFrame<T>,
        frame_rate: Y4mFrameRate,
    ) -> Y4mResult<Self> {
        Self::new(writer, Y4mStreamHeader::from_frame(frame, frame_rate)?)
    }

    /// Returns the number of accepted frames written after the stream header.
    pub const fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Flushes the wrapped writer.
    ///
    /// # Errors
    /// Returns [`Y4mError::Io`] if the caller-provided writer fails.
    pub fn flush(&mut self) -> Y4mResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    /// Appends one decoded frame to the stream.
    ///
    /// The frame's visible dimensions, bit depth, and pixel format must match
    /// the stream header. Mismatches are rejected before `FRAME\n` or payload
    /// bytes are written for the attempted frame.
    ///
    /// # Errors
    /// Returns [`Y4mError::StreamParameterMismatch`] if the frame format differs
    /// from the stream header, [`Y4mError::UnsupportedFrameFormat`] if no pinned
    /// chroma tag exists for the frame, or [`Y4mError::Io`] if the
    /// caller-provided writer fails.
    pub fn write_frame<T: ReconSample>(&mut self, frame: &DecodedFrame<T>) -> Y4mResult<()> {
        let actual = Y4mFrameFormat::from_frame(frame)?;
        let expected = self.stream_header.frame_format();
        if actual != expected {
            return Err(Y4mError::StreamParameterMismatch { expected, actual });
        }

        Y4mFrameHeader::new().write_to(&mut self.writer)?;
        write_frame_payload(frame, &mut self.writer)?;
        self.frames_written += 1;
        Ok(())
    }

    /// Consumes the Y4M writer and returns the wrapped writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

fn write_frame_payload<T: ReconSample, W: Write + ?Sized>(
    frame: &DecodedFrame<T>,
    writer: &mut W,
) -> Y4mResult<()> {
    let bit_depth = frame.bit_depth();
    write_visible_plane(bit_depth, frame.y(), writer)?;
    if let Some(u) = frame.u() {
        write_visible_plane(bit_depth, u, writer)?;
    }
    if let Some(v) = frame.v() {
        write_visible_plane(bit_depth, v, writer)?;
    }
    Ok(())
}

fn write_visible_plane<T: ReconSample, W: Write + ?Sized>(
    bit_depth: BitDepth,
    plane: &Plane<T>,
    writer: &mut W,
) -> Y4mResult<()> {
    let row_byte_len = plane
        .visible_size()
        .width()
        .checked_mul(bytes_per_sample(bit_depth))
        .ok_or(Y4mError::ArithmeticOverflow {
            context: "Y4M visible row byte length",
        })?;
    let mut row_bytes = Vec::new();
    row_bytes
        .try_reserve_exact(row_byte_len)
        .map_err(|err| Y4mError::Io {
            source: io::Error::other(format!("Y4M row buffer allocation failed: {err}")),
        })?;

    for row in plane.visible_rows() {
        row_bytes.clear();
        for sample in row {
            push_sample(bit_depth, sample.to_u16(), &mut row_bytes);
        }
        writer.write_all(&row_bytes)?;
    }

    Ok(())
}

const fn bytes_per_sample(bit_depth: BitDepth) -> usize {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn push_sample(bit_depth: BitDepth, sample: u16, row_bytes: &mut Vec<u8>) {
    match bit_depth {
        BitDepth::Eight => row_bytes.push(sample as u8),
        // splot-copy-ok: serialize a decoded sample into the Y4M output byte row
        BitDepth::Ten => row_bytes.extend_from_slice(&sample.to_le_bytes()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{DecodedFrameInfo, FramePlanes, OutputIndex, PlaneRect};

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn frame_rate() -> Y4mFrameRate {
        Y4mFrameRate::new(24_000, 1001).unwrap()
    }

    fn info(
        output_index: u64,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        coded_luma_size: PlaneSize,
        visible_luma_rect: PlaneRect,
    ) -> DecodedFrameInfo {
        DecodedFrameInfo::new(
            OutputIndex::new(output_index),
            bit_depth,
            pixel_format,
            coded_luma_size,
            visible_luma_rect,
        )
        .unwrap()
    }

    fn plane<T: ReconSample>(
        storage_size: PlaneSize,
        stride_samples: usize,
        visible_rect: PlaneRect,
        samples: Vec<T>,
    ) -> Plane<T> {
        Plane::from_vec(storage_size, stride_samples, visible_rect, samples).unwrap()
    }

    fn mono_frame<T: ReconSample>(
        output_index: u64,
        bit_depth: BitDepth,
        coded_luma_size: PlaneSize,
        visible_luma_rect: PlaneRect,
        y: Plane<T>,
    ) -> DecodedFrame<T> {
        DecodedFrame::try_new(
            info(
                output_index,
                bit_depth,
                PixelFormat::Monochrome,
                coded_luma_size,
                visible_luma_rect,
            ),
            FramePlanes::new(y, None, None),
        )
        .unwrap()
    }

    fn yuv_frame<T: ReconSample>(
        output_index: u64,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        coded_luma_size: PlaneSize,
        visible_luma_rect: PlaneRect,
        planes: FramePlanes<T>,
    ) -> DecodedFrame<T> {
        DecodedFrame::try_new(
            info(
                output_index,
                bit_depth,
                pixel_format,
                coded_luma_size,
                visible_luma_rect,
            ),
            planes,
        )
        .unwrap()
    }

    fn compact_mono_u8(width: usize, height: usize, sample: u8) -> DecodedFrame<u8> {
        let luma_size = size(width, height);
        let luma_rect = rect(0, 0, width, height);
        mono_frame(
            0,
            BitDepth::Eight,
            luma_size,
            luma_rect,
            plane(luma_size, width, luma_rect, vec![sample; width * height]),
        )
    }

    fn compact_yuv_u8(
        pixel_format: PixelFormat,
        width: usize,
        height: usize,
        y_sample: u8,
    ) -> DecodedFrame<u8> {
        let luma_size = size(width, height);
        let luma_rect = rect(0, 0, width, height);
        let chroma_size = pixel_format.chroma_size(luma_size).unwrap().unwrap();
        let chroma_rect = rect(0, 0, chroma_size.width(), chroma_size.height());
        let chroma_len = chroma_size.width() * chroma_size.height();

        yuv_frame(
            0,
            BitDepth::Eight,
            pixel_format,
            luma_size,
            luma_rect,
            FramePlanes::new(
                plane(luma_size, width, luma_rect, vec![y_sample; width * height]),
                Some(plane(
                    chroma_size,
                    chroma_size.width(),
                    chroma_rect,
                    vec![y_sample.wrapping_add(1); chroma_len],
                )),
                Some(plane(
                    chroma_size,
                    chroma_size.width(),
                    chroma_rect,
                    vec![y_sample.wrapping_add(2); chroma_len],
                )),
            ),
        )
    }

    fn compact_mono_u16(bit_depth: BitDepth, sample: u16) -> DecodedFrame<u16> {
        let luma_size = size(2, 1);
        let luma_rect = rect(0, 0, 2, 1);
        mono_frame(
            0,
            bit_depth,
            luma_size,
            luma_rect,
            plane(luma_size, 2, luma_rect, vec![sample, sample]),
        )
    }

    fn header_bytes(header: Y4mStreamHeader) -> Vec<u8> {
        let mut bytes = Vec::new();
        header.write_to(&mut bytes).unwrap();
        bytes
    }

    fn stream_bytes<T: ReconSample>(frame: &DecodedFrame<T>) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = Y4mWriter::from_frame(&mut bytes, frame, frame_rate()).unwrap();
            writer.write_frame(frame).unwrap();
        }
        bytes
    }

    #[test]
    fn stream_headers_are_exact_for_supported_formats() {
        let luma_size = size(2, 2);
        let cases = [
            (BitDepth::Eight, PixelFormat::Monochrome, "Cmono"),
            (BitDepth::Ten, PixelFormat::Monochrome, "Cmono10"),
            (BitDepth::Eight, PixelFormat::Yuv420, "C420"),
            (BitDepth::Ten, PixelFormat::Yuv420, "C420p10"),
            (BitDepth::Eight, PixelFormat::Yuv422, "C422"),
            (BitDepth::Ten, PixelFormat::Yuv422, "C422p10"),
            (BitDepth::Eight, PixelFormat::Yuv444, "C444"),
            (BitDepth::Ten, PixelFormat::Yuv444, "C444p10"),
        ];

        for (bit_depth, pixel_format, tag) in cases {
            let format = Y4mFrameFormat::new(luma_size, bit_depth, pixel_format).unwrap();
            let header = Y4mStreamHeader::new(format, frame_rate());
            let expected = format!("YUV4MPEG2 W2 H2 F24000:1001 Ip A0:0 {tag}\n");
            assert_eq!(header_bytes(header), expected.as_bytes());
        }
    }

    #[test]
    fn stream_header_uses_visible_luma_size_from_frame() {
        let storage = size(5, 4);
        let visible = rect(2, 2, 2, 2);
        let frame = mono_frame(
            0,
            BitDepth::Eight,
            storage,
            visible,
            plane(storage, 6, visible, (0_u8..24).collect()),
        );

        let header = Y4mStreamHeader::from_frame(&frame, frame_rate()).unwrap();
        assert_eq!(
            header_bytes(header),
            b"YUV4MPEG2 W2 H2 F24000:1001 Ip A0:0 Cmono\n"
        );
    }

    #[test]
    fn frame_payload_excludes_crop_stride_and_coded_padding() {
        let storage = size(4, 3);
        let visible = rect(1, 1, 2, 2);
        let frame = mono_frame(
            0,
            BitDepth::Eight,
            storage,
            visible,
            plane(storage, 5, visible, (0_u8..15).collect()),
        );

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W2 H2 F24000:1001 Ip A0:0 Cmono\nFRAME\n\x06\x07\x0b\x0c"
        );
    }

    #[test]
    fn non_monochrome_payload_uses_y_u_v_plane_order() {
        let luma_size = size(2, 2);
        let luma_rect = rect(0, 0, 2, 2);
        let chroma_size = size(1, 1);
        let chroma_rect = rect(0, 0, 1, 1);
        let frame = yuv_frame(
            0,
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
            FramePlanes::new(
                plane(luma_size, 2, luma_rect, vec![1_u8, 2, 3, 4]),
                Some(plane(chroma_size, 1, chroma_rect, vec![10_u8])),
                Some(plane(chroma_size, 1, chroma_rect, vec![20_u8])),
            ),
        );

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W2 H2 F24000:1001 Ip A0:0 C420\nFRAME\n\x01\x02\x03\x04\x0a\x14"
        );
    }

    #[test]
    fn monochrome_payload_writes_y_only() {
        let frame = compact_mono_u8(2, 1, 7);

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W2 H1 F24000:1001 Ip A0:0 Cmono\nFRAME\n\x07\x07"
        );
    }

    #[test]
    fn odd_size_yuv420_uses_ceil_chroma_dimensions() {
        let luma_size = size(3, 3);
        let luma_rect = rect(0, 0, 3, 3);
        let chroma_size = size(2, 2);
        let chroma_rect = rect(0, 0, 2, 2);
        let frame = yuv_frame(
            0,
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
            FramePlanes::new(
                plane(luma_size, 3, luma_rect, vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 9]),
                Some(plane(chroma_size, 2, chroma_rect, vec![10_u8, 11, 12, 13])),
                Some(plane(chroma_size, 2, chroma_rect, vec![20_u8, 21, 22, 23])),
            ),
        );

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W3 H3 F24000:1001 Ip A0:0 C420\nFRAME\n\
              \x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x14\x15\x16\x17"
        );
    }

    #[test]
    fn eight_bit_u16_storage_writes_one_byte_per_sample() {
        let frame = compact_mono_u16(BitDepth::Eight, 255);

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W2 H1 F24000:1001 Ip A0:0 Cmono\nFRAME\n\xff\xff"
        );
    }

    #[test]
    fn ten_bit_samples_write_little_endian_pairs() {
        let luma_size = size(3, 1);
        let luma_rect = rect(0, 0, 3, 1);
        let frame = mono_frame(
            0,
            BitDepth::Ten,
            luma_size,
            luma_rect,
            plane(luma_size, 3, luma_rect, vec![1_u16, 0x0102, 1023]),
        );

        assert_eq!(
            stream_bytes(&frame),
            b"YUV4MPEG2 W3 H1 F24000:1001 Ip A0:0 Cmono10\nFRAME\n\x01\x00\x02\x01\xff\x03"
        );
    }

    #[test]
    fn multi_frame_stream_writes_header_once_and_one_frame_header_per_frame() {
        let first = compact_mono_u8(2, 1, 3);
        let second = compact_mono_u8(2, 1, 4);
        let mut bytes = Vec::new();

        {
            let mut writer = Y4mWriter::from_frame(&mut bytes, &first, frame_rate()).unwrap();
            writer.write_frame(&first).unwrap();
            writer.write_frame(&second).unwrap();
            assert_eq!(writer.frames_written(), 2);
        }

        assert_eq!(
            bytes,
            b"YUV4MPEG2 W2 H1 F24000:1001 Ip A0:0 Cmono\nFRAME\n\x03\x03FRAME\n\x04\x04"
        );
    }

    #[test]
    fn invalid_frame_rate_is_rejected() {
        for (numerator, denominator) in [(0, 1), (1, 0)] {
            let err = Y4mFrameRate::new(numerator, denominator).unwrap_err();

            assert!(matches!(
                err,
                Y4mError::InvalidFrameRate {
                    numerator: actual_numerator,
                    denominator: actual_denominator
                } if actual_numerator == numerator && actual_denominator == denominator
            ));
        }
    }

    #[test]
    fn mismatched_frame_is_rejected_before_frame_header_and_payload() {
        let first = compact_mono_u8(2, 1, 3);
        let mismatch = compact_yuv_u8(PixelFormat::Yuv420, 2, 2, 5);
        let mut bytes = Vec::new();
        {
            let mut writer = Y4mWriter::from_frame(&mut bytes, &first, frame_rate()).unwrap();
            writer.write_frame(&first).unwrap();

            let err = writer.write_frame(&mismatch).unwrap_err();
            assert!(matches!(
                err,
                Y4mError::StreamParameterMismatch { expected, actual }
                    if expected == Y4mFrameFormat::from_frame(&first).unwrap()
                        && actual == Y4mFrameFormat::from_frame(&mismatch).unwrap()
            ));
        }

        assert_eq!(
            bytes,
            b"YUV4MPEG2 W2 H1 F24000:1001 Ip A0:0 Cmono\nFRAME\n\x03\x03"
        );
    }

    #[test]
    fn io_error_while_writing_stream_header_is_propagated() {
        #[derive(Debug)]
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let frame = compact_mono_u8(1, 1, 5);
        let err = Y4mWriter::from_frame(FailingWriter, &frame, frame_rate()).unwrap_err();

        assert!(
            matches!(err, Y4mError::Io { source } if source.kind() == io::ErrorKind::BrokenPipe)
        );
    }

    #[test]
    fn io_error_while_writing_frame_header_is_propagated_without_payload() {
        #[derive(Debug)]
        struct FailOnFrameHeader {
            bytes: Vec<u8>,
        }

        impl Write for FailOnFrameHeader {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if buf == Y4mFrameHeader::new().as_bytes() {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
                }
                // splot-copy-ok: test fixture construction only (accumulates written bytes)
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let frame = compact_mono_u8(1, 1, 5);
        let writer = FailOnFrameHeader { bytes: Vec::new() };
        let mut writer = Y4mWriter::from_frame(writer, &frame, frame_rate()).unwrap();

        let err = writer.write_frame(&frame).unwrap_err();
        assert!(
            matches!(err, Y4mError::Io { source } if source.kind() == io::ErrorKind::BrokenPipe)
        );

        let writer = writer.into_inner();
        assert_eq!(writer.bytes, b"YUV4MPEG2 W1 H1 F24000:1001 Ip A0:0 Cmono\n");
    }

    #[test]
    fn io_error_while_writing_payload_is_propagated() {
        #[derive(Debug)]
        struct FailOnPayload {
            frame_header_seen: bool,
        }

        impl Write for FailOnPayload {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if self.frame_header_seen {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
                }
                if buf == Y4mFrameHeader::new().as_bytes() {
                    self.frame_header_seen = true;
                }
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let frame = compact_mono_u8(1, 1, 5);
        let writer = FailOnPayload {
            frame_header_seen: false,
        };
        let mut writer = Y4mWriter::from_frame(writer, &frame, frame_rate()).unwrap();

        let err = writer.write_frame(&frame).unwrap_err();
        assert!(
            matches!(err, Y4mError::Io { source } if source.kind() == io::ErrorKind::BrokenPipe)
        );
        assert_eq!(writer.frames_written(), 0);
    }
}
