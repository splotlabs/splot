// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Y4M output adapter.
//!
//! Feature tracking: `DECODE-Y4M-RUNTIME-OUTPUT`.

use std::io::Write;

use splot_core::ivf::IvfHeader;
use splot_recon::{BitDepth, DecodedFrame, ReconSample, Y4mError, Y4mFrameRate, Y4mWriter};

use crate::DecodeOptions;
use crate::bitstream::byte_stream::UnitSource;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain::{GrainPlanes, write_display};
use crate::pipeline::PipelineDecodedFrame;

pub(crate) fn write_y4m_stream_to_writer(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
    output: &mut (dyn Write + Send),
) -> Result<()> {
    let mut target = Some(output);
    let mut writer = None;
    let mut sample_bit_depth = None;
    let mut seen_xlayer = None;
    let frame_rate_override = options.y4m_frame_rate_override();
    let mut spare = GrainPlanes::default();
    let backend = session.backend();
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |header| preflight_y4m_source(header, frame_rate_override),
        |output| {
            super::track_flat_output_xlayer(&mut seen_xlayer, output.extended_layer_id)?;
            let rate = output.frame_rate;
            let frame_rate = || {
                frame_rate_override
                    .map_or_else(|| Y4mFrameRate::new(rate.numerator, rate.denominator), Ok)
            };
            let grain = output.output_grain(options);
            match &output.ready_frame()? {
                PipelineDecodedFrame::Eight(frame) => {
                    ensure_y4m_sample_bit_depth(&mut sample_bit_depth, BitDepth::Eight)?;
                    write_display(backend, frame.get(), grain, &mut spare.eight, |display| {
                        write_y4m_frame(&mut writer, &mut target, display, frame_rate)
                    })
                }
                PipelineDecodedFrame::Ten(frame) => {
                    ensure_y4m_sample_bit_depth(&mut sample_bit_depth, BitDepth::Ten)?;
                    write_display(backend, frame.get(), grain, &mut spare.ten, |display| {
                        write_y4m_frame(&mut writer, &mut target, display, frame_rate)
                    })
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
    Ok(())
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

fn write_y4m_frame<'w, T: ReconSample>(
    writer: &mut Option<Y4mWriter<&'w mut (dyn Write + Send)>>,
    output: &mut Option<&'w mut (dyn Write + Send)>,
    frame: &DecodedFrame<T>,
    frame_rate: impl FnOnce() -> std::result::Result<Y4mFrameRate, Y4mError>,
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
        let frame_rate = frame_rate().map_err(map_y4m_writer_error)?;
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

fn preflight_y4m_source(
    header: Option<IvfHeader>,
    frame_rate_override: Option<Y4mFrameRate>,
) -> Result<()> {
    if frame_rate_override.is_some() {
        return Ok(());
    }
    let header = header.ok_or_else(|| {
        DecodeOutputError::invalid_frame_set(
            DecodeOutputOperation::SerializeY4m,
            "Y4M output for raw Annex B requires a caller-provided frame rate",
        )
    })?;
    preflight_y4m_header(header)
}

#[cfg(test)]
#[path = "y4m_tests.rs"]
mod tests;
