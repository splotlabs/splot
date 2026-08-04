// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Canonical decoded-frame hash input byte serialization and digest computation.

use std::{
    fmt,
    io::{self, Write},
};

use sha2::{Digest, Sha256};

use crate::{
    BitDepth, DecodedFrame, DecodedFrameInfo, Plane, PlaneSize, ReconError, ReconSample, Result,
};

/// Byte length of a `splot-dfh-sha256-v1` digest.
const SHA256_DIGEST_BYTES: usize = 32;
const SHA256_HEX_CHARS: usize = SHA256_DIGEST_BYTES * 2;
const LOWER_HEX_DIGITS: &str = "0123456789abcdef";
const BYTE_STREAM_ID: &str = "av2-output-samples-v1";
const VARIANT_ID: &str = "raw_intermediate_output";

/// Repository-owned decoded-frame hash digest.
///
/// The digest is SHA-256 over the canonical `av2-output-samples-v1` byte
/// stream for the `raw_intermediate_output` variant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DecodedFrameHash([u8; SHA256_DIGEST_BYTES]);

impl DecodedFrameHash {
    /// Stable decoded-frame hash contract identifier.
    pub const CONTRACT_ID: &'static str = "splot.decoded_frame_hash";

    /// Stable decoded-frame hash contract version.
    pub const CONTRACT_VERSION: u32 = 1;

    /// Stable digest algorithm identifier.
    pub const ALGORITHM_ID: &'static str = "splot-dfh-sha256-v1";

    /// Stable digest byte-stream identifier.
    pub const BYTE_STREAM_ID: &'static str = BYTE_STREAM_ID;

    /// Stable digest variant identifier.
    pub const VARIANT_ID: &'static str = VARIANT_ID;

    /// Number of raw bytes in the digest.
    pub const BYTE_LEN: usize = SHA256_DIGEST_BYTES;

    /// Returns the raw 32-byte digest.
    pub const fn as_bytes(&self) -> &[u8; SHA256_DIGEST_BYTES] {
        &self.0
    }

    /// Returns the digest as 64 lowercase hexadecimal characters.
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(SHA256_HEX_CHARS);
        for byte in self.0.iter().copied() {
            push_lower_hex_byte(byte, &mut hex);
        }
        hex
    }
}

impl AsRef<[u8]> for DecodedFrameHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl fmt::Display for DecodedFrameHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write_lower_hex_byte(f, byte)?;
        }
        Ok(())
    }
}

/// Canonical byte-stream view used as input to decoded-frame hashes.
///
/// This type serializes already materialized decoded output samples following
/// AV2 § 6.16.13 sample-byte conversion
/// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13`)
/// for the repository-owned `av2-output-samples-v1` byte stream and computes
/// `splot-dfh-sha256-v1` over that same stream. It does not verify AV2
/// decoded-frame-hash metadata, apply film grain, or determine output order.
#[derive(Clone, Copy, Debug)]
pub struct DecodedFrameHashInput<'a, T: ReconSample> {
    frame: &'a DecodedFrame<T>,
}

impl<'a, T: ReconSample> DecodedFrameHashInput<'a, T> {
    /// Repository-owned canonical decoded-output sample byte stream identifier.
    pub const BYTE_STREAM_ID: &'static str = BYTE_STREAM_ID;

