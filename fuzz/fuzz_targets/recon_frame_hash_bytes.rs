// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::{
    fmt::Write as _,
    io::{self, Write},
};

use libfuzzer_sys::fuzz_target;
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameHash, DecodedFrameHashInput, DecodedFrameInfo, FramePlanes,
    OutputIndex, PixelFormat, Plane, PlaneRect, PlaneSize, ReconSample,
};

const MAX_LUMA_WIDTH: usize = 16;
const MAX_LUMA_HEIGHT: usize = 16;
const MAX_CROP_ORIGIN: usize = 2;
const MAX_STORAGE_PADDING: usize = 2;
const MAX_STRIDE_PADDING: usize = 2;
const MAX_FAILING_WRITER_BYTES: usize = 128;

fuzz_target!(|data: &[u8]| {
    let header = Header::new(data);
    let bit_depth = if header.flags & 0b0000_0001 == 0 {
        BitDepth::Eight
    } else {
        BitDepth::Ten
    };
    let pixel_format = match (header.flags >> 1) & 0b0000_0011 {
        0 => PixelFormat::Monochrome,
        1 => PixelFormat::Yuv420,
        2 => PixelFormat::Yuv422,
        _ => PixelFormat::Yuv444,
    };

    match (bit_depth, header.flags & 0b0000_1000 != 0) {
        (BitDepth::Eight, true) => run_case::<u16>(header, bit_depth, pixel_format),
        (BitDepth::Eight, false) => run_case::<u8>(header, bit_depth, pixel_format),
        (BitDepth::Ten, _) => run_case::<u16>(header, bit_depth, pixel_format),
    }
});

fn run_case<T: ReconSample>(header: Header<'_>, bit_depth: BitDepth, pixel_format: PixelFormat) {
    let Some(model) = FrameModel::new(header, bit_depth, pixel_format) else {
        return;
    };
    let Some(frame) = model.build_frame::<T>(0) else {
        return;
    };

    let hash_input = DecodedFrameHashInput::new(&frame);
    assert_eq!(
        hash_input.frame().output_index(),
        OutputIndex::new(model.output_index)
    );

    let expected_byte_len = model.manual_visible_byte_len();
    assert_eq!(hash_input.byte_len().ok(), Some(expected_byte_len));

    let mut bytes = Vec::new();
    assert!(hash_input.write_to(&mut bytes).is_ok());
    assert_eq!(bytes.len(), expected_byte_len);
    assert_eq!(bytes, model.manual_visible_bytes());

    let mut repeated_bytes = Vec::new();
    assert!(hash_input.write_to(&mut repeated_bytes).is_ok());
    assert_eq!(repeated_bytes, bytes);

    let first_hash = hash_input.compute_hash();
    let second_hash = hash_input.compute_hash();
    assert_eq!(first_hash, second_hash);
    assert_hash_contract(first_hash);

    let Some(padded_frame) = model.build_frame::<T>(1) else {
        return;
    };
    let padded_hash_input = DecodedFrameHashInput::new(&padded_frame);
    assert_eq!(
        padded_hash_input.frame().output_index(),
        OutputIndex::new(model.output_index.wrapping_add(1))
    );

    let mut padded_bytes = Vec::new();
    assert!(padded_hash_input.write_to(&mut padded_bytes).is_ok());
    assert_eq!(padded_bytes, bytes);
    assert_eq!(padded_hash_input.compute_hash(), first_hash);

    if model.failing_writer {
        let mut writer = FailAfterBytes::new(model.failing_writer_budget);
        let result = hash_input.write_to(&mut writer);
        if model.failing_writer_budget < expected_byte_len {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
            assert_eq!(writer.bytes_written, expected_byte_len);
        }
    }
}

