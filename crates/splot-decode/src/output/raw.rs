// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Raw sample-byte output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use core::num::NonZeroUsize;

use std::io::Write;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::bitstream::byte_stream::FlatParsedBitstream;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn write_raw_stream_from_plan<W: Write + Send>(
    bitstream: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    mut writer: W,
) -> Result<()> {
    let decode_started = crate::timing::start();
    crate::pipeline::emit_materialized_frames_from_prepared(
        bitstream,
        parsed,
        options,
        plan,
        frame_delay,
        |_| Ok(()),
        |output| {
            let serialize_started = crate::timing::start();
            let result = match &output.ready_frame()? {
                PipelineDecodedFrame::Eight(frame) => {
                    let display =
                        film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                    write_raw_frame(display.as_ref(), &mut writer)
                }
                PipelineDecodedFrame::Ten(frame) => {
                    let display =
                        film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                    write_raw_frame(display.as_ref(), &mut writer)
                }
            };
            crate::timing::report("raw_serialize", serialize_started);
            result
        },
    )?;
    crate::timing::report("runtime_decode", decode_started);
    Ok(())
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
