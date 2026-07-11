// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Y4M output adapter.
//!
//! Feature tracking: `DECODE-Y4M-RUNTIME-OUTPUT`.

use splot_core::ivf::IvfHeader;
use splot_recon::{
    BitDepth, DecodedFrame, PixelFormat, PlaneSize, ReconSample, Y4mFrameFormat, Y4mFrameHeader,
    Y4mFrameRate, Y4mStreamHeader, Y4mWriter,
};

use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use crate::support::pipeline_limits::{checked_add, checked_mul};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions, DecodeStreamPlan};

const MINIMAL_Y4M_LUMA_WIDTH: usize = 64;
const MINIMAL_Y4M_LUMA_HEIGHT: usize = 64;

pub(crate) fn encode_y4m_stream_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<u8>> {
    let limits = options.limits();
    let outputs = crate::pipeline::decode_frames_from_plan_with_ivf_preflight(
        bytes,
        options,
        plan,
        |header| {
            let Some(header) = header else {
                return Err(crate::pipeline::unsupported(
                    "annex_b_y4m_timebase",
                    None,
                    "Y4M output requires IVF timebase metadata",
                ));
            };
            preflight_y4m_minimal_header(header, limits)
        },
    )?;
    let first = outputs.first().ok_or_else(|| {
        DecodeOutputError::invalid_frame_set(
            DecodeOutputOperation::SerializeY4m,
            "runtime Y4M output requires at least one decoded frame",
        )
    })?;
    let frame_rate = Y4mFrameRate::new(first.frame_rate_numerator, first.frame_rate_denominator)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;

    let mut y4m = Vec::new();
    match &first.frame {
        PipelineDecodedFrame::Eight(first_frame) => {
            write_y4m_stream(
                &mut y4m,
                first_frame,
                frame_rate,
                &outputs,
                |output| match &output.frame {
                    PipelineDecodedFrame::Eight(frame) => Some(frame),
                    PipelineDecodedFrame::Ten(_) => None,
                },
            )?;
        }
        PipelineDecodedFrame::Ten(first_frame) => {
            write_y4m_stream(
                &mut y4m,
                first_frame,
                frame_rate,
                &outputs,
                |output| match &output.frame {
                    PipelineDecodedFrame::Ten(frame) => Some(frame),
                    PipelineDecodedFrame::Eight(_) => None,
                },
            )?;
        }
    }

    options
        .limits()
        .ensure(DecodeLimitName::MaxOutputBytes, y4m.len() as u64)?;
    Ok(y4m)
}

fn write_y4m_stream<T: ReconSample>(
    y4m: &mut Vec<u8>,
    first_frame: &DecodedFrame<T>,
    frame_rate: Y4mFrameRate,
    outputs: &[crate::pipeline::PipelineFrame],
    select: impl Fn(&crate::pipeline::PipelineFrame) -> Option<&DecodedFrame<T>>,
) -> Result<()> {
    let mut writer = Y4mWriter::from_frame(y4m, first_frame, frame_rate)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    for output in outputs {
        let frame = select(output).ok_or_else(|| {
            DecodeOutputError::invalid_frame_set(
                DecodeOutputOperation::SerializeY4m,
                "runtime Y4M output requires every displayed frame to share the first frame's sample bit depth",
            )
        })?;
        let display = film_grain::frame_for_output(frame, output.display_grain.as_ref())?;
        writer.write_frame(display.as_ref()).map_err(|source| {
            DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source)
        })?;
    }
    writer
        .flush()
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    Ok(())
}

fn preflight_y4m_minimal_header(header: IvfHeader, limits: DecodeLimits) -> Result<()> {
    let frame_rate = Y4mFrameRate::new(header.timebase_denominator, header.timebase_numerator)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    ensure_minimal_y4m_output_limit(limits, frame_rate)
}

fn ensure_minimal_y4m_output_limit(limits: DecodeLimits, frame_rate: Y4mFrameRate) -> Result<()> {
    let luma_size = PlaneSize::new(MINIMAL_Y4M_LUMA_WIDTH, MINIMAL_Y4M_LUMA_HEIGHT)?;
    let frame_format = Y4mFrameFormat::new(luma_size, BitDepth::Eight, PixelFormat::Monochrome)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    let stream_header = Y4mStreamHeader::new(frame_format, frame_rate);
    let mut stream_header_bytes = Vec::new();
    stream_header
        .write_to(&mut stream_header_bytes)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;

    let luma_bytes = checked_mul(
        DecodeLimitName::MaxOutputBytes,
        MINIMAL_Y4M_LUMA_WIDTH as u64,
        MINIMAL_Y4M_LUMA_HEIGHT as u64,
    )?;
    let headers_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        stream_header_bytes.len() as u64,
        Y4mFrameHeader::new().as_bytes().len() as u64,
    )?;
    let total_bytes = checked_add(DecodeLimitName::MaxOutputBytes, headers_bytes, luma_bytes)?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, total_bytes)?;

    Ok(())
}

#[cfg(test)]
#[path = "y4m_tests.rs"]
mod tests;
