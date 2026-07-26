// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Hash report output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, PixelFormat, ReconSample};
use std::sync::{Mutex, OnceLock};

use crate::bitstream::byte_stream::FlatParsedBitstream;
use crate::error::Result;
use crate::hash_report::{
    DecodeHashEntry, DecodeHashFrame, DecodeHashPixelFormat, DecodeHashReport,
};
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn decode_hash_report_from_plan(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    resolved_threads: NonZeroUsize,
    frame_delay: NonZeroUsize,
) -> Result<DecodeHashReport> {
    if discard_hash() {
        crate::pipeline::emit_frames_from_prepared(
            bytes,
            parsed,
            options,
            plan,
            frame_delay,
            |_| Ok(()),
        )?;
        return Ok(DecodeHashReport::raw_intermediate_output(
            resolved_threads.to_string(),
            Vec::new(),
        ));
    }
    let report_frames = if splot_parallel::on_multiworker_pool() {
        decode_hash_frames_pipelined(
            bytes,
            parsed,
            options,
            plan,
            hasher_frame_delay(frame_delay, resolved_threads),
        )?
    } else {
        let mut frames = Vec::new();
        crate::pipeline::emit_frames_from_prepared(
            bytes,
            parsed,
            options,
            plan,
            frame_delay,
            |output| {
                let emitted = frames.len() as u64;
                frames.push(hash_pipeline_frame(&output.ready_frame()?, emitted));
                Ok(())
            },
        )?;
        frames
    };

    Ok(DecodeHashReport::raw_intermediate_output(
        resolved_threads.to_string(),
        report_frames,
    ))
}

/// The pipelining depth the decode driver keeps once the hasher takes a worker.
///
/// The hasher task parks on the queue for the whole decode, so the driver plus
/// the `depth - 1` filter phases it keeps in flight must fit the remaining
/// workers, or the driver could park on a filter phase no worker is left to run.
/// Only a depth that would overrun those workers is clamped; a depth the pool
/// can still afford is kept, so `--threads 4 --frame-delay 2` stays pipelined.
fn hasher_frame_delay(frame_delay: NonZeroUsize, resolved_threads: NonZeroUsize) -> NonZeroUsize {
    NonZeroUsize::new(resolved_threads.get() - 1)
        .map_or(NonZeroUsize::MIN, |available| frame_delay.min(available))
}

/// Hashes decoded frames on a dedicated worker task while the driver decodes.
fn decode_hash_frames_pipelined(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
) -> Result<Vec<DecodeHashFrame>> {
    let completed = Mutex::new(Vec::new());
    let capacity = splot_parallel::QueueCapacity::new(NonZeroUsize::MIN.saturating_add(1));
    let (sender, receiver) = splot_parallel::bounded_queue::<(u64, PipelineDecodedFrame)>(capacity);
    splot_parallel::ready_task_scope(|scope| {
        let completed = &completed;
        scope.spawn(move |_| {
            while let Ok((emitted, frame)) = receiver.recv() {
                let hashed = hash_pipeline_frame(&frame, emitted);
                completed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(hashed);
            }
        });

        let mut emitted = 0u64;
        let decoded = crate::pipeline::emit_frames_from_prepared(
            bytes,
            parsed,
            options,
            plan,
            frame_delay,
            |output| {
                let ready = output.ready_frame()?;
                let index = emitted;
                emitted += 1;
                if let Err(disconnected) = sender.send((index, ready)) {
                    let (index, frame) = disconnected.0;
                    let hashed = hash_pipeline_frame(&frame, index);
                    completed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(hashed);
                }
                Ok(())
            },
        );
        drop(sender);
        decoded
    })??;

    Ok(completed
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner))
}

fn discard_hash() -> bool {
    static DISCARD_HASH: OnceLock<bool> = OnceLock::new();
    *DISCARD_HASH.get_or_init(|| std::env::var_os("SPLOT_DECODE_DISCARD_HASH").is_some())
}

/// Hashes one emitted frame, recording `emitted` as its report row index.
///
/// `emitted` is the frame's 0-based emission ordinal, counted where the driver
/// hands the frame to the report; frame pipelining may finish frames out of
/// order, but it never reorders emission.
fn hash_pipeline_frame(frame: &PipelineDecodedFrame, emitted: u64) -> DecodeHashFrame {
    let timer = crate::timing::start();
    let output = match frame {
        PipelineDecodedFrame::Eight(frame) => {
            let hash = DecodedFrameHashInput::new(frame.get()).compute_hash();
            hash_frame_from_decoded(frame.get(), hash.to_hex(), emitted)
        }
        PipelineDecodedFrame::Ten(frame) => {
            let hash = DecodedFrameHashInput::new(frame.get()).compute_hash();
            hash_frame_from_decoded(frame.get(), hash.to_hex(), emitted)
        }
    };
    crate::timing::report("output_hash", timer);
    output
}

fn hash_frame_from_decoded<T: ReconSample>(
    frame: &DecodedFrame<T>,
    digest_hex: String,
    emitted: u64,
) -> DecodeHashFrame {
    let visible = frame.visible_luma_rect();
    let chroma = frame
        .pixel_format()
        .chroma_size(visible.size())
        .ok()
        .flatten();
    DecodeHashFrame {
        output_index: emitted,
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