fn assert_hash_contract(hash: DecodedFrameHash) {
    assert_eq!(DecodedFrameHash::CONTRACT_ID, "splot.decoded_frame_hash");
    assert_eq!(DecodedFrameHash::CONTRACT_VERSION, 1);
    assert_eq!(DecodedFrameHash::ALGORITHM_ID, "splot-dfh-sha256-v1");
    assert_eq!(DecodedFrameHash::BYTE_STREAM_ID, "av2-output-samples-v1");
    assert_eq!(DecodedFrameHash::VARIANT_ID, "raw_intermediate_output");
    assert_eq!(DecodedFrameHash::BYTE_LEN, 32);
    assert_eq!(
        DecodedFrameHashInput::<u8>::BYTE_STREAM_ID,
        DecodedFrameHash::BYTE_STREAM_ID
    );
    assert_eq!(
        DecodedFrameHashInput::<u8>::VARIANT_ID,
        DecodedFrameHash::VARIANT_ID
    );
    assert_eq!(hash.as_bytes().len(), DecodedFrameHash::BYTE_LEN);
    assert_eq!(hash.as_ref(), hash.as_bytes());

    let hex = hash.to_hex();
    assert_eq!(hex.len(), 64);
    assert!(
        hex.bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );

    let mut display = String::new();
    assert!(write!(&mut display, "{hash}").is_ok());
    assert_eq!(display, hex);
}

struct FrameModel {
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    output_index: u64,
    failing_writer: bool,
    failing_writer_budget: usize,
    luma_storage: PlaneSize,
    luma_visible: PlaneRect,
    y: PlaneModel,
    u: Option<PlaneModel>,
    v: Option<PlaneModel>,
}

impl FrameModel {
    fn new(header: Header<'_>, bit_depth: BitDepth, pixel_format: PixelFormat) -> Option<Self> {
        let visible_width = 1 + usize::from(header.width) % MAX_LUMA_WIDTH;
        let visible_height = 1 + usize::from(header.height) % MAX_LUMA_HEIGHT;
        let crop_x = crop_origin(pixel_format, header.crop_enabled, header.crop_x_seed, true);
        let crop_y = crop_origin(pixel_format, header.crop_enabled, header.crop_y_seed, false);
        let storage_pad_x = usize::from(header.storage_padding >> 4) % (MAX_STORAGE_PADDING + 1);
        let storage_pad_y = usize::from(header.storage_padding) % (MAX_STORAGE_PADDING + 1);
        let stride_padding = usize::from(header.stride_padding) % (MAX_STRIDE_PADDING + 1);

        let luma_storage = PlaneSize::new(
            crop_x + visible_width + storage_pad_x,
            crop_y + visible_height + storage_pad_y,
        )
        .ok()?;
        let luma_visible = PlaneRect::new(crop_x, crop_y, visible_width, visible_height).ok()?;

        let mut samples = SampleReader::new(header.samples, bit_depth);
        let y = PlaneModel::new(
            luma_storage,
            luma_visible,
            stride_padding,
            &mut samples,
            header.crop_x_seed,
        )?;

        let (u, v) = match pixel_format.chroma_size(luma_visible.size()).ok()? {
            None => (None, None),
            Some(chroma_visible_size) => {
                let chroma_x = crop_x >> usize::from(pixel_format.subsampling_x());
                let chroma_y = crop_y >> usize::from(pixel_format.subsampling_y());
                let chroma_pad_x = usize::from(header.crop_x_seed >> 4) % (MAX_STORAGE_PADDING + 1);
                let chroma_pad_y = usize::from(header.crop_y_seed >> 4) % (MAX_STORAGE_PADDING + 1);
                let chroma_storage = PlaneSize::new(
                    chroma_x + chroma_visible_size.width() + chroma_pad_x,
                    chroma_y + chroma_visible_size.height() + chroma_pad_y,
                )
                .ok()?;
                let chroma_visible = PlaneRect::new(
                    chroma_x,
                    chroma_y,
                    chroma_visible_size.width(),
                    chroma_visible_size.height(),
                )
                .ok()?;
                let u = PlaneModel::new(
                    chroma_storage,
                    chroma_visible,
                    stride_padding,
                    &mut samples,
                    header.crop_y_seed,
                )?;
                let v = PlaneModel::new(
                    chroma_storage,
                    chroma_visible,
                    stride_padding,
                    &mut samples,
                    header.storage_padding,
                )?;
                (Some(u), Some(v))
            }
        };

        let failing_writer_budget = usize::from(
            header.output_index ^ header.crop_x_seed ^ header.crop_y_seed ^ header.storage_padding,
        ) % (MAX_FAILING_WRITER_BYTES + 1);

        Some(Self {
            bit_depth,
            pixel_format,
            output_index: u64::from(header.output_index),
            failing_writer: header.failing_writer,
            failing_writer_budget,
            luma_storage,
            luma_visible,
            y,
            u,
            v,
        })
    }