    /// Hash-input variant for raw § 7.21.2 intermediate output samples
    /// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-21-2`).
    pub const VARIANT_ID: &'static str = VARIANT_ID;

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
        visible_byte_len(self.frame.info())
    }

    /// Writes canonical decoded output sample bytes to `writer`.
    ///
    /// This method writes bounded groups of visible rows and may leave `writer`
    /// partially written if the writer returns an error.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] if `writer` fails, or if an internal
    /// row-buffer capacity/allocation fails. These are not wrapped in
    /// [`ReconError`].
    pub fn write_to<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        let bit_depth = self.frame.bit_depth();
        for plane in self.planes() {
            write_visible_plane(bit_depth, plane, writer)?;
        }
        Ok(())
    }

    /// Computes the repository-owned `splot-dfh-sha256-v1` digest.
    ///
    /// This method hashes visible samples directly and does not call
    /// [`Self::write_to`], so it is infallible for an already validated
    /// decoded frame and does not allocate a row buffer.
    pub fn compute_hash(&self) -> DecodedFrameHash {
        let mut hasher = Sha256::new();
        let bit_depth = self.frame.bit_depth();
        for plane in self.planes() {
            hash_visible_plane(bit_depth, plane, &mut hasher);
        }
        DecodedFrameHash(hasher.finalize().into())
    }

    fn planes(&self) -> impl Iterator<Item = &'a Plane<T>> {
        [Some(self.frame.y()), self.frame.u(), self.frame.v()]
            .into_iter()
            .flatten()
    }
}

/// Returns the exact number of canonical decoded-output sample bytes a frame
/// described by `info` serializes to.
///
/// The visible plane sizes a [`DecodedFrame`] is validated against are fully
/// determined by its [`DecodedFrameInfo`], so this answers the byte length for a
/// frame whose samples are not materialized yet.
///
/// # Errors
/// Returns [`ReconError::ArithmeticOverflow`] if the chroma size derivation, the
/// visible sample count, or the byte count overflows `usize`.
pub fn visible_byte_len(info: DecodedFrameInfo) -> Result<usize> {
    let bytes_per_sample = bytes_per_sample(info.bit_depth());
    let luma_size = info.visible_luma_rect().size();
    let chroma_size = info.pixel_format().chroma_size(luma_size)?;
    let mut total = 0usize;

    for size in [Some(luma_size), chroma_size, chroma_size]
        .into_iter()
        .flatten()
    {
        let plane_len = visible_plane_byte_len(size, bytes_per_sample)?;
        total = total
            .checked_add(plane_len)
            .ok_or(ReconError::ArithmeticOverflow {
                context: BYTE_LEN_OVERFLOW_CONTEXT,
            })?;
    }

    Ok(total)
}

const BYTE_LEN_OVERFLOW_CONTEXT: &str = "decoded frame hash input byte length";

