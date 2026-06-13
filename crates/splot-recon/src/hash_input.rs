// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Canonical decoded-frame hash input byte serialization.

use std::io::{self, Write};

use crate::{BitDepth, DecodedFrame, Plane, ReconError, ReconSample, Result};

/// Canonical byte-stream view used as input to future decoded-frame hashes.
///
/// This type serializes already materialized decoded output samples following
/// AV2 § 6.16.13 sample-byte conversion for the repository-owned
/// `av2-output-samples-v1` byte stream. It does not compute SHA-256, verify AV2
/// decoded-frame-hash metadata, apply film grain, or determine output order.
#[derive(Clone, Copy, Debug)]
pub struct DecodedFrameHashInput<'a, T: ReconSample> {
    frame: &'a DecodedFrame<T>,
}

impl<'a, T: ReconSample> DecodedFrameHashInput<'a, T> {
    /// Repository-owned canonical decoded-output sample byte stream identifier.
    pub const BYTE_STREAM_ID: &'static str = "av2-output-samples-v1";

    /// Hash-input variant for raw § 7.21.2 intermediate output samples.
    pub const VARIANT_ID: &'static str = "raw_intermediate_output";

    /// Creates a byte-stream view over `frame`.
    pub const fn new(frame: &'a DecodedFrame<T>) -> Self {
        Self { frame }
    }

    /// Returns the decoded frame used as byte-stream input.
    pub const fn frame(&self) -> &'a DecodedFrame<T> {
        self.frame
    }

    /// Returns the exact number of bytes that [`Self::write_to`] will emit.
    ///
    /// # Errors
    /// Returns [`ReconError::ArithmeticOverflow`] if the visible sample count or
    /// byte count overflows `usize`.
    pub fn byte_len(&self) -> Result<usize> {
        let bytes_per_sample = bytes_per_sample(self.frame.bit_depth());
        let mut total = 0usize;

        for plane in self.planes() {
            let plane_len = visible_plane_byte_len(plane, bytes_per_sample)?;
            total = total
                .checked_add(plane_len)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: BYTE_LEN_OVERFLOW_CONTEXT,
                })?;
        }

        Ok(total)
    }

    /// Writes canonical decoded output sample bytes to `writer`.
    ///
    /// This method may leave `writer` partially written if the writer returns an
    /// error. Writer failures are propagated as [`io::Error`] values and are not
    /// wrapped in [`ReconError`].
    pub fn write_to<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        let bit_depth = self.frame.bit_depth();
        for plane in self.planes() {
            write_visible_plane(bit_depth, plane, writer)?;
        }
        Ok(())
    }

    fn planes(&self) -> impl Iterator<Item = &'a Plane<T>> {
        [Some(self.frame.y()), self.frame.u(), self.frame.v()]
            .into_iter()
            .flatten()
    }
}

const BYTE_LEN_OVERFLOW_CONTEXT: &str = "decoded frame hash input byte length";

const fn bytes_per_sample(bit_depth: BitDepth) -> usize {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn visible_plane_byte_len<T: ReconSample>(
    plane: &Plane<T>,
    bytes_per_sample: usize,
) -> Result<usize> {
    let visible_size = plane.visible_size();
    let sample_count = visible_size
        .width()
        .checked_mul(visible_size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: BYTE_LEN_OVERFLOW_CONTEXT,
        })?;
    sample_count
        .checked_mul(bytes_per_sample)
        .ok_or(ReconError::ArithmeticOverflow {
            context: BYTE_LEN_OVERFLOW_CONTEXT,
        })
}

fn write_visible_plane<T: ReconSample, W: Write + ?Sized>(
    bit_depth: BitDepth,
    plane: &Plane<T>,
    writer: &mut W,
) -> io::Result<()> {
    for row in plane.visible_rows() {
        for sample in row {
            write_sample(bit_depth, sample.to_u16(), writer)?;
        }
    }
    Ok(())
}

