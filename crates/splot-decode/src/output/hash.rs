// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier hash adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, PixelFormat, ReconSample};

use crate::error::Result;
use crate::hash_report::{
    DecodeHashEntry, DecodeHashFrame, DecodeHashPixelFormat, DecodeHashReport,
};
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn decode_hash_report_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    resolved_threads: NonZeroUsize,
) -> Result<DecodeHashReport> {
    let outputs = crate::pipeline::decode_frames_from_plan(bytes, options, plan)?;
    let mut report_frames = Vec::with_capacity(outputs.len());
    for output in &outputs {
        let report_frame = match &output.frame {
            PipelineDecodedFrame::Eight(frame) => {
                let hash = DecodedFrameHashInput::new(frame).compute_hash();
                hash_frame_from_decoded(frame, hash.to_hex())
            }
            PipelineDecodedFrame::Ten(frame) => {
                let hash = DecodedFrameHashInput::new(frame).compute_hash();
                hash_frame_from_decoded(frame, hash.to_hex())
            }
        };
        report_frames.push(report_frame);
    }

    Ok(DecodeHashReport::raw_intermediate_output(
        resolved_threads.to_string(),
        report_frames,
    ))
}

fn hash_frame_from_decoded<T: ReconSample>(
    frame: &DecodedFrame<T>,
    digest_hex: String,
) -> DecodeHashFrame {
    let visible = frame.visible_luma_rect();
    let chroma = frame
        .pixel_format()
        .chroma_size(visible.size())
        .ok()
        .flatten();
    DecodeHashFrame {
        output_index: frame.output_index().get(),
        visible_luma_left: visible.x() as u32,
        visible_luma_top: visible.y() as u32,
        visible_luma_width: visible.width() as u32,
        visible_luma_height: visible.height() as u32,
        chroma_left: chroma
            .map(|_| (visible.x() >> usize::from(frame.pixel_format().subsampling_x())) as u32),
        chroma_top: chroma
            .map(|_| (visible.y() >> usize::from(frame.pixel_format().subsampling_y())) as u32),
        chroma_width: chroma.map(|size| size.width() as u32),
        chroma_height: chroma.map(|size| size.height() as u32),
        bit_depth: frame.bit_depth().bits(),
        pixel_format: decode_hash_pixel_format(frame.pixel_format()),
        hashes: vec![DecodeHashEntry::raw_intermediate_output_sha256(digest_hex)],
    }
}

fn decode_hash_pixel_format(pixel_format: PixelFormat) -> DecodeHashPixelFormat {
    match pixel_format {
        PixelFormat::Monochrome => DecodeHashPixelFormat::Monochrome,
        PixelFormat::Yuv420 => DecodeHashPixelFormat::Yuv420,
        PixelFormat::Yuv422 => DecodeHashPixelFormat::Yuv422,
        PixelFormat::Yuv444 => DecodeHashPixelFormat::Yuv444,
    }
}

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;
