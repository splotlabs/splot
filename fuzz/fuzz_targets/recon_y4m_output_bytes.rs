// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

#[path = "../support/output.rs"]
mod output;
use output::FailAfterBytes;

use libfuzzer_sys::fuzz_target;
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane,
    PlaneRect, PlaneSize, ReconSample, Y4mFrameRate, Y4mWriter,
};

const MAX_LUMA_WIDTH: usize = 16;
const MAX_LUMA_HEIGHT: usize = 16;
const MAX_STORAGE_PADDING: usize = 2;
const MAX_STRIDE_PADDING: usize = 2;
const MAX_EXTRA_FRAMES: usize = 2;
const MAX_FAILING_WRITER_BYTES: usize = 128;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let selector = input.byte();
    let bit_depth = if selector & 0b0000_0001 == 0 {
        BitDepth::Eight
    } else {
        BitDepth::Ten
    };
    let pixel_format = match (selector >> 1) & 0b0000_0011 {
        0 => PixelFormat::Monochrome,
        1 => PixelFormat::Yuv420,
        2 => PixelFormat::Yuv422,
        _ => PixelFormat::Yuv444,
    };

    match (bit_depth, selector & 0b0000_1000 != 0) {
        (BitDepth::Eight, true) => run_y4m_case::<u16>(&mut input, bit_depth, pixel_format),
        (BitDepth::Eight, false) => run_y4m_case::<u8>(&mut input, bit_depth, pixel_format),
        (BitDepth::Ten, _) => run_y4m_case::<u16>(&mut input, bit_depth, pixel_format),
    }
});

fn run_y4m_case<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
) {
    let width = 1 + usize::from(input.byte()) % MAX_LUMA_WIDTH;
    let height = 1 + usize::from(input.byte()) % MAX_LUMA_HEIGHT;
    let Some(first) = frame_from_input::<T>(input, bit_depth, pixel_format, 0, width, height)
    else {
        return;
    };
    let Some(frame_rate) = frame_rate_from_input(input) else {
        return;
    };

    let mode = input.byte();
    if mode & 0b0000_0001 == 0 {
        run_vec_writer(
            input,
            first,
            bit_depth,
            pixel_format,
            width,
            height,
            frame_rate,
            mode,
        );
    } else {
        run_failing_writer(input, &first, frame_rate);
    }
}

fn run_vec_writer<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    first: DecodedFrame<T>,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    frame_rate: Y4mFrameRate,
    mode: u8,
) {
    let mut bytes = Vec::new();
    {
        let Ok(mut writer) = Y4mWriter::from_frame(&mut bytes, &first, frame_rate) else {
            return;
        };
        let _ = writer.write_frame(&first);

        let extra_frames = usize::from(input.byte()) % (MAX_EXTRA_FRAMES + 1);
        for index in 1..=extra_frames {
            let Some(frame) =
                frame_from_input::<T>(input, bit_depth, pixel_format, index as u64, width, height)
            else {
                return;
            };
            let _ = writer.write_frame(&frame);
        }

        if mode & 0b0000_0010 != 0 {
            let mismatch_format = alternate_pixel_format(pixel_format);
            let Some(mismatch) = frame_from_input::<T>(
                input,
                bit_depth,
                mismatch_format,
                u64::from(extra_frames as u32) + 1,
                width,
                height,
            ) else {
                return;
            };
            let _ = writer.write_frame(&mismatch);
        }

        let _ = writer.flush();
    }

    assert!(bytes.starts_with(b"YUV4MPEG2 "));
}

fn run_failing_writer<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    first: &DecodedFrame<T>,
    frame_rate: Y4mFrameRate,
) {
    let budget = usize::from(input.byte()) % (MAX_FAILING_WRITER_BYTES + 1);
    let writer = FailAfterBytes::new(budget);
    let Ok(mut writer) = Y4mWriter::from_frame(writer, first, frame_rate) else {
        return;
    };
    let _ = writer.write_frame(first);
    let _ = writer.flush();
}