    fn build_frame<T: ReconSample>(&self, padding_variant: u8) -> Option<DecodedFrame<T>> {
        let y = self.y.build_plane::<T>(self.bit_depth, padding_variant)?;
        let u = match self.u.as_ref() {
            Some(plane) => Some(plane.build_plane::<T>(self.bit_depth, padding_variant)?),
            None => None,
        };
        let v = match self.v.as_ref() {
            Some(plane) => Some(plane.build_plane::<T>(self.bit_depth, padding_variant)?),
            None => None,
        };
        let info = DecodedFrameInfo::new(
            OutputIndex::new(self.output_index.wrapping_add(u64::from(padding_variant))),
            self.bit_depth,
            self.pixel_format,
            self.luma_storage,
            self.luma_visible,
        )
        .ok()?;
        DecodedFrame::try_new(info, FramePlanes::new(y, u, v)).ok()
    }

    fn manual_visible_byte_len(&self) -> usize {
        let bytes_per_sample = bytes_per_sample(self.bit_depth);
        self.visible_sample_count() * bytes_per_sample
    }

    fn manual_visible_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.manual_visible_byte_len());
        self.y.push_visible_bytes(self.bit_depth, &mut bytes);
        if let Some(u) = self.u.as_ref() {
            u.push_visible_bytes(self.bit_depth, &mut bytes);
        }
        if let Some(v) = self.v.as_ref() {
            v.push_visible_bytes(self.bit_depth, &mut bytes);
        }
        bytes
    }

    fn visible_sample_count(&self) -> usize {
        let mut count = self.y.visible_sample_count();
        if let Some(u) = self.u.as_ref() {
            count += u.visible_sample_count();
        }
        if let Some(v) = self.v.as_ref() {
            count += v.visible_sample_count();
        }
        count
    }
}

#[derive(Clone)]
struct PlaneModel {
    storage: PlaneSize,
    visible: PlaneRect,
    stride: usize,
    visible_samples: Vec<u16>,
    padding_seed: u8,
}

impl PlaneModel {
    fn new(
        storage: PlaneSize,
        visible: PlaneRect,
        stride_padding: usize,
        samples: &mut SampleReader<'_>,
        padding_seed: u8,
    ) -> Option<Self> {
        let stride = storage.width() + stride_padding;
        let visible_count = visible.width().checked_mul(visible.height())?;
        let visible_samples = samples.take(visible_count);
        Some(Self {
            storage,
            visible,
            stride,
            visible_samples,
            padding_seed,
        })
    }

    fn build_plane<T: ReconSample>(
        &self,
        bit_depth: BitDepth,
        padding_variant: u8,
    ) -> Option<Plane<T>> {
        let sample_count = self.stride.checked_mul(self.storage.height())?;
        let mut samples = Vec::new();
        samples.try_reserve_exact(sample_count).ok()?;

        let mut visible_index = 0usize;
        for y in 0..self.storage.height() {
            for x in 0..self.stride {
                let value = if x >= self.visible.x()
                    && x < self.visible.x() + self.visible.width()
                    && y >= self.visible.y()
                    && y < self.visible.y() + self.visible.height()
                {
                    let value = self.visible_samples[visible_index];
                    visible_index += 1;
                    value
                } else {
                    padding_sample(bit_depth, self.padding_seed, padding_variant, x, y)
                };
                samples.push(T::try_from_u16(value).ok()?);
            }
        }

        Plane::from_vec(self.storage, self.stride, self.visible, samples).ok()
    }

