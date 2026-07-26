// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Raw sample-byte output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::bitstream::byte_stream::FlatParsedBitstream;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn encode_raw_stream_from_plan(
    bitstream: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
) -> Result<Vec<u8>> {
    let decode_started = crate::timing::start();
    let outputs = crate::pipeline::decode_frames_from_prepared(
        bitstream,
        parsed,
        options,
        plan,
        frame_delay,
    )?;
    crate::timing::report("runtime_decode", decode_started);
    let serialize_started = crate::timing::start();
    let mut total_bytes = 0usize;
    for output in &outputs {
        total_bytes = total_bytes.saturating_add(output.byte_len()?);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(total_bytes).map_err(|source| {
        DecodeOutputError::io(
            DecodeOutputOperation::SerializeRaw,
            std::io::Error::other(format!("raw output allocation failed: {source}")),
        )
    })?;
    for output in &outputs {
        match &output.ready_frame()? {
            PipelineDecodedFrame::Eight(frame) => {
                let display =
                    film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                write_raw_frame(display.as_ref(), &mut bytes)?;
            }
            PipelineDecodedFrame::Ten(frame) => {
                let display =
                    film_grain::frame_for_output(frame.get(), output.display_grain.as_ref())?;
                write_raw_frame(display.as_ref(), &mut bytes)?;
            }
        }
    }
    crate::timing::report("raw_serialize", serialize_started);
    options
        .limits()
        .ensure(crate::DecodeLimitName::MaxOutputBytes, bytes.len() as u64)?;
    Ok(bytes)
}

fn write_raw_frame<T: ReconSample>(frame: &DecodedFrame<T>, bytes: &mut Vec<u8>) -> Result<()> {
    let raw = DecodedFrameHashInput::new(frame);
    bytes.try_reserve_exact(raw.byte_len()?).map_err(|source| {
        DecodeOutputError::io(
            DecodeOutputOperation::SerializeRaw,
            std::io::Error::other(format!("raw output allocation failed: {source}")),
        )
    })?;
    raw.write_to(bytes)
        .map_err(|source| DecodeOutputError::io(DecodeOutputOperation::SerializeRaw, source))?;
    Ok(())
}

#[cfg(test)]
#[path = "raw_tests.rs"]
mod tests;