fn write_sample<W: Write + ?Sized>(
    bit_depth: BitDepth,
    sample: u16,
    writer: &mut W,
) -> io::Result<()> {
    match bit_depth {
        BitDepth::Eight => writer.write_all(&[sample as u8]),
        BitDepth::Ten => writer.write_all(&sample.to_le_bytes()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
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

    fn bytes<T: ReconSample>(frame: &DecodedFrame<T>) -> Vec<u8> {
        let input = DecodedFrameHashInput::new(frame);
        let mut bytes = Vec::new();
        input.write_to(&mut bytes).unwrap();
        assert_eq!(input.byte_len().unwrap(), bytes.len());
        bytes
    }

    #[test]
    fn identifiers_are_stable() {
        assert_eq!(
            DecodedFrameHashInput::<u8>::BYTE_STREAM_ID,
            "av2-output-samples-v1"
        );
        assert_eq!(
            DecodedFrameHashInput::<u8>::VARIANT_ID,
            "raw_intermediate_output"
        );
    }

    #[test]
    fn monochrome_visible_rows_exclude_stride_and_padding() {
        let storage = size(4, 3);
        let visible = rect(1, 1, 2, 2);
        let y = plane(
            storage,
            5,
            visible,
            vec![
                90_u8, 91, 92, 93, 94, 100, 101, 102, 103, 104, 110, 111, 112, 113, 114,
            ],
        );
        let frame = mono_frame(0, BitDepth::Eight, storage, visible, y);

        assert_eq!(bytes(&frame), vec![101, 102, 111, 112]);
    }

    #[test]
    fn eight_bit_u16_storage_emits_one_byte_per_sample() {
        let storage = size(3, 1);
        let visible = rect(0, 0, 3, 1);
        let y = plane(storage, 3, visible, vec![1_u16, 2, 255]);
        let frame = mono_frame(0, BitDepth::Eight, storage, visible, y);

        assert_eq!(bytes(&frame), vec![1, 2, 255]);
    }

    #[test]
    fn ten_bit_samples_emit_little_endian_pairs() {
        let storage = size(3, 1);
        let visible = rect(0, 0, 3, 1);
        let y = plane(storage, 3, visible, vec![1_u16, 0x0102, 1023]);
        let frame = mono_frame(0, BitDepth::Ten, storage, visible, y);

        assert_eq!(bytes(&frame), vec![1, 0, 2, 1, 255, 3]);
    }

    #[test]
    fn ten_bit_yuv_samples_emit_little_endian_y_then_u_then_v() {
        let luma_size = size(2, 2);
        let luma_rect = rect(0, 0, 2, 2);
        let chroma_size = size(1, 1);
        let chroma_rect = rect(0, 0, 1, 1);
        let y = plane(luma_size, 2, luma_rect, vec![1_u16, 0x0102, 511, 1023]);
        let u = plane(chroma_size, 1, chroma_rect, vec![33_u16]);
        let v = plane(chroma_size, 1, chroma_rect, vec![0x0201_u16]);
        let frame = yuv_frame(
            0,
            BitDepth::Ten,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
            FramePlanes::new(y, Some(u), Some(v)),
        );

        assert_eq!(bytes(&frame), vec![1, 0, 2, 1, 255, 1, 255, 3, 33, 0, 1, 2]);
    }

    #[test]
    fn yuv420_odd_luma_dimensions_emit_y_then_u_then_v() {
        let luma_size = size(3, 3);
        let luma_rect = rect(0, 0, 3, 3);
        let chroma_size = size(2, 2);
        let chroma_rect = rect(0, 0, 2, 2);
        let y = plane(luma_size, 3, luma_rect, vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 9]);
        let u = plane(chroma_size, 2, chroma_rect, vec![10_u8, 11, 12, 13]);
        let v = plane(chroma_size, 2, chroma_rect, vec![20_u8, 21, 22, 23]);
        let frame = yuv_frame(
            0,
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
            FramePlanes::new(y, Some(u), Some(v)),
        );

        assert_eq!(
            bytes(&frame),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 20, 21, 22, 23]
        );
    }

    #[test]
    fn yuv422_and_yuv444_emit_y_then_u_then_v_with_expected_lengths() {
        let luma_size = size(3, 2);
        let luma_rect = rect(0, 0, 3, 2);

        let y422 = plane(luma_size, 3, luma_rect, vec![1_u8, 2, 3, 4, 5, 6]);
        let u422 = plane(size(2, 2), 2, rect(0, 0, 2, 2), vec![10_u8, 11, 12, 13]);
        let v422 = plane(size(2, 2), 2, rect(0, 0, 2, 2), vec![20_u8, 21, 22, 23]);
        let frame422 = yuv_frame(
            0,
            BitDepth::Eight,
            PixelFormat::Yuv422,
            luma_size,
            luma_rect,
            FramePlanes::new(y422, Some(u422), Some(v422)),
        );
        assert_eq!(
            bytes(&frame422),
            vec![1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 20, 21, 22, 23]
        );

        let y444 = plane(luma_size, 3, luma_rect, vec![31_u8, 32, 33, 34, 35, 36]);
        let u444 = plane(luma_size, 3, luma_rect, vec![41_u8, 42, 43, 44, 45, 46]);
        let v444 = plane(luma_size, 3, luma_rect, vec![51_u8, 52, 53, 54, 55, 56]);
        let frame444 = yuv_frame(
            0,
            BitDepth::Eight,
            PixelFormat::Yuv444,
            luma_size,
            luma_rect,
            FramePlanes::new(y444, Some(u444), Some(v444)),
        );
        assert_eq!(
            bytes(&frame444),
            vec![
                31, 32, 33, 34, 35, 36, 41, 42, 43, 44, 45, 46, 51, 52, 53, 54, 55, 56
            ]
        );
    }

    #[test]
    fn output_metadata_and_coded_padding_do_not_change_bytes() {
        let small_size = size(1, 1);
        let small_visible = rect(0, 0, 1, 1);
        let compact = mono_frame(
            0,
            BitDepth::Eight,
            small_size,
            small_visible,
            plane(small_size, 1, small_visible, vec![77_u8]),
        );

        let padded_size = size(3, 3);
        let padded_visible = rect(1, 1, 1, 1);
        let padded = mono_frame(
            1,
            BitDepth::Eight,
            padded_size,
            padded_visible,
            plane(
                padded_size,
                4,
                padded_visible,
                vec![1_u8, 2, 3, 4, 5, 77, 7, 8, 9, 10, 11, 12],
            ),
        );

        assert_eq!(bytes(&compact), vec![77]);
        assert_eq!(bytes(&padded), vec![77]);
    }

    #[test]
    fn writer_error_is_propagated() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let storage = size(1, 1);
        let visible = rect(0, 0, 1, 1);
        let frame = mono_frame(
            0,
            BitDepth::Eight,
            storage,
            visible,
            plane(storage, 1, visible, vec![5_u8]),
        );

        let mut writer = FailingWriter;
        let err = DecodedFrameHashInput::new(&frame)
            .write_to(&mut writer)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
