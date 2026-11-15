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
use crate::output::film_grain::{GrainPlanes, write_display};
use crate::pipeline::PipelineDecodedFrame;

pub(crate) fn write_raw_stream_from_plan<W: Write + Send>(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
    mut writer: W,
) -> Result<()> {
    let mut spare = GrainPlanes::default();
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            let frame = output.ready_frame()?;
            write_display_frame(&frame, output.display_grain.as_ref(), &mut spare, &mut writer)
        },
    )?;
    Ok(())
}

/// Writes one displayed frame, turning a panicking writer into an error at
/// every pool width.
fn write_display_frame(
    frame: &PipelineDecodedFrame,
    display_grain: Option<&crate::pipeline::ActiveFilmGrain>,
    spare: &mut GrainPlanes,
    writer: &mut impl Write,
) -> Result<()> {
    catch_unwind(AssertUnwindSafe(|| match frame {
        PipelineDecodedFrame::Eight(frame) => {
            write_display(frame.get(), display_grain, &mut spare.eight, |display| {
                write_raw_frame(display, writer)
            })
        }
        PipelineDecodedFrame::Ten(frame) => {
            write_display(frame.get(), display_grain, &mut spare.ten, |display| {
                write_raw_frame(display, writer)
            })
        }
    }))
    .unwrap_or_else(|_| Err(raw_output_task_error("raw output task panicked")))
}

pub(crate) fn discard_raw_stream_from_plan(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
) -> Result<()> {
    let mut spare = GrainPlanes::default();
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            let grain = output.display_grain.as_ref();
            match &output.ready_frame()? {
                PipelineDecodedFrame::Eight(frame) => {
                    write_display(frame.get(), grain, &mut spare.eight, |_| Ok(()))
                }
                PipelineDecodedFrame::Ten(frame) => {
                    write_display(frame.get(), grain, &mut spare.ten, |_| Ok(()))
                }
            }
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
