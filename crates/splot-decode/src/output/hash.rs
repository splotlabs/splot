// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Hash report output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_parallel::CompletionCell;
use splot_recon::{DecodedFrame, DecodedFrameHashInput, PixelFormat, ReconSample};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

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
        decode_hash_frames_pipelined(bytes, parsed, options, plan, frame_delay)?
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

/// How many emitted frames may have an unfinished hash task at once.
///
/// A hash task holds a second handle on its frame's sample storage, and the
/// driver retires a decoded frame only once its storage has a single handle, so
/// every unfinished hash task keeps one more decoded frame charged against
/// [`crate::DecodeLimitName::MaxReferenceStoreBytes`]. Unbounded, that term is
/// set by how far the pool trails the driver rather than by the decoder, which
/// makes a documented memory limit fail or hold by scheduling luck. Four keeps
/// the handoff off the driver's critical path at 2, 4, 8, and 10 workers while
/// bounding the extra live frames by a constant.
const MAX_OUTSTANDING_HASH_FRAMES: usize = 4;

/// Hashes decoded frames on short worker tasks while the driver decodes.
fn decode_hash_frames_pipelined(
    bytes: &[u8],
    parsed: &FlatParsedBitstream<'_>,
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    frame_delay: NonZeroUsize,
) -> Result<Vec<DecodeHashFrame>> {
    let completed = Mutex::new(Vec::new());
    splot_parallel::ready_task_scope(|scope| {
        let mut emitted = 0u64;
        let mut outstanding: VecDeque<Arc<CompletionCell<()>>> = VecDeque::new();
        crate::pipeline::emit_frames_from_prepared(
            bytes,
            parsed,
            options,
            plan,
            frame_delay,
            |output| {
                while outstanding.len() >= MAX_OUTSTANDING_HASH_FRAMES
                    && let Some(oldest) = outstanding.pop_front()
                {
                    let () = oldest.wait_with_pool_assist();
                }
                let ready = output.ready_frame()?;
                let index = emitted;
                emitted += 1;
                let completed = &completed;
                let hashed_done = Arc::new(CompletionCell::new());
                outstanding.push_back(Arc::clone(&hashed_done));
                scope.spawn(move |_| {
                    let hashed = hash_pipeline_frame(&ready, index);
                    completed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(hashed);
                    let _ = hashed_done.set(());
                });
                Ok(())
            },
        )
    })??;

    let mut completed = completed
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    completed.sort_unstable_by_key(|frame| frame.output_index);
    Ok(completed)
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
