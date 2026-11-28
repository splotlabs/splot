// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Raw sample-byte output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use std::io::Write;

use splot_core::simd::SimdBackend;
use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::DecodeOptions;
use crate::bitstream::byte_stream::UnitSource;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain::{GrainPlanes, write_display};
use crate::pipeline::PipelineDecodedFrame;

pub(crate) fn write_raw_stream_from_plan(
    backend: impl SimdBackend,
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
    writer: &mut (dyn Write + Send),
) -> Result<()> {
    let mut spare = GrainPlanes::default();
    let mut seen_xlayer = None;
    crate::pipeline::emit_frames(
        backend,
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            super::track_flat_output_xlayer(&mut seen_xlayer, output.extended_layer_id)?;
            let frame = output.ready_frame()?;
            write_display_frame(
                backend,
                &frame,
                output.output_grain(options),
                &mut spare,
                writer,
            )
        },
    )?;
    Ok(())
}

/// Writes one displayed frame.
fn write_display_frame(
    backend: impl SimdBackend,
    frame: &PipelineDecodedFrame,
    display_grain: Option<&crate::pipeline::ActiveFilmGrain>,
    spare: &mut GrainPlanes,
    writer: &mut (dyn Write + Send),
) -> Result<()> {
    match frame {
        PipelineDecodedFrame::Eight(frame) => write_display(
            backend,
            frame.get(),
            display_grain,
            &mut spare.eight,
            |display| write_raw_frame(display, writer),
        ),
        PipelineDecodedFrame::Ten(frame) => write_display(
            backend,
            frame.get(),
            display_grain,
            &mut spare.ten,
            |display| write_raw_frame(display, writer),
        ),
    }
}

pub(crate) fn discard_raw_stream_from_plan(
    backend: impl SimdBackend,
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
) -> Result<()> {
    let mut spare = GrainPlanes::default();
    crate::pipeline::emit_frames(
        backend,
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            let grain = output.output_grain(options);
            match &output.ready_frame()? {
                PipelineDecodedFrame::Eight(frame) => {
                    write_display(backend, frame.get(), grain, &mut spare.eight, |_| Ok(()))
                }
                PipelineDecodedFrame::Ten(frame) => {
                    write_display(backend, frame.get(), grain, &mut spare.ten, |_| Ok(()))
                }
            }
        },
    )?;
    Ok(())
}

fn write_raw_frame<T: ReconSample>(
    frame: &DecodedFrame<T>,
    writer: &mut (dyn Write + Send),
) -> Result<()> {
    let raw = DecodedFrameHashInput::new(frame);
    raw.write_to(writer)
        .map_err(|source| DecodeOutputError::io(DecodeOutputOperation::WriteRawStream, source))?;
    Ok(())
}

#[cfg(test)]
#[path = "raw_tests.rs"]
mod tests;