const fn bytes_per_sample(bit_depth: BitDepth) -> usize {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn visible_plane_byte_len(visible_size: PlaneSize, bytes_per_sample: usize) -> Result<usize> {
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
    const WRITE_BATCH_BYTES: usize = 64 * 1024;

    let bytes_per_sample = bytes_per_sample(bit_depth);
    let row_byte_len = plane
        .visible_size()
        .width()
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "decoded frame hash input row byte length overflow",
            )
        })?;
    let rows_per_batch = WRITE_BATCH_BYTES
        .checked_div(row_byte_len)
        .unwrap_or(1)
        .max(1);
    let batch_byte_len = row_byte_len.checked_mul(rows_per_batch).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "decoded frame hash input write batch length overflow",
        )
    })?;
    let mut batch = Vec::new();
    batch.try_reserve_exact(batch_byte_len).map_err(|err| {
        io::Error::other(format!(
            "decoded frame hash input write buffer allocation failed: {err}"
        ))
    })?;

    for row in plane.visible_rows() {
        let start = batch.len();
        batch.resize(start + row_byte_len, 0);
        fill_sample_bytes(bit_depth, row, &mut batch[start..]);
        if batch.len() == batch_byte_len {
            writer.write_all(&batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        writer.write_all(&batch)?;
    }
    Ok(())
}

/// Serializes one visible row into `row_bytes` per § 6.16.13: one byte per
/// 8-bit sample, two little-endian bytes per 10-bit sample. `row_bytes` must
/// already hold the row's exact byte length; any tail beyond the zipped
/// samples is left untouched, so callers size it exactly.
fn fill_sample_bytes<T: ReconSample>(bit_depth: BitDepth, row: &[T], row_bytes: &mut [u8]) {
    match bit_depth {
        BitDepth::Eight => {
            for (byte, sample) in row_bytes.iter_mut().zip(row) {
                *byte = sample.to_u16() as u8;
            }
        }
        BitDepth::Ten => {
            for (pair, sample) in row_bytes.chunks_exact_mut(2).zip(row) {
                // splot-copy-ok: serialize a decoded sample into the frame-hash input byte stream
                pair.copy_from_slice(&sample.to_u16().to_le_bytes());
            }
        }
    }
}

fn hash_visible_plane<T: ReconSample>(bit_depth: BitDepth, plane: &Plane<T>, hasher: &mut Sha256) {
    let bytes_per_sample = bytes_per_sample(bit_depth);
    let mut chunk = [0u8; 4096];
    let samples_per_chunk = chunk.len() / bytes_per_sample;
    for row in plane.visible_rows() {
        for samples in row.chunks(samples_per_chunk) {
            let filled = &mut chunk[..samples.len() * bytes_per_sample];
            fill_sample_bytes(bit_depth, samples, filled);
            hasher.update(&*filled);
        }
    }
}

fn push_lower_hex_byte(byte: u8, hex: &mut String) {
    hex.push_str(lower_hex_digit(byte >> 4));
    hex.push_str(lower_hex_digit(byte & 0x0f));
}

fn write_lower_hex_byte(f: &mut fmt::Formatter<'_>, byte: u8) -> fmt::Result {
    f.write_str(lower_hex_digit(byte >> 4))?;
    f.write_str(lower_hex_digit(byte & 0x0f))
}

fn lower_hex_digit(nibble: u8) -> &'static str {
    let index = usize::from(nibble);
    LOWER_HEX_DIGITS.get(index..index + 1).unwrap_or("")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

    const MONOCHROME_VISIBLE_ROWS_SHA256: &str =
        "224a16a96e1b0b8b96ee83ec8146eed4144a1fabeb478cceb9ca26cb22e6ed0f";
    const EIGHT_BIT_U16_SHA256: &str =
        "0526d0e18ea19dfaad9d79166bec1e18d6221ef6b1830385fe9bf67022ed5f96";
    const TEN_BIT_MONOCHROME_SHA256: &str =
        "2ac60eed0d8e830b4c8807c24809d6e4511cada2c1457a89fa1cc03ca00efd72";
    const TEN_BIT_YUV420_SHA256: &str =
        "a6f36b1a26e02def03117c77bac50366d6a7c77e37bcb874fd5dcbcd34eee99c";
    const ODD_SIZE_YUV420_SHA256: &str =
        "93d30f9d5c5bca8daf2ecaef55337ac84ba835d871b199bd5c581fcd53dff922";

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

    fn assert_digest_matches_emitted_bytes<T: ReconSample>(frame: &DecodedFrame<T>) {
        let input = DecodedFrameHashInput::new(frame);
        let emitted = bytes(frame);
        let expected = Sha256::digest(&emitted);

        assert_eq!(input.compute_hash().as_ref(), expected.as_slice());
    }

    #[test]
    fn identifiers_are_stable() {
        assert_eq!(DecodedFrameHash::CONTRACT_ID, "splot.decoded_frame_hash");
        assert_eq!(DecodedFrameHash::CONTRACT_VERSION, 1);
        assert_eq!(DecodedFrameHash::ALGORITHM_ID, "splot-dfh-sha256-v1");
        assert_eq!(DecodedFrameHash::BYTE_STREAM_ID, "av2-output-samples-v1");
        assert_eq!(DecodedFrameHash::VARIANT_ID, "raw_intermediate_output");
        assert_eq!(DecodedFrameHash::BYTE_LEN, 32);
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
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            MONOCHROME_VISIBLE_ROWS_SHA256
        );
    }

    #[test]
    fn eight_bit_u16_storage_emits_one_byte_per_sample() {
        let storage = size(3, 1);
        let visible = rect(0, 0, 3, 1);
        let y = plane(storage, 3, visible, vec![1_u16, 2, 255]);
        let frame = mono_frame(0, BitDepth::Eight, storage, visible, y);

        assert_eq!(bytes(&frame), vec![1, 2, 255]);
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            EIGHT_BIT_U16_SHA256
        );
    }

    #[test]
    fn ten_bit_samples_emit_little_endian_pairs() {
        let storage = size(3, 1);
        let visible = rect(0, 0, 3, 1);
        let y = plane(storage, 3, visible, vec![1_u16, 0x0102, 1023]);
        let frame = mono_frame(0, BitDepth::Ten, storage, visible, y);

        assert_eq!(bytes(&frame), vec![1, 0, 2, 1, 255, 3]);
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            TEN_BIT_MONOCHROME_SHA256
        );
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
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            TEN_BIT_YUV420_SHA256
        );
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
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            ODD_SIZE_YUV420_SHA256
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
        assert_eq!(
            DecodedFrameHashInput::new(&compact).compute_hash(),
            DecodedFrameHashInput::new(&padded).compute_hash()
        );
    }

    #[test]
    fn digest_exposes_raw_bytes_lowercase_hex_display_and_as_ref() {
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
        let frame = mono_frame(7, BitDepth::Eight, storage, visible, y);
        let hash = DecodedFrameHashInput::new(&frame).compute_hash();

        assert_eq!(hash.as_bytes().len(), DecodedFrameHash::BYTE_LEN);
        assert_eq!(hash.as_ref(), hash.as_bytes());
        assert_eq!(hash.to_hex(), MONOCHROME_VISIBLE_ROWS_SHA256);
        assert_eq!(hash.to_string(), hash.to_hex());
        assert_eq!(hash.to_hex().len(), 64);
        assert!(hash.to_hex().chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(hash.to_hex().chars().all(|ch| !ch.is_ascii_uppercase()));
    }

    #[test]
    fn digest_matches_sha256_over_write_to_bytes() {
        let luma_size = size(3, 3);
        let luma_rect = rect(0, 0, 3, 3);
        let chroma_size = size(2, 2);
        let chroma_rect = rect(0, 0, 2, 2);
        let y = plane(luma_size, 3, luma_rect, vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 9]);
        let u = plane(chroma_size, 2, chroma_rect, vec![10_u8, 11, 12, 13]);
        let v = plane(chroma_size, 2, chroma_rect, vec![20_u8, 21, 22, 23]);
        let frame = yuv_frame(
            11,
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
            FramePlanes::new(y, Some(u), Some(v)),
        );
        assert_digest_matches_emitted_bytes(&frame);

        let ten_bit_luma_size = size(2, 2);
        let ten_bit_luma_rect = rect(0, 0, 2, 2);
        let ten_bit_chroma_size = size(1, 1);
        let ten_bit_chroma_rect = rect(0, 0, 1, 1);
        let ten_bit_y = plane(
            ten_bit_luma_size,
            2,
            ten_bit_luma_rect,
            vec![1_u16, 0x0102, 511, 1023],
        );
        let ten_bit_u = plane(ten_bit_chroma_size, 1, ten_bit_chroma_rect, vec![33_u16]);
        let ten_bit_v = plane(
            ten_bit_chroma_size,
            1,
            ten_bit_chroma_rect,
            vec![0x0201_u16],
        );
        let ten_bit_frame = yuv_frame(
            12,
            BitDepth::Ten,
            PixelFormat::Yuv420,
            ten_bit_luma_size,
            ten_bit_luma_rect,
            FramePlanes::new(ten_bit_y, Some(ten_bit_u), Some(ten_bit_v)),
        );

        assert_digest_matches_emitted_bytes(&ten_bit_frame);
    }

    #[test]
    fn write_to_batches_each_visible_plane() {
        #[derive(Default)]
        struct CountingWriter {
            writes: usize,
            bytes: Vec<u8>,
        }

        impl Write for CountingWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                // splot-copy-ok: test fixture construction only (accumulates written bytes)
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

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

        let mut writer = CountingWriter::default();
        DecodedFrameHashInput::new(&frame)
            .write_to(&mut writer)
            .unwrap();

        assert_eq!(writer.writes, 3);
        assert_eq!(writer.bytes, vec![1, 0, 2, 1, 255, 1, 255, 3, 33, 0, 1, 2]);
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