fn frame_from_input<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    output_index: u64,
    visible_width: usize,
    visible_height: usize,
) -> Option<DecodedFrame<T>> {
    let luma_storage = storage_size_from_input(input, visible_width, visible_height)?;
    let luma_visible = PlaneRect::new(0, 0, visible_width, visible_height).ok()?;
    let luma_stride = stride_from_input(input, luma_storage);
    let y = plane_from_input::<T>(input, bit_depth, luma_storage, luma_stride, luma_visible)?;

    let visible_luma_size = luma_visible.size();
    let planes = match pixel_format.chroma_size(visible_luma_size).ok()? {
        None => FramePlanes::new(y, None, None),
        Some(chroma_visible_size) => {
            let chroma_storage = storage_size_from_input(
                input,
                chroma_visible_size.width(),
                chroma_visible_size.height(),
            )?;
            let chroma_visible = PlaneRect::new(
                0,
                0,
                chroma_visible_size.width(),
                chroma_visible_size.height(),
            )
            .ok()?;
            let u_stride = stride_from_input(input, chroma_storage);
            let v_stride = stride_from_input(input, chroma_storage);
            let u =
                plane_from_input::<T>(input, bit_depth, chroma_storage, u_stride, chroma_visible)?;
            let v =
                plane_from_input::<T>(input, bit_depth, chroma_storage, v_stride, chroma_visible)?;
            FramePlanes::new(y, Some(u), Some(v))
        }
    };

    let info = DecodedFrameInfo::new(
        OutputIndex::new(output_index),
        bit_depth,
        pixel_format,
        luma_storage,
        luma_visible,
    )
    .ok()?;
    DecodedFrame::try_new(info, planes).ok()
}

fn storage_size_from_input(
    input: &mut FuzzInput<'_>,
    visible_width: usize,
    visible_height: usize,
) -> Option<PlaneSize> {
    let pad_x = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
    let pad_y = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
    PlaneSize::new(visible_width + pad_x, visible_height + pad_y).ok()
}

fn stride_from_input(input: &mut FuzzInput<'_>, storage: PlaneSize) -> usize {
    storage.width() + usize::from(input.byte()) % (MAX_STRIDE_PADDING + 1)
}

fn plane_from_input<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
    storage: PlaneSize,
    stride_samples: usize,
    visible_rect: PlaneRect,
) -> Option<Plane<T>> {
    let sample_count = stride_samples.checked_mul(storage.height())?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(sample_count).ok()?;
    for _ in 0..sample_count {
        samples.push(sample_from_input::<T>(input, bit_depth)?);
    }
    Plane::from_vec(storage, stride_samples, visible_rect, samples).ok()
}

fn sample_from_input<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) -> Option<T> {
    let value = match bit_depth {
        BitDepth::Eight => u16::from(input.byte()),
        BitDepth::Ten => {
            let high = u16::from(input.byte());
            let low = u16::from(input.byte() & 0b0000_0011);
            ((high << 2) | low) & bit_depth.max_sample()
        }
    };
    T::try_from_u16(value).ok()
}

fn frame_rate_from_input(input: &mut FuzzInput<'_>) -> Option<Y4mFrameRate> {
    let numerator = 1 + u32::from(input.byte());
    let denominator = 1 + u32::from(input.byte());
    Y4mFrameRate::new(numerator, denominator).ok()
}

const fn alternate_pixel_format(pixel_format: PixelFormat) -> PixelFormat {
    match pixel_format {
        PixelFormat::Monochrome => PixelFormat::Yuv420,
        PixelFormat::Yuv420 => PixelFormat::Monochrome,
        PixelFormat::Yuv422 => PixelFormat::Yuv444,
        PixelFormat::Yuv444 => PixelFormat::Yuv422,
    }
}

struct FuzzInput<'a> {
    bytes: std::slice::Iter<'a, u8>,
}

impl<'a> FuzzInput<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes: bytes.iter(),
        }
    }

    fn byte(&mut self) -> u8 {
        self.bytes.next().copied().unwrap_or(0)
    }
}
