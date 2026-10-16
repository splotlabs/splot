// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Raw sample-byte output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::DecodeOptions;
use crate::bitstream::byte_stream::UnitSource;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use parking_lot::Mutex;

pub(crate) fn write_raw_stream_from_plan<W: Write + Send>(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
    writer: W,
) -> Result<()> {
    let writer = Mutex::new(writer);
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            let frame = output.ready_frame()?;
            write_display_frame(&frame, output.display_grain.as_ref(), &writer)
        },
    )?;
    Ok(())
}

/// Writes one displayed frame, turning a panicking writer into an error at
/// every pool width.
fn write_display_frame(
    frame: &PipelineDecodedFrame,
    display_grain: Option<&crate::pipeline::ActiveFilmGrain>,
    writer: &Mutex<impl Write>,
) -> Result<()> {
    catch_unwind(AssertUnwindSafe(|| {
        let writer = &mut *writer.lock();
        match frame {
            PipelineDecodedFrame::Eight(frame) => {
                let display = film_grain::frame_for_output(frame.get(), display_grain)?;
                write_raw_frame(display.as_ref(), writer)
            }
            PipelineDecodedFrame::Ten(frame) => {
                let display = film_grain::frame_for_output(frame.get(), display_grain)?;
                write_raw_frame(display.as_ref(), writer)
            }
        }
    }))
    .unwrap_or_else(|_| Err(raw_output_task_error("raw output task panicked")))
}

pub(crate) fn discard_raw_stream_from_plan(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
) -> Result<()> {
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            let frame = output.ready_frame()?;
            match &frame {
                PipelineDecodedFrame::Eight(frame) => {
                    film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                }
                PipelineDecodedFrame::Ten(frame) => {
                    film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn raw_output_task_error(message: &'static str) -> crate::error::DecodeError {
    DecodeOutputError::io(
        DecodeOutputOperation::WriteRawStream,
        std::io::Error::other(message),
    )
    .into()
}

fn write_raw_frame<T: ReconSample>(frame: &DecodedFrame<T>, writer: &mut impl Write) -> Result<()> {
    let raw = DecodedFrameHashInput::new(frame);
    raw.write_to(writer)
        .map_err(|source| DecodeOutputError::io(DecodeOutputOperation::WriteRawStream, source))?;
    Ok(())
}

#[cfg(test)]
#[path = "raw_tests.rs"]
mod tests;
