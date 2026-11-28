// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Hash report output adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::DecodeOptions;
use crate::bitstream::byte_stream::UnitSource;
use crate::error::Result;
use crate::hash_report::{DecodeHashEntry, DecodeHashFrame, DecodeHashReport};
use crate::pipeline::PipelineDecodedFrame;

pub(crate) fn decode_hash_report_from_plan(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    resolved_threads: NonZeroUsize,
    session: &crate::pipeline::DecodeSession,
) -> Result<DecodeHashReport> {
    let report_frames = decode_hash_frames(source, options, session)?;

    Ok(DecodeHashReport::raw_intermediate_output(
        resolved_threads.to_string(),
        report_frames,
    ))
}

/// Hashes each decoded frame as the driver emits it.
fn decode_hash_frames(
    source: &mut dyn UnitSource,
    options: &DecodeOptions,
    session: &crate::pipeline::DecodeSession,
) -> Result<Vec<DecodeHashFrame>> {
    let mut completed = Vec::new();
    let mut emitted = 0u64;
    crate::pipeline::emit_frames(
        source,
        options,
        session,
        |_| Ok(()),
        |output| {
            completed.push(hash_pipeline_frame(output, emitted)?);
            emitted += 1;
            Ok(())
        },
    )?;
    Ok(completed)
}

/// Hashes one emitted frame, recording `emitted` as its report row index.
///
/// `emitted` is the frame's 0-based emission ordinal, counted where the driver
/// hands the frame to the report; frame pipelining may finish frames out of
/// order, but it never reorders emission.
fn hash_pipeline_frame(
    output: &crate::pipeline::PipelineFrame,
    emitted: u64,
) -> Result<DecodeHashFrame> {
    Ok(match output.ready_frame()? {
        PipelineDecodedFrame::Eight(frame) => {
            let hash = DecodedFrameHashInput::new(frame.get()).compute_hash();
            hash_frame_from_decoded(frame.get(), hash.to_hex(), emitted, output)
        }
        PipelineDecodedFrame::Ten(frame) => {
            let hash = DecodedFrameHashInput::new(frame.get()).compute_hash();
            hash_frame_from_decoded(frame.get(), hash.to_hex(), emitted, output)
        }
    })
}

fn hash_frame_from_decoded<T: ReconSample>(
    frame: &DecodedFrame<T>,
    digest_hex: String,
    emitted: u64,
    output: &crate::pipeline::PipelineFrame,
) -> DecodeHashFrame {
    let visible = frame.visible_luma_rect();
    let chroma = frame
        .pixel_format()
        .chroma_size(visible.size())
        .ok()
        .flatten();
    DecodeHashFrame {
        output_index: emitted,
        extended_layer_id: output.extended_layer_id,
        embedded_layer_id: output.embedded_layer_id,
        atlas_associations: output.output_effects.atlas.clone(),
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
        pixel_format: frame.pixel_format(),
        hashes: vec![DecodeHashEntry::raw_intermediate_output_sha256(digest_hex)],
    }
}

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;
