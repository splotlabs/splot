// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Y4M output adapter.
//!
//! Feature tracking: `DECODE-Y4M-RUNTIME-OUTPUT`.

use core::num::NonZeroUsize;
use std::io::Write;

use splot_core::ivf::IvfHeader;
use splot_recon::{BitDepth, DecodedFrame, ReconSample, Y4mError, Y4mFrameRate, Y4mWriter};

use crate::bitstream::byte_stream::FlatParsedBitstream;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn write_y4m_stream_to_writer<W: Write + Send>(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    output: W,
) -> Result<W> {
    let mut target = Some(output);
    let mut writer = None;
    let mut sample_bit_depth = None;
    crate::pipeline::emit_materialized_frames_from_prepared(
        bytes,
        parsed,
        options,
        plan,
        frame_delay,
        |header| {
            let Some(header) = header else {
                return Err(crate::pipeline::unsupported(
                    "annex_b_y4m_timebase",
                    None,
                    "Y4M output requires IVF timebase metadata",
                ));
            };
            preflight_y4m_header(header)
        },
        |output| {
            let frame_rate =
                Y4mFrameRate::new(output.frame_rate_numerator, output.frame_rate_denominator)
                    .map_err(|source| {
                        DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source)
                    })?;
            match &output.ready_frame()? {
                PipelineDecodedFrame::Eight(frame) => {
                    ensure_y4m_sample_bit_depth(&mut sample_bit_depth, BitDepth::Eight)?;
                    let display =
                        film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                    write_y4m_frame(&mut writer, &mut target, display.as_ref(), frame_rate)
                }
                PipelineDecodedFrame::Ten(frame) => {
                    ensure_y4m_sample_bit_depth(&mut sample_bit_depth, BitDepth::Ten)?;
                    let display =
                        film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                    write_y4m_frame(&mut writer, &mut target, display.as_ref(), frame_rate)
                }
            }
        },
    )?;
    let mut writer = writer.ok_or_else(|| {
        DecodeOutputError::invalid_frame_set(
            DecodeOutputOperation::SerializeY4m,
            "runtime Y4M output requires at least one decoded frame",
        )
    })?;
    writer.flush().map_err(map_y4m_writer_error)?;
    Ok(writer.into_inner())
}

fn ensure_y4m_sample_bit_depth(expected: &mut Option<BitDepth>, actual: BitDepth) -> Result<()> {
    match *expected {
        Some(expected) if expected != actual => Err(DecodeOutputError::invalid_frame_set(
            DecodeOutputOperation::SerializeY4m,
            "runtime Y4M output requires every displayed frame to share the first frame's sample bit depth",
        )
        .into()),
        Some(_) => Ok(()),
        None => {
            *expected = Some(actual);
            Ok(())
        }
    }
}

fn write_y4m_frame<T: ReconSample, W: Write>(
    writer: &mut Option<Y4mWriter<W>>,
    output: &mut Option<W>,
    frame: &DecodedFrame<T>,
    frame_rate: Y4mFrameRate,
) -> Result<()> {
    let mut active = if let Some(active) = writer.take() {
        active
    } else {
        let output = output.take().ok_or_else(|| {
            DecodeOutputError::invalid_frame_set(
                DecodeOutputOperation::SerializeY4m,
                "runtime Y4M output writer is unavailable",
            )
        })?;
        Y4mWriter::from_frame(output, frame, frame_rate).map_err(map_y4m_writer_error)?
    };
    active.write_frame(frame).map_err(map_y4m_writer_error)?;
    *writer = Some(active);
    Ok(())
}

fn map_y4m_writer_error(source: Y4mError) -> DecodeOutputError {
    match source {
        Y4mError::Io { source } => {
            DecodeOutputError::io(DecodeOutputOperation::WriteY4mStream, source)
        }
        source => DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source),
    }
}

fn preflight_y4m_header(header: IvfHeader) -> Result<()> {
    Y4mFrameRate::new(header.timebase_denominator, header.timebase_numerator)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    Ok(())
}

#[cfg(test)]
#[path = "y4m_tests.rs"]
mod tests;