    fn push_visible_bytes(&self, bit_depth: BitDepth, bytes: &mut Vec<u8>) {
        for sample in self.visible_samples.iter().copied() {
            push_sample_bytes(bit_depth, sample, bytes);
        }
    }

    fn visible_sample_count(&self) -> usize {
        self.visible_samples.len()
    }
}

#[derive(Clone, Copy)]
struct Header<'a> {
    flags: u8,
    width: u8,
    height: u8,
    output_index: u8,
    crop_x_seed: u8,
    crop_y_seed: u8,
    storage_padding: u8,
    stride_padding: u8,
    crop_enabled: bool,
    failing_writer: bool,
    samples: &'a [u8],
}

impl<'a> Header<'a> {
    fn new(data: &'a [u8]) -> Self {
        let byte = |index| data.get(index).copied().unwrap_or(0);
        let flags = byte(0);
        Self {
            flags,
            width: byte(1),
            height: byte(2),
            output_index: byte(3),
            crop_x_seed: byte(4),
            crop_y_seed: byte(5),
            storage_padding: byte(6),
            stride_padding: byte(7),
            crop_enabled: flags & 0b0001_0000 != 0,
            failing_writer: flags & 0b0010_0000 != 0,
            samples: data.get(8..).unwrap_or(&[]),
        }
    }
}

struct SampleReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    bit_depth: BitDepth,
}

impl<'a> SampleReader<'a> {
    const fn new(bytes: &'a [u8], bit_depth: BitDepth) -> Self {
        Self {
            bytes,
            offset: 0,
            bit_depth,
        }
    }

    fn take(&mut self, count: usize) -> Vec<u16> {
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            samples.push(self.sample());
        }
        samples
    }

    fn sample(&mut self) -> u16 {
        match self.bit_depth {
            BitDepth::Eight => u16::from(self.byte()),
            BitDepth::Ten => {
                let high = u16::from(self.byte());
                let low = u16::from(self.byte() & 0b0000_0011);
                ((high << 2) | low) & self.bit_depth.max_sample()
            }
        }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.offset).copied().unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        byte
    }
}

const fn bytes_per_sample(bit_depth: BitDepth) -> usize {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn push_sample_bytes(bit_depth: BitDepth, sample: u16, bytes: &mut Vec<u8>) {
    match bit_depth {
        BitDepth::Eight => bytes.push(sample as u8),
        BitDepth::Ten => bytes.extend_from_slice(&sample.to_le_bytes()),
    }
}

fn padding_sample(bit_depth: BitDepth, seed: u8, padding_variant: u8, x: usize, y: usize) -> u16 {
    let mixed = usize::from(seed)
        .wrapping_add(usize::from(padding_variant) * 37)
        .wrapping_add(x * 13)
        .wrapping_add(y * 17);
    (mixed as u16) & bit_depth.max_sample()
}

fn crop_origin(pixel_format: PixelFormat, enabled: bool, seed: u8, horizontal: bool) -> usize {
    if !enabled {
        return 0;
    }

    let subsampling = if pixel_format.is_monochrome() {
        0
    } else if horizontal {
        pixel_format.subsampling_x()
    } else {
        pixel_format.subsampling_y()
    };

    if subsampling == 0 {
        1 + usize::from(seed) % MAX_CROP_ORIGIN
    } else {
        MAX_CROP_ORIGIN
    }
}

#[derive(Debug)]
struct FailAfterBytes {
    bytes_written: usize,
    max_bytes: usize,
}

impl FailAfterBytes {
    const fn new(max_bytes: usize) -> Self {
        Self {
            bytes_written: 0,
            max_bytes,
        }
    }
}

impl Write for FailAfterBytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes_written >= self.max_bytes {
            return Err(io::Error::other("fuzz writer byte budget exhausted"));
        }
        let allowed = (self.max_bytes - self.bytes_written).min(buf.len());
        self.bytes_written += allowed;
        Ok(allowed)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
