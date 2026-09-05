// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Raw sample-byte output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use core::num::NonZeroUsize;

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use splot_parallel::CompletionCell;
use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::bitstream::byte_stream::FlatParsedBitstream;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::output::film_grain;
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};
use parking_lot::Mutex;

pub(crate) fn write_raw_stream_from_plan<W: Write + Send>(
    bitstream: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
    writer: W,
) -> Result<()> {
    let writer = Mutex::new(writer);
    let output_error = Mutex::new(None);
    let decode_result = splot_parallel::ready_task_scope(|scope| {
        let mut outstanding: Option<Arc<CompletionCell<()>>> = None;
        let decode_result = crate::pipeline::emit_materialized_frames_from_prepared(
            bitstream,
            parsed,
            options,
            plan,
            frame_delay,
            |_| Ok(()),
            |output| {
                if let Some(done) = outstanding.take() {
                    let () = done.wait_with_pool_assist();
                    if let Some(error) = output_error.lock().take() {
                        return Err(error);
                    }
                }
                let frame = output.ready_frame()?;
                let display_grain = output.display_grain.clone();
                let writer = &writer;
                let output_error = &output_error;
                let done = Arc::new(CompletionCell::new());
                outstanding = Some(Arc::clone(&done));
                scope.spawn(move |_| {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let mut writer = writer.lock();
                        match &frame {
                            PipelineDecodedFrame::Eight(frame) => {
                                let display = film_grain::frame_for_output(
                                    frame.get(),
                                    display_grain.as_ref(),
                                )?;
                                write_raw_frame(display.as_ref(), &mut *writer)
                            }
                            PipelineDecodedFrame::Ten(frame) => {
                                let display = film_grain::frame_for_output(
                                    frame.get(),
                                    display_grain.as_ref(),
                                )?;
                                write_raw_frame(display.as_ref(), &mut *writer)
                            }
                        }
                    }))
                    .unwrap_or_else(|_| Err(raw_output_task_error("raw output task panicked")));
                    if let Err(error) = result {
                        let mut failure = output_error.lock();
                        if failure.is_none() {
                            *failure = Some(error);
                        }
                    }
                    let _ = done.set(());
                });
                Ok(())
            },
        );
        if let Some(done) = outstanding {
            let () = done.wait_with_pool_assist();
        }
        decode_result
    })?;
    if let Some(error) = output_error.into_inner() {
        return Err(error);
    }
    decode_result?;
    Ok(())
}

pub(crate) fn discard_raw_stream_from_plan(
    bitstream: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
) -> Result<()> {
    crate::pipeline::emit_materialized_frames_from_prepared(
        bitstream,
        parsed,
        options,
        plan,
        frame_delay,
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
